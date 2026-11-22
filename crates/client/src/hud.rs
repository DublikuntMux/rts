//! RTS console: tactical map, selected units, and a spatial command card.

mod layout;
mod minimap;
mod phone;
pub use layout::{HudLayout, PhoneHud, PhonePanel, hud_inset, phone_layout};
#[cfg(test)]
mod tests;

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    text::{FontWeight, LineHeight},
    ui::{InteractionDisabled, Pressed, UiSystems},
    ui_widgets::{Activate, Button, observe},
};
use rts_assets::Graphics;
use rts_base::{Unit, UnitId};

use crate::{
    font::GameFont,
    game::{AppState, Match},
    input::Selection,
    locale::Localization,
    menu::widgets::{BAR, EDGE, FIELD, GOOD, INK, INK_MUTED, RAIL, SIGNAL, Tone, button_look},
    pause_menu::PauseMenu,
    server_assets::{ServerAssets, load_image},
    skirmish,
};

/// Maximum height, in logical pixels before interface scaling.
pub const HUD_HEIGHT: f32 = 184.0;
/// The command waiting for a point on the battlefield.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Targeting {
    #[default]
    None,
    Move,
    Barracks,
}

impl Targeting {
    pub fn prompt(self, loc: &Localization) -> &str {
        match self {
            Self::None => "",
            Self::Move => loc.t("hud.target_move"),
            Self::Barracks => loc.t("hud.target_barracks"),
        }
    }
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Targeting>()
            .init_resource::<PhoneHud>()
            .init_resource::<Localization>()
            .add_systems(OnEnter(AppState::InGame), spawn_hud)
            .add_systems(OnExit(AppState::InGame), (reset_targeting, phone::reset))
            .add_systems(
                Update,
                (
                    phone::resize_hud,
                    phone::update_surfaces,
                    phone::update_controls,
                    update_selection,
                    fit_unit_images,
                    update_button_availability,
                    update_button_looks,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                PostUpdate,
                (minimap::update_minimap, minimap::update_minimap_fog)
                    .before(UiSystems::Layout)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn reset_targeting(mut targeting: ResMut<Targeting>) {
    *targeting = Targeting::None;
}

#[derive(Component)]
struct Console;
#[derive(Component)]
struct SelectionPanel;
#[derive(Component)]
struct Portrait;
#[derive(Component)]
struct PortraitImage;
/// A unit image inside a square frame, resized to keep the image's aspect ratio.
#[derive(Component)]
struct UnitImage;
#[derive(Component)]
struct Roster;
#[derive(Component)]
struct RosterUnit(UnitId);
#[derive(Component)]
struct HudText(TextRole);
#[derive(Clone, Copy)]
enum TextRole {
    Title,
    Detail,
    Hint,
}
#[derive(Component)]
struct HealthFill;
#[derive(Component)]
struct CommandCard;
#[derive(Component)]
struct CommandSlot(usize);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Order {
    Move,
    Stop,
    Clear,
}
#[derive(Component)]
struct ButtonInk;

fn spawn_hud(
    mut commands: Commands,
    font: Res<GameFont>,
    assets: Res<AssetServer>,
    server_assets: Option<Res<ServerAssets>>,
    game: Res<Match>,
    loc: Res<Localization>,
    mut images: ResMut<Assets<Image>>,
) {
    let map = minimap::spawn(&mut commands, &game, &mut images);
    let root = commands
        .spawn((
            Name::new("RTS console"),
            Console,
            DespawnOnExit(AppState::InGame),
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                bottom: px(0),
                height: px(HUD_HEIGHT),
                border: UiRect::top(px(1)),
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            },
            Pickable::IGNORE,
            BackgroundColor(BAR),
            BorderColor::all(EDGE),
        ))
        .id();
    commands.entity(root).add_child(map).with_children(|bar| {
        spawn_selection_panel(bar, &font.0, &loc);
        spawn_command_card(bar, &font.0, &assets, server_assets.as_deref(), &game, &loc);
        phone::spawn_overlays(bar, &font.0, &loc);
    });
}

fn spawn_selection_panel(bar: &mut ChildSpawnerCommands, font: &Handle<Font>, loc: &Localization) {
    bar.spawn((
        SelectionPanel,
        GlobalZIndex(4),
        bevy::ui_widgets::ScrollArea,
        observe(crate::menu::scroll_on_touch_drag),
        observe(crate::menu::scroll_focus_into_view),
        Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            flex_basis: px(0),
            min_width: px(0),
            padding: UiRect::axes(px(16), px(10)),
            border: UiRect::horizontal(px(1)),
            row_gap: px(7),
            overflow: Overflow::clip(),
            ..default()
        },
        BorderColor::all(EDGE),
        BackgroundColor(Color::NONE),
    ))
    .with_children(|panel| {
        panel.spawn((
            HudText(TextRole::Title),
            Node {
                flex_shrink: 0.0,
                ..default()
            },
            Text::new(loc.t("hud.no_units")),
            text_font(font, 20.0),
            TextColor(INK),
        ));
        panel
            .spawn(Node {
                column_gap: px(12),
                min_height: px(48),
                flex_shrink: 0.0,
                ..default()
            })
            .with_children(|row| {
                spawn_unit_details(row, font);
            });
        panel.spawn((
            Roster,
            Node {
                column_gap: px(4),
                height: px(48),
                flex_shrink: 0.0,
                overflow: Overflow::clip(),
                ..default()
            },
        ));
        panel.spawn((
            HudText(TextRole::Hint),
            phone::DesktopOnly,
            Text::new(loc.t("hud.hint_click_select")),
            text_font(font, 12.0),
            TextColor(INK_MUTED),
            LineHeight::RelativeToFont(1.2),
        ));
    });
}

fn spawn_unit_details(row: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    row.spawn((
        Portrait,
        Node {
            width: px(64),
            height: px(64),
            padding: UiRect::all(px(5)),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(FIELD),
        BorderColor::all(EDGE),
        children![(PortraitImage, unit_image(Handle::default(), Color::WHITE))],
    ));
    row.spawn(Node {
        flex_direction: FlexDirection::Column,
        justify_content: JustifyContent::Center,
        flex_grow: 1.0,
        min_width: px(0),
        row_gap: px(6),
        ..default()
    })
    .with_children(|details| {
        details.spawn((
            HudText(TextRole::Detail),
            Text::new(""),
            text_font(font, 13.0),
            TextColor(INK_MUTED),
        ));
        details
            .spawn((
                Node {
                    height: px(5),
                    max_width: px(200),
                    width: percent(100),
                    ..default()
                },
                BackgroundColor(FIELD),
            ))
            .with_children(|track| {
                track.spawn((
                    HealthFill,
                    Node {
                        height: percent(100),
                        width: px(0),
                        ..default()
                    },
                    BackgroundColor(GOOD),
                ));
            });
    });
}

fn spawn_command_card(
    bar: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    assets: &AssetServer,
    server_assets: Option<&ServerAssets>,
    game: &Match,
    loc: &Localization,
) {
    bar.spawn((
        CommandCard,
        Node {
            width: px(184),
            height: percent(100),
            flex_shrink: 0.0,
            padding: UiRect::all(px(12)),
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::px(3, 48.0),
            grid_template_rows: RepeatedGridTrack::px(3, 48.0),
            column_gap: px(8),
            row_gap: px(8),
            ..default()
        },
        BackgroundColor(RAIL),
    ))
    .with_children(|card| {
        for index in 0..9 {
            let order = match index {
                0 => Some(Order::Move),
                1 => Some(Order::Stop),
                8 => Some(Order::Clear),
                _ => None,
            };
            if let Some(order) = order {
                spawn_order(card, index, order, font, assets, server_assets, loc);
            } else {
                card.spawn((
                    CommandSlot(index),
                    Node {
                        width: px(48),
                        height: px(48),
                        border: UiRect::all(px(1)),
                        ..default()
                    },
                    BackgroundColor(FIELD.with_alpha(0.5)),
                    BorderColor::all(EDGE.with_alpha(0.45)),
                ))
                .with_children(|slot| skirmish::spawn_desktop_action(slot, index, font, game, loc));
            }
        }
        phone::spawn_selection_button(card, font);
    });
}

fn spawn_order(
    card: &mut ChildSpawnerCommands,
    index: usize,
    order: Order,
    font: &Handle<Font>,
    assets: &AssetServer,
    server_assets: Option<&ServerAssets>,
    loc: &Localization,
) {
    card.spawn((
        CommandSlot(index),
        order,
        Button,
        Hovered::default(),
        Node {
            width: px(48),
            height: px(48),
            border: UiRect::all(px(1)),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(2),
            ..default()
        },
        BackgroundColor(FIELD),
        BorderColor::all(EDGE),
        observe(activate_order),
    ))
    .with_children(|button| {
        spawn_order_contents(button, order, font, assets, server_assets, loc);
    });
}

fn spawn_order_contents(
    button: &mut ChildSpawnerCommands,
    order: Order,
    font: &Handle<Font>,
    assets: &AssetServer,
    server_assets: Option<&ServerAssets>,
    loc: &Localization,
) {
    let (label_key, key, icon) = match order {
        Order::Move => ("hud.order_move", "M", "textures/ui/commands/move.png"),
        Order::Stop => ("hud.order_stop", "X", "textures/ui/commands/stop.png"),
        Order::Clear => ("hud.order_clear", "", "textures/ui/commands/clear.png"),
    };
    if !key.is_empty() {
        button.spawn((
            phone::DesktopOnly,
            Pickable::IGNORE,
            Text::new(key),
            text_font(font, 9.0),
            TextColor(INK_MUTED),
            Node {
                position_type: PositionType::Absolute,
                left: px(3),
                top: px(1),
                ..default()
            },
        ));
    }
    button.spawn((
        ButtonInk,
        ImageNode::new(load_image(server_assets, assets, icon)),
        Node {
            width: px(20),
            height: px(20),
            ..default()
        },
        Pickable::IGNORE,
    ));
    button.spawn((
        ButtonInk,
        Pickable::IGNORE,
        Text::new(loc.t(label_key)),
        text_font(font, 11.0),
        TextColor(INK),
    ));
}

pub fn text_font(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(size),
        weight: if size >= 20.0 {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        },
        ..default()
    }
}

fn unit_image(image: Handle<Image>, color: Color) -> impl Bundle {
    (
        UnitImage,
        Pickable::IGNORE,
        Node {
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ImageNode {
            image,
            color,
            ..default()
        },
    )
}

/// The longer side of a loaded unit image fills its centered frame.
fn fit_unit_images(
    textures: Res<Assets<Image>>,
    mut images: Query<(&ImageNode, &mut Node), With<UnitImage>>,
) {
    for (image, mut node) in &mut images {
        let Some(size) = textures.get(&image.image).map(Image::size_f32) else {
            continue;
        };
        if size.min_element() <= 0.0 {
            continue;
        }
        let fit = size / size.max_element() * 100.0;
        if node.width != percent(fit.x) || node.height != percent(fit.y) {
            node.width = percent(fit.x);
            node.height = percent(fit.y);
        }
    }
}

type PortraitFilter = (With<Portrait>, Without<Roster>);
type HealthFilter = (With<HealthFill>, Without<Portrait>, Without<Roster>);
type RosterFilter = (With<Roster>, Without<Portrait>, Without<HealthFill>);

fn activate_order(
    event: On<Activate>,
    buttons: Query<(&Order, Has<InteractionDisabled>)>,
    mut selection: ResMut<Selection>,
    mut game: ResMut<Match>,
    mut targeting: ResMut<Targeting>,
) {
    let Ok((&order, disabled)) = buttons.get(event.entity) else {
        return;
    };
    if disabled {
        return;
    }
    *targeting = if order == Order::Move && *targeting != Targeting::Move {
        Targeting::Move
    } else {
        Targeting::None
    };
    match order {
        Order::Move => {}
        Order::Stop => selection.stop(&mut game),
        Order::Clear => selection.units.clear(),
    }
}

fn update_button_availability(
    mut commands: Commands,
    selection: Res<Selection>,
    game: Res<Match>,
    pause: Res<PauseMenu>,
    phone: Res<PhoneHud>,
    buttons: Query<(Entity, &Order, Has<InteractionDisabled>)>,
) {
    let disabled =
        selection.units.is_empty() || !game.accepts_orders || pause.open || phone.panel.is_some();
    let mobile = selection.mobile_units(&game).next().is_some();
    for (entity, order, was_disabled) in &buttons {
        let disabled = disabled || (*order != Order::Clear && !mobile);
        if disabled != was_disabled {
            if disabled {
                commands.entity(entity).insert(InteractionDisabled);
            } else {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
        }
    }
}

type OrderState<'a> = (
    &'a Order,
    &'a Hovered,
    Has<Pressed>,
    Has<InteractionDisabled>,
    &'a Children,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
);
type Ink<'a> = (Option<&'a mut TextColor>, Option<&'a mut ImageNode>);

fn update_button_looks(
    targeting: Res<Targeting>,
    mut buttons: Query<OrderState>,
    mut inks: Query<Ink, (With<ButtonInk>, Without<Order>)>,
) {
    for (order, hovered, pressed, disabled, children, mut fill, mut border) in &mut buttons {
        let selected = *order == Order::Move && *targeting == Targeting::Move;
        let look = button_look(Tone::Item, selected, hovered.get(), pressed, disabled);
        fill.set_if_neq(BackgroundColor(look.fill));
        border.set_if_neq(BorderColor::all(if selected { SIGNAL } else { look.edge }));
        for child in children {
            if let Ok((text, image)) = inks.get_mut(*child) {
                if let Some(mut text) = text {
                    text.set_if_neq(TextColor(look.ink));
                }
                if let Some(mut image) = image
                    && image.color != look.ink
                {
                    image.color = look.ink;
                }
            }
        }
    }
}

fn update_selection(
    mut commands: Commands,
    selection: Res<Selection>,
    game: Res<Match>,
    font: Res<GameFont>,
    graphics: Res<Graphics>,
    assets: Res<AssetServer>,
    server_assets: Option<Res<ServerAssets>>,
    targeting: Res<Targeting>,
    loc: Res<Localization>,
    mut labels: Query<(&HudText, &mut Text)>,
    mut portrait: Query<&mut Node, PortraitFilter>,
    mut portrait_image: Query<&mut ImageNode, With<PortraitImage>>,
    mut health: Query<&mut Node, HealthFilter>,
    mut roster: Single<(Entity, &mut Node), RosterFilter>,
    mut last_roster: Local<Vec<UnitId>>,
) {
    let units: Vec<_> = selection
        .units
        .iter()
        .filter_map(|id| game.view.unit(*id).map(|unit| (*id, unit)))
        .collect();
    let title = match units.as_slice() {
        [] => loc.t("hud.no_units").to_owned(),
        [(_, unit)] => loc.unit_name(&game.view.rules().unit(unit.kind).id),
        units => {
            let count = units.len().to_string();
            loc.fmt("hud.units_selected", &[("count", &count)])
        }
    };
    let total_health: i64 = units.iter().map(|(_, unit)| unit.health()).sum();
    let max_health: i64 = units
        .iter()
        .map(|(_, unit)| {
            let definition = game.view.rules().unit(unit.kind);
            i64::from(definition.max_health) * i64::from(definition.squad)
        })
        .sum();
    let detail = selection_detail(&game, &units, total_health, max_health, &loc);
    for (role, mut text) in &mut labels {
        let value = match role.0 {
            TextRole::Title => title.clone(),
            TextRole::Detail => detail.clone(),
            TextRole::Hint => selection_hint(&game, &selection, *targeting, &loc),
        };
        text.set_if_neq(Text::new(value));
    }
    for mut node in &mut portrait {
        node.display = if units.len() == 1 {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut image in &mut portrait_image {
        if let Some((_, unit)) = units.first() {
            image.image = load_image(
                server_assets.as_deref(),
                &assets,
                &game.view.rules().unit(unit.kind).sprite,
            );
            image.color = graphics.player_color(unit.owner.0);
        }
    }
    for mut node in &mut health {
        node.width = percent(health_percent(total_health, max_health));
    }
    let ids: Vec<_> = units.iter().map(|(id, _)| *id).collect();
    roster.1.display = if units.len() > 1 {
        Display::Flex
    } else {
        Display::None
    };
    if *last_roster == ids {
        return;
    }
    last_roster.clone_from(&ids);
    commands.entity(roster.0).despawn_children();
    if units.len() <= 1 {
        return;
    }
    spawn_roster(
        &mut commands,
        roster.0,
        &units,
        &font.0,
        &game,
        &graphics,
        &assets,
        server_assets.as_deref(),
    );
}

fn spawn_roster(
    commands: &mut Commands,
    entity: Entity,
    units: &[(UnitId, &rts_base::Unit)],
    font: &Handle<Font>,
    game: &Match,
    graphics: &Graphics,
    assets: &AssetServer,
    server_assets: Option<&ServerAssets>,
) {
    commands.entity(entity).with_children(|row| {
        for (id, unit) in units.iter().take(12) {
            row.spawn((
                RosterUnit(*id),
                Button,
                Hovered::default(),
                Node {
                    width: px(48),
                    height: px(48),
                    flex_shrink: 0.0,
                    padding: UiRect::all(px(5)),
                    border: UiRect::all(px(1)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(FIELD),
                BorderColor::all(EDGE),
                observe(select_portrait),
                children![unit_image(
                    load_image(
                        server_assets,
                        assets,
                        &game.view.rules().unit(unit.kind).sprite,
                    ),
                    graphics.player_color(unit.owner.0),
                )],
            ));
        }
        if units.len() > 12 {
            row.spawn((
                Text::new(format!("+{}", units.len() - 12)),
                text_font(font, 15.0),
                TextColor(INK_MUTED),
            ));
        }
    });
}

#[expect(
    clippy::cast_precision_loss,
    reason = "health ratios are for presentation only"
)]
fn health_percent(health: i64, max: i64) -> f32 {
    if max > 0 {
        (health as f32 / max as f32 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

fn select_portrait(
    event: On<Activate>,
    portraits: Query<&RosterUnit>,
    mut selection: ResMut<Selection>,
    pause: Res<PauseMenu>,
) {
    if pause.open {
        return;
    }
    if let Ok(unit) = portraits.get(event.entity) {
        selection.units.clear();
        selection.units.insert(unit.0);
    }
}

fn selection_detail(
    game: &Match,
    units: &[(UnitId, &Unit)],
    total_health: i64,
    max_health: i64,
    loc: &Localization,
) -> String {
    if units.is_empty() {
        return loc.t("hud.await_orders").to_owned();
    }
    let soldiers: usize = units.iter().map(|(_, unit)| unit.alive()).sum();
    let squad: usize = units.iter().map(|(_, unit)| unit.soldiers.len()).sum();
    let veterancy = match units {
        [(_, unit)] => match game.view.rules().rank(unit.kills) {
            0 if unit.kills == 0 => String::new(),
            0 => {
                let kills = unit.kills.to_string();
                loc.fmt("hud.kills", &[("kills", &kills)])
            }
            rank => {
                let kills = unit.kills.to_string();
                let rank_name = &game.view.rules().ranks()[rank - 1].name;
                loc.fmt("hud.rank_kills", &[("rank", rank_name), ("kills", &kills)])
            }
        },
        _ => String::new(),
    };
    let state = selection_state(game, units, loc);
    let hp = total_health.to_string();
    let max_hp = max_health.to_string();
    if squad > units.len() {
        let soldiers_str = soldiers.to_string();
        let squad_str = squad.to_string();
        loc.fmt(
            "hud.squad_detail",
            &[
                ("soldiers", &soldiers_str),
                ("squad", &squad_str),
                ("hp", &hp),
                ("max_hp", &max_hp),
                ("state", state),
                ("veterancy", &veterancy),
            ],
        )
    } else {
        loc.fmt(
            "hud.unit_detail",
            &[
                ("hp", &hp),
                ("max_hp", &max_hp),
                ("state", state),
                ("veterancy", &veterancy),
            ],
        )
    }
}

fn selection_state<'a>(game: &Match, units: &[(UnitId, &Unit)], loc: &'a Localization) -> &'a str {
    if let [(id, _)] = units
        && let Some(state) = skirmish::economic_state(game, *id, loc)
    {
        return state;
    }
    if units
        .iter()
        .any(|(id, _)| game.view.move_target(*id).is_some())
    {
        loc.t("hud.state_moving")
    } else if units
        .iter()
        .any(|(id, _)| skirmish::economic_state(game, *id, loc).is_some())
    {
        loc.t("hud.state_working")
    } else {
        loc.t("hud.state_idle")
    }
}

fn selection_hint(
    game: &Match,
    selection: &Selection,
    targeting: Targeting,
    loc: &Localization,
) -> String {
    if targeting != Targeting::None {
        return targeting.prompt(loc).to_owned();
    }
    if let Some(hint) = skirmish::economy_hint(game, selection, targeting, loc) {
        return hint;
    }
    let key = match (selection.touch_mode, selection.units.is_empty()) {
        (false, true) => "hud.hint_click_select",
        (true, true) => "hud.hint_tap_select",
        (_, false) if selection.mobile_units(game).next().is_none() => return String::new(),
        (false, false) => "hud.hint_desktop_move",
        (true, false) => "hud.hint_touch_move",
    };
    loc.t(key).to_owned()
}
