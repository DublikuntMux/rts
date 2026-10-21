//! RTS console: tactical map, selected units, and a spatial command card.

mod minimap;
#[cfg(test)]
mod tests;

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    text::{FontWeight, LineHeight},
    ui::{InteractionDisabled, Pressed, UiSystems},
    ui_widgets::{Activate, Button, observe},
    window::PrimaryWindow,
};
use rts_assets::Graphics;
use rts_base::UnitId;

use crate::{
    font::GameFont,
    game::{AppState, Match},
    input::Selection,
    menu::widgets::{BAR, EDGE, FIELD, GOOD, INK, INK_MUTED, RAIL, SIGNAL, Tone, button_look},
    pause_menu::PauseMenu,
};

/// Maximum height, in logical pixels before interface scaling.
pub const HUD_HEIGHT: f32 = 184.0;
const COMPACT_WIDTH: f32 = 620.0;

pub fn hud_height(width: f32) -> f32 {
    if width < COMPACT_WIDTH {
        152.0
    } else {
        HUD_HEIGHT
    }
}

/// The Move command waits for a world destination without changing the selection.
#[derive(Resource, Default)]
pub struct MoveTargeting(pub bool);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MoveTargeting>()
            .add_systems(OnEnter(AppState::InGame), spawn_hud)
            .add_systems(OnExit(AppState::InGame), reset_targeting)
            .add_systems(
                Update,
                (
                    resize_hud,
                    update_selection,
                    update_button_availability,
                    update_button_looks,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                PostUpdate,
                minimap::update_minimap
                    .before(UiSystems::Layout)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn reset_targeting(mut targeting: ResMut<MoveTargeting>) {
    targeting.0 = false;
}

#[derive(Component)]
struct Console;
#[derive(Component)]
struct SelectionPanel;
#[derive(Component)]
struct Portrait;
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
    Compact,
}
#[derive(Component)]
struct HealthFill;
#[derive(Component)]
struct CommandCard;
#[derive(Component)]
struct CompactSummary;
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
    game: Res<Match>,
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
            BackgroundColor(BAR),
            BorderColor::all(EDGE),
        ))
        .id();
    commands.entity(root).add_child(map).with_children(|bar| {
        spawn_selection_panel(bar, &font.0);
        spawn_command_card(bar, &font.0, &assets);
    });
}

fn spawn_selection_panel(bar: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    bar.spawn((
        SelectionPanel,
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
    ))
    .with_children(|panel| {
        panel.spawn((
            HudText(TextRole::Title),
            Text::new("No units selected"),
            text_font(font, 20.0),
            TextColor(INK),
        ));
        panel
            .spawn(Node {
                column_gap: px(12),
                min_height: px(48),
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
            Text::new("Click or drag to select units."),
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
            ..default()
        },
        ImageNode::default(),
        BackgroundColor(FIELD),
        BorderColor::all(EDGE),
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

fn spawn_command_card(bar: &mut ChildSpawnerCommands, font: &Handle<Font>, assets: &AssetServer) {
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
                spawn_order(card, index, order, font, assets);
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
                ));
            }
        }
        card.spawn((
            CompactSummary,
            HudText(TextRole::Compact),
            Text::new("No selection"),
            text_font(font, 11.0),
            TextColor(INK_MUTED),
            Node {
                position_type: PositionType::Absolute,
                left: px(12),
                bottom: px(6),
                display: Display::None,
                ..default()
            },
        ));
    });
}

fn spawn_order(
    card: &mut ChildSpawnerCommands,
    index: usize,
    order: Order,
    font: &Handle<Font>,
    assets: &AssetServer,
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
        spawn_order_contents(button, order, font, assets);
    });
}

fn spawn_order_contents(
    button: &mut ChildSpawnerCommands,
    order: Order,
    font: &Handle<Font>,
    assets: &AssetServer,
) {
    let (label, key, icon) = match order {
        Order::Move => ("Move", "M", "textures/ui/commands/move.png"),
        Order::Stop => ("Stop", "X", "textures/ui/commands/stop.png"),
        Order::Clear => ("Clear", "", "textures/ui/commands/clear.png"),
    };
    if !key.is_empty() {
        button.spawn((
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
        ImageNode::new(assets.load(icon)),
        Node {
            width: px(20),
            height: px(20),
            ..default()
        },
        Pickable::IGNORE,
    ));
    button.spawn((
        ButtonInk,
        Text::new(label),
        text_font(font, 11.0),
        TextColor(INK),
    ));
}

fn text_font(font: &Handle<Font>, size: f32) -> TextFont {
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

type ConsoleNode<'a> = (
    &'a mut Node,
    Has<Console>,
    Has<SelectionPanel>,
    Has<CommandCard>,
    Option<&'a CommandSlot>,
    Has<CompactSummary>,
);
type PortraitFilter = (With<Portrait>, Without<Roster>);
type HealthFilter = (With<HealthFill>, Without<Portrait>, Without<Roster>);
type RosterFilter = (With<Roster>, Without<Portrait>, Without<HealthFill>);

fn resize_hud(
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut nodes: Query<ConsoleNode>,
) {
    let width = window.width() / scale.0;
    let compact = width < COMPACT_WIDTH;
    for (mut node, root, selection, card, slot, summary) in &mut nodes {
        if summary {
            node.display = if width < 560.0 {
                Display::Flex
            } else {
                Display::None
            };
        }
        if root {
            node.height = px(hud_height(width));
        }
        if selection {
            node.display = if width < 560.0 {
                Display::None
            } else {
                Display::Flex
            };
        }
        if card {
            node.width = px(if compact { 176 } else { 184 });
            node.column_gap = px(if compact { 4 } else { 8 });
            node.grid_template_rows = RepeatedGridTrack::px(if compact { 2 } else { 3 }, 48.0);
        }
        if let Some(slot) = slot {
            node.display = if compact && (5..8).contains(&slot.0) {
                Display::None
            } else {
                Display::Flex
            };
        }
    }
}

fn activate_order(
    event: On<Activate>,
    buttons: Query<(&Order, Has<InteractionDisabled>)>,
    mut selection: ResMut<Selection>,
    mut game: ResMut<Match>,
    mut targeting: ResMut<MoveTargeting>,
) {
    let Ok((&order, disabled)) = buttons.get(event.entity) else {
        return;
    };
    if disabled {
        return;
    }
    targeting.0 = order == Order::Move && !targeting.0;
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
    buttons: Query<(Entity, Has<InteractionDisabled>), With<Order>>,
) {
    let disabled = selection.units.is_empty() || !game.accepts_orders || pause.open;
    for (entity, was_disabled) in &buttons {
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
    targeting: Res<MoveTargeting>,
    mut buttons: Query<OrderState>,
    mut inks: Query<Ink, (With<ButtonInk>, Without<Order>)>,
) {
    for (order, hovered, pressed, disabled, children, mut fill, mut border) in &mut buttons {
        let selected = *order == Order::Move && targeting.0;
        let look = button_look(Tone::Secondary, selected, hovered.get(), pressed, disabled);
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
    targeting: Res<MoveTargeting>,
    mut labels: Query<(&HudText, &mut Text)>,
    mut portrait: Query<(&mut ImageNode, &mut Node), PortraitFilter>,
    mut health: Query<&mut Node, HealthFilter>,
    mut roster: Single<(Entity, &mut Node), RosterFilter>,
    mut last_roster: Local<Vec<UnitId>>,
) {
    let units: Vec<_> = selection
        .units
        .iter()
        .filter_map(|id| game.sim.unit(*id).map(|unit| (*id, unit)))
        .collect();
    let title = match units.as_slice() {
        [] => "No units selected".into(),
        [(_, unit)] => unit_name(&game.sim.rules().unit(unit.kind).id),
        units => format!("{} units selected", units.len()),
    };
    let total_health: i64 = units.iter().map(|(_, unit)| i64::from(unit.health)).sum();
    let max_health: i64 = units
        .iter()
        .map(|(_, unit)| i64::from(game.sim.rules().unit(unit.kind).max_health))
        .sum();
    let moving = units
        .iter()
        .filter(|(id, _)| game.sim.move_target(*id).is_some())
        .count();
    let detail = if units.is_empty() {
        "Your forces await orders".into()
    } else {
        format!(
            "{total_health} / {max_health} HP  ·  {}",
            if moving > 0 { "Moving" } else { "Idle" }
        )
    };
    for (role, mut text) in &mut labels {
        let value = match role.0 {
            TextRole::Title => title.clone(),
            TextRole::Detail => detail.clone(),
            TextRole::Hint => hint(&selection, targeting.0).into(),
            TextRole::Compact => {
                if units.is_empty() {
                    "No selection".into()
                } else {
                    format!("{} selected", units.len())
                }
            }
        };
        text.set_if_neq(Text::new(value));
    }
    for (mut image, mut node) in &mut portrait {
        node.display = if units.len() == 1 {
            Display::Flex
        } else {
            Display::None
        };
        if let Some((_, unit)) = units.first() {
            image.image = assets.load(game.sim.rules().unit(unit.kind).sprite.clone());
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
                    ..default()
                },
                ImageNode {
                    image: assets.load(game.sim.rules().unit(unit.kind).sprite.clone()),
                    color: graphics.player_color(unit.owner.0),
                    ..default()
                },
                BackgroundColor(FIELD),
                BorderColor::all(EDGE),
                observe(select_portrait),
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

fn unit_name(id: &str) -> String {
    let mut chars = id.chars();
    chars.next().map_or_else(String::new, |first| {
        first
            .to_uppercase()
            .chain(chars)
            .collect::<String>()
            .replace('_', " ")
    })
}

fn hint(selection: &Selection, targeting: bool) -> &'static str {
    if targeting {
        "Choose a destination."
    } else {
        match (selection.touch_mode, selection.units.is_empty()) {
            (false, true) => "Click or drag to select units.",
            (false, false) => "Right-click to move · X to stop",
            (true, true) => "Tap or drag to select units.",
            (true, false) => "Tap ground to move · Two fingers pan",
        }
    }
}
