//! Player-facing economy controls and battlefield resource markers.

use bevy::{
    picking::hover::Hovered,
    prelude::*,
    ui::{InteractionDisabled, Pressed},
    ui_widgets::{Activate, Button, observe},
    window::PrimaryWindow,
};
use rts_assets::SelectionStyle;
use rts_base::{
    Command, DepositId, IVec2 as SimPos, PRODUCTION_QUEUE_LIMIT, ResourceKind, Resources, Training,
    TrainingKind, UnitId,
};

use crate::{
    font::GameFont,
    game::{AppState, Match},
    hud::{HudLayout, PhoneHud, Targeting, text_font},
    input::Selection,
    locale::Localization,
    map::{to_world, to_world_distance},
    menu::{
        Menu,
        widgets::{BAR, EDGE, INK, INK_MUTED, SIGNAL, Tone, button_look},
    },
    pause_menu::{PauseMenu, world_input_blocked},
};

const DEPOSIT_SIZE: f32 = 72.0;
const ACTION_WIDTH: f32 = 94.0;
const ACTION_HEIGHT: f32 = 50.0;
const ACTION_GAP: f32 = 6.0;

pub struct SkirmishPlugin;

impl Plugin for SkirmishPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Localization>()
            .add_systems(OnEnter(AppState::InGame), spawn_skirmish)
            .add_systems(
                Update,
                (update_skirmish, update_deposits).run_if(in_state(AppState::InGame)),
            );
    }
}

#[derive(Component)]
struct DepositSprite(DepositId);

#[derive(Component)]
struct ResourceLabel(ResourceKind);

#[derive(Component)]
struct ResourceBar;

#[derive(Component)]
struct EconomyCard;

#[derive(Component)]
struct EconomyHint;

#[derive(Component)]
struct ActionCost;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum EconomyAction {
    Barracks,
    Worker,
    RifleSquad,
    Scout,
}

impl EconomyAction {
    const ALL: [Self; 4] = [Self::Barracks, Self::Worker, Self::RifleSquad, Self::Scout];

    const fn label(self) -> &'static str {
        match self {
            Self::Barracks => "Build barracks",
            Self::Worker => "Train worker",
            Self::RifleSquad => "Rifle squad",
            Self::Scout => "Train scout",
        }
    }

    /// The price, or `None` when the rules do not offer this action.
    fn cost(self, game: &Match) -> Option<Resources> {
        let rules = game.view.rules().economy()?;
        match self {
            Self::Barracks => Some(rules.barracks_cost),
            Self::Worker => Some(rules.worker_cost),
            Self::RifleSquad => Some(rules.rifle_cost),
            Self::Scout => rules.scout.map(|scout| scout.cost),
        }
    }

    fn units<'a>(
        self,
        game: &'a Match,
        selection: &'a Selection,
    ) -> impl Iterator<Item = UnitId> + 'a {
        selected_kind(
            game,
            selection,
            match self {
                Self::Barracks => "worker",
                Self::Worker => "headquarters",
                Self::RifleSquad | Self::Scout => "barracks",
            },
        )
        .filter(move |_| self.cost(game).is_some())
    }

    fn ready_units<'a>(
        self,
        game: &'a Match,
        selection: &'a Selection,
    ) -> impl Iterator<Item = UnitId> + 'a {
        self.units(game, selection).filter(move |&id| {
            self == Self::Barracks
                || game
                    .view
                    .production(id)
                    .is_some_and(|queue| queue.len() < PRODUCTION_QUEUE_LIMIT)
        })
    }

    fn affordable(self, game: &Match) -> bool {
        self.cost(game)
            .zip(game.view.resources())
            .is_some_and(|(cost, funds)| funds.can_afford(cost))
    }
}

fn resource_label(kind: ResourceKind, loc: &Localization) -> &str {
    match kind {
        ResourceKind::Wood => loc.t("economy.wood"),
        ResourceKind::Gold => loc.t("economy.gold"),
    }
}

const fn resource_icon(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Wood => "textures/terrain/wood.png",
        ResourceKind::Gold => "textures/terrain/gold.png",
    }
}

fn spawn_skirmish(
    mut commands: Commands,
    game: Res<Match>,
    font: Res<GameFont>,
    assets: Res<AssetServer>,
    loc: Res<Localization>,
) {
    if !game.view.is_skirmish() {
        return;
    }
    spawn_balances(&mut commands, &font.0, &assets);
    spawn_economy_card(&mut commands, &font.0, &game, &loc);
}

fn spawn_balances(commands: &mut Commands, font: &Handle<Font>, assets: &AssetServer) {
    commands
        .spawn((
            Name::new("Resource balances"),
            ResourceBar,
            DespawnOnExit(AppState::InGame),
            Pickable::IGNORE,
            BackgroundColor(BAR),
            BorderColor::all(EDGE),
            GlobalZIndex(2),
            Node {
                position_type: PositionType::Absolute,
                left: px(68),
                top: px(12),
                padding: UiRect::axes(px(8), px(4)),
                column_gap: px(12),
                border: UiRect::all(px(1)),
                ..default()
            },
        ))
        .with_children(|bar| {
            for kind in [ResourceKind::Wood, ResourceKind::Gold] {
                bar.spawn((
                    Node {
                        column_gap: px(3),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|item| {
                    item.spawn((
                        ImageNode::new(assets.load(resource_icon(kind))),
                        Node {
                            width: px(18),
                            height: px(18),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    item.spawn((
                        ResourceLabel(kind),
                        Text::new(""),
                        text_font(font, 12.0),
                        TextColor(INK),
                        Pickable::IGNORE,
                    ));
                });
            }
        });
}

fn spawn_economy_card(
    commands: &mut Commands,
    font: &Handle<Font>,
    game: &Match,
    loc: &Localization,
) {
    commands
        .spawn((
            Name::new("Economy controls"),
            EconomyCard,
            DespawnOnExit(AppState::InGame),
            Pickable::IGNORE,
            GlobalZIndex(2),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                left: px(8),
                flex_direction: FlexDirection::Column,
                row_gap: px(ACTION_GAP),
                align_items: AlignItems::Start,
                ..default()
            },
        ))
        .with_children(|card| {
            card.spawn((
                EconomyHint,
                Text::new(""),
                text_font(font, 12.0),
                TextColor(INK),
                BackgroundColor(BAR),
                Node {
                    padding: UiRect::axes(px(7), px(4)),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            card.spawn((
                Node {
                    column_gap: px(ACTION_GAP),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|row| {
                for action in EconomyAction::ALL {
                    spawn_economy_button(row, action, ActionLayout::Phone, font, game, loc);
                }
            });
        });
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ActionLayout {
    Desktop,
    Phone,
}

pub fn spawn_desktop_action(
    slot: &mut ChildSpawnerCommands,
    index: usize,
    font: &Handle<Font>,
    game: &Match,
    loc: &Localization,
) {
    if game.view.is_skirmish()
        && let Some(&action) = index
            .checked_sub(3)
            .and_then(|offset| EconomyAction::ALL.get(offset))
    {
        spawn_economy_button(slot, action, ActionLayout::Desktop, font, game, loc);
    }
}

fn spawn_economy_button(
    row: &mut ChildSpawnerCommands,
    action: EconomyAction,
    layout: ActionLayout,
    font: &Handle<Font>,
    game: &Match,
    loc: &Localization,
) {
    let desktop = layout == ActionLayout::Desktop;
    row.spawn((
        Name::new(action.label()),
        action,
        layout,
        Button,
        Hovered::default(),
        Node {
            display: Display::None,
            width: px(if desktop { 46.0 } else { ACTION_WIDTH }),
            height: px(if desktop { 46.0 } else { ACTION_HEIGHT }),
            border: UiRect::all(px(1)),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: px(1),
            ..default()
        },
        BackgroundColor(BAR),
        BorderColor::all(EDGE),
        observe(activate_economy),
    ))
    .with_children(|button| {
        let label_key = if desktop {
            match action {
                EconomyAction::Barracks => "economy.barracks_short",
                EconomyAction::Worker => "economy.worker_short",
                EconomyAction::RifleSquad => "economy.rifles_short",
                EconomyAction::Scout => "economy.scout_short",
            }
        } else {
            match action {
                EconomyAction::Barracks => "economy.build_barracks",
                EconomyAction::Worker => "economy.train_worker",
                EconomyAction::RifleSquad => "economy.train_rifles",
                EconomyAction::Scout => "economy.train_scout",
            }
        };
        button.spawn((
            Text::new(loc.t(label_key)),
            text_font(font, 11.0),
            TextColor(INK),
            Pickable::IGNORE,
        ));
        let cost = action.cost(game).unwrap_or_default();
        let w = loc.t("economy.wood_short");
        let g = loc.t("economy.gold_short");
        let cost_label = if desktop {
            format!("{}{w} {}{g}", cost.wood, cost.gold)
        } else {
            format!("{} {w} · {} {g}", cost.wood, cost.gold)
        };
        button.spawn((
            ActionCost,
            Text::new(cost_label),
            text_font(font, if desktop { 9.0 } else { 10.0 }),
            TextColor(INK_MUTED),
            Pickable::IGNORE,
        ));
    });
}

fn activate_economy(
    event: On<Activate>,
    buttons: Query<(&EconomyAction, Has<InteractionDisabled>)>,
    mut game: ResMut<Match>,
    selection: Res<Selection>,
    mut targeting: ResMut<Targeting>,
    pause: Res<PauseMenu>,
    phone: Res<PhoneHud>,
    menu: Res<Menu>,
) {
    let Ok((&action, disabled)) = buttons.get(event.entity) else {
        return;
    };
    if disabled
        || !game.accepts_orders
        || world_input_blocked(Some(&pause), Some(&menu))
        || phone.panel.is_some()
    {
        return;
    }
    if action == EconomyAction::Barracks && *targeting == Targeting::Barracks {
        *targeting = Targeting::None;
        return;
    }
    if !action.affordable(&game) {
        return;
    }
    let units = action.ready_units(&game, &selection).collect::<Vec<_>>();
    if units.is_empty() {
        return;
    }
    match action {
        EconomyAction::Barracks => *targeting = Targeting::Barracks,
        EconomyAction::Worker => {
            game.queue_command(Command::TrainWorker { units });
        }
        EconomyAction::RifleSquad => {
            game.queue_command(Command::TrainRifleSquad { units });
        }
        EconomyAction::Scout => {
            game.queue_command(Command::TrainScout { units });
        }
    }
}

type EconomyButton<'a> = (
    Entity,
    &'a EconomyAction,
    &'a ActionLayout,
    &'a Hovered,
    Has<Pressed>,
    Has<InteractionDisabled>,
    &'a Children,
    &'a mut Node,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
);
type EconomyCardFilter = (
    With<EconomyCard>,
    Without<EconomyAction>,
    Without<ResourceBar>,
);

fn update_skirmish(
    mut commands: Commands,
    game: Res<Match>,
    selection: Res<Selection>,
    targeting: Res<Targeting>,
    phone: Res<PhoneHud>,
    pause: Res<PauseMenu>,
    loc: Res<Localization>,
    size: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut cards: Query<&mut Node, EconomyCardFilter>,
    mut resource_bars: Query<&mut Node, (With<ResourceBar>, Without<EconomyAction>)>,
    mut buttons: Query<EconomyButton>,
    mut balances: Query<(&ResourceLabel, &mut Text), Without<EconomyHint>>,
    mut hints: Query<&mut Text, With<EconomyHint>>,
    mut inks: Query<(&mut TextColor, Has<ActionCost>)>,
) {
    let Some(resources) = game.view.resources() else {
        return;
    };
    for (label, mut text) in &mut balances {
        text.set_if_neq(Text::new(format!(
            "{} {}",
            resource_label(label.0, &loc),
            resources.get(label.0),
        )));
    }
    let size = size.size() / scale.0;
    let hud = HudLayout::new(size, !selection.units.is_empty(), phone.panel);
    let count = visible_actions(&game, &selection);
    for mut bar in &mut resource_bars {
        let position = resource_bar_position(&hud);
        bar.top = px(position.y);
        bar.left = px(position.x);
        bar.display = if phone.panel.is_none() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut card in &mut cards {
        card.display = if hud.phone && count > 0 && phone.panel.is_none() && !pause.open {
            Display::Flex
        } else {
            Display::None
        };
        card.bottom = px(actions_bottom(&hud));
        card.max_width = px((size.x - 16.0).max(0.0));
    }
    for mut hint in &mut hints {
        hint.set_if_neq(Text::new(
            economy_hint(&game, &selection, *targeting, &loc).unwrap_or_default(),
        ));
    }
    for (
        entity,
        action,
        layout,
        hovered,
        pressed,
        was_disabled,
        children,
        mut node,
        mut fill,
        mut border,
    ) in &mut buttons
    {
        let visible = (*layout == ActionLayout::Phone) == hud.phone
            && action.units(&game, &selection).next().is_some();
        if *layout == ActionLayout::Phone {
            node.width = px(action_width(size.x, count));
        }
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        let selected = *action == EconomyAction::Barracks && *targeting == Targeting::Barracks;
        let disabled = !game.accepts_orders
            || !visible
            || (!selected
                && (!action.affordable(&game)
                    || action.ready_units(&game, &selection).next().is_none()));
        if disabled != was_disabled {
            if disabled {
                commands.entity(entity).insert(InteractionDisabled);
            } else {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
        }
        let look = button_look(Tone::Item, selected, hovered.get(), pressed, disabled);
        fill.set_if_neq(BackgroundColor(look.fill));
        border.set_if_neq(BorderColor::all(if selected { SIGNAL } else { look.edge }));
        for child in children {
            if let Ok((mut text, cost)) = inks.get_mut(*child) {
                text.0 = if cost && !disabled {
                    INK_MUTED
                } else {
                    look.ink
                };
            }
        }
    }
}

pub fn resource_bar_position(hud: &HudLayout) -> Vec2 {
    if hud.phone && hud.minimap.min.x < 243.0 {
        Vec2::new(8.0, hud.minimap.max.y + 8.0)
    } else {
        Vec2::new(68.0, 12.0)
    }
}

pub fn economy_hint(
    game: &Match,
    selection: &Selection,
    targeting: Targeting,
    loc: &Localization,
) -> Option<String> {
    if targeting == Targeting::Barracks {
        return Some(targeting.prompt(loc).to_owned());
    }
    if let Some(work) = selection
        .units
        .iter()
        .find_map(|&unit| economic_activity(game, unit))
    {
        return Some(work.description(loc));
    }
    if workers(game, selection).next().is_some() && game.view.is_skirmish() {
        return Some(
            loc.t(if selection.touch_mode {
                "economy.hint_tap_deposit"
            } else {
                "economy.hint_click_deposit"
            })
            .to_owned(),
        );
    }
    if game.view.is_skirmish()
        && EconomyAction::Worker
            .units(game, selection)
            .next()
            .is_some()
    {
        return Some(loc.t("economy.hint_train_workers").to_owned());
    }
    if game.view.is_skirmish()
        && EconomyAction::RifleSquad
            .units(game, selection)
            .next()
            .is_some()
    {
        return Some(
            loc.t(
                if EconomyAction::Scout.units(game, selection).next().is_some() {
                    "economy.hint_train_both"
                } else {
                    "economy.hint_train_rifles"
                },
            )
            .to_owned(),
        );
    }
    None
}

pub fn economic_state<'a>(game: &Match, unit: UnitId, loc: &'a Localization) -> Option<&'a str> {
    economic_activity(game, unit).map(|activity| activity.state(loc))
}

enum EconomicActivity {
    ApproachingSite,
    Building(u32),
    Delivering(ResourceKind, u32),
    Carrying(ResourceKind, u32),
    Gathering(ResourceKind),
    Training(Training, usize),
}

impl EconomicActivity {
    fn state<'a>(&self, loc: &'a Localization) -> &'a str {
        let key = match self {
            Self::ApproachingSite => "economy.state_en_route",
            Self::Building(_) => "economy.state_building",
            Self::Delivering(..) => "economy.state_delivering",
            Self::Carrying(..) => "economy.state_carrying",
            Self::Gathering(_) => "economy.state_gathering",
            Self::Training(training, _) if training.remaining == 0 => "economy.state_waiting",
            Self::Training(..) => "economy.state_training",
        };
        loc.t(key)
    }

    fn description(&self, loc: &Localization) -> String {
        match *self {
            Self::ApproachingSite => loc.t("economy.desc_en_route").to_owned(),
            Self::Building(ticks) => {
                let seconds = ticks.div_ceil(rts_base::TICK_RATE).to_string();
                loc.fmt("economy.desc_building", &[("seconds", &seconds)])
            }
            Self::Delivering(kind, amount) | Self::Carrying(kind, amount) => {
                let state = self.state(loc);
                let amount = amount.to_string();
                let resource = resource_label(kind, loc).to_lowercase();
                loc.fmt(
                    "economy.desc_carrying",
                    &[
                        ("state", state),
                        ("amount", &amount),
                        ("resource", &resource),
                    ],
                )
            }
            Self::Gathering(kind) => {
                let resource = resource_label(kind, loc).to_lowercase();
                loc.fmt("economy.desc_gathering", &[("resource", &resource)])
            }
            Self::Training(training, queued) => {
                let name = match training.kind {
                    TrainingKind::Worker => loc.unit_name("worker"),
                    TrainingKind::RifleSquad => loc.unit_name("soldier"),
                    TrainingKind::Scout => loc.unit_name("scout"),
                };
                let progress = if training.remaining == 0 {
                    loc.t("economy.state_waiting").to_owned()
                } else {
                    format!("{}s", training.remaining.div_ceil(rts_base::TICK_RATE))
                };
                let queued = queued.to_string();
                loc.fmt(
                    "economy.desc_training",
                    &[
                        ("name", &name),
                        ("progress", &progress),
                        ("queued", &queued),
                    ],
                )
            }
        }
    }
}

fn economic_activity(game: &Match, unit: UnitId) -> Option<EconomicActivity> {
    if let Some(remaining) = game.view.construction_remaining(unit) {
        return Some(if game.view.move_target(unit).is_some() {
            EconomicActivity::ApproachingSite
        } else {
            EconomicActivity::Building(remaining)
        });
    }
    let gathering = game
        .view
        .gather_target(unit)
        .and_then(|id| game.view.deposit(id));
    if let Some((kind, amount)) = game.view.cargo(unit) {
        return Some(if gathering.is_some() {
            EconomicActivity::Delivering(kind, amount)
        } else {
            EconomicActivity::Carrying(kind, amount)
        });
    }
    if let Some(deposit) = gathering {
        return Some(EconomicActivity::Gathering(deposit.kind));
    }
    let queue = game.view.production(unit)?;
    queue
        .first()
        .map(|job| EconomicActivity::Training(*job, queue.len()))
}

/// Shows deposits once discovered, with the stock last seen, dimmed while out of sight.
fn update_deposits(
    mut commands: Commands,
    game: Res<Match>,
    assets: Res<AssetServer>,
    loc: Res<Localization>,
    mut sprites: Query<(&DepositSprite, &mut Visibility, &mut Sprite)>,
    mut shown: Local<Vec<DepositId>>,
) {
    if game.is_added() {
        shown.clear();
    }
    for (id, deposit) in game.view.deposits() {
        if !shown.contains(&id) {
            shown.push(id);
            commands.spawn((
                Name::new(resource_label(deposit.kind, &loc).to_owned()),
                DespawnOnExit(AppState::InGame),
                DepositSprite(id),
                Sprite {
                    image: assets.load(resource_icon(deposit.kind)),
                    custom_size: Some(Vec2::splat(DEPOSIT_SIZE)),
                    ..default()
                },
                Transform::from_translation(to_world(deposit.pos).extend(0.5)),
            ));
        }
    }
    for (sprite, mut visibility, mut image) in &mut sprites {
        let Some(deposit) = game.view.deposit(sprite.0) else {
            continue;
        };
        visibility.set_if_neq(if deposit.remaining > 0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let color = if game.view.fog().sees(deposit.pos) {
            Color::WHITE
        } else {
            Color::srgb(0.5, 0.5, 0.55)
        };
        if image.color != color {
            image.color = color;
        }
    }
}

fn selected_kind<'a>(
    game: &'a Match,
    selection: &'a Selection,
    kind: &'a str,
) -> impl Iterator<Item = UnitId> + 'a {
    let kind = game.view.rules().unit_kind(kind);
    selection.units.iter().copied().filter(move |&id| {
        game.view
            .unit(id)
            .is_some_and(|unit| Some(unit.kind) == kind && unit.owner == game.local_player)
    })
}

pub fn workers<'a>(game: &'a Match, selection: &'a Selection) -> impl Iterator<Item = UnitId> + 'a {
    selected_kind(game, selection, "worker")
}

pub fn deposit_at(game: &Match, position: Vec2, scale: f32) -> Option<DepositId> {
    let radius = (DEPOSIT_SIZE / 2.0).max(14.0 * scale);
    game.view
        .deposits()
        .filter(|(_, deposit)| {
            deposit.remaining > 0
                && game.view.visibility(rts_base::tile_of(deposit.pos))
                    == rts_base::TileVisibility::Visible
        })
        .map(|(id, deposit)| (id, position.distance_squared(to_world(deposit.pos))))
        .filter(|(_, distance)| *distance <= radius * radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

pub fn place_barracks(game: &mut Match, selection: &Selection, target: SimPos) -> bool {
    if !can_place_barracks(game, target) {
        return false;
    }
    let units = workers(game, selection).collect::<Vec<_>>();
    !units.is_empty() && game.queue_command(Command::BuildBarracks { units, target })
}

fn can_place_barracks(game: &Match, target: SimPos) -> bool {
    EconomyAction::Barracks.affordable(game) && game.view.can_place_barracks(target)
}

pub fn draw_construction(
    gizmos: &mut Gizmos,
    game: &Match,
    selection: &Selection,
    style: &SelectionStyle,
    preview: Option<SimPos>,
) {
    if !game.view.is_skirmish() {
        return;
    }
    let size = Vec2::splat(to_world_distance(rts_base::TILE));
    for site in workers(game, selection).filter_map(|worker| game.view.build_target(worker)) {
        gizmos.rect_2d(Isometry2d::from_translation(to_world(site)), size, SIGNAL);
    }
    if let Some(target) = preview {
        let tile = rts_base::tile_of(target);
        let center = (tile * rts_base::TILE) + SimPos::splat(rts_base::TILE / 2);
        let color = if can_place_barracks(game, target) {
            style.accepted_order_color
        } else {
            style.blocked_order_color
        };
        gizmos.rect_2d(Isometry2d::from_translation(to_world(center)), size, color);
    }
}

fn visible_actions(game: &Match, selection: &Selection) -> usize {
    if !game.view.is_skirmish() {
        return 0;
    }
    EconomyAction::ALL
        .into_iter()
        .filter(|action| action.units(game, selection).next().is_some())
        .count()
}

fn actions_bottom(hud: &HudLayout) -> f32 {
    8.0 + hud.commands.map_or(0.0, |rect| rect.height() + 8.0)
}

#[expect(clippy::cast_precision_loss, reason = "only four actions")]
fn action_width(screen_width: f32, count: usize) -> f32 {
    let gaps = count.saturating_sub(1) as f32;
    let count = count.max(1) as f32;
    (ACTION_GAP.mul_add(-gaps, screen_width - 16.0) / count).clamp(0.0, ACTION_WIDTH)
}

pub fn actions_bounds(
    game: &Match,
    selection: &Selection,
    size: Vec2,
    hud: &HudLayout,
) -> Option<Rect> {
    if !hud.phone {
        return None;
    }
    let count = visible_actions(game, selection);
    if count == 0 {
        return None;
    }
    #[expect(clippy::cast_precision_loss, reason = "only four actions")]
    let width = (action_width(size.x, count) + ACTION_GAP).mul_add(count as f32, -ACTION_GAP);
    let y = size.y - actions_bottom(hud);
    Some(Rect::from_corners(
        Vec2::new(8.0, y - ACTION_HEIGHT),
        Vec2::new(8.0 + width, y),
    ))
}

#[cfg(test)]
pub fn test_setup() -> rts_net::MatchSetup {
    rts_net::MatchSetup {
        rules: r##"(
            tile_capacity: 4,
            terrain: [(id: "grass", symbol: '.', speed_modifier: 1, color: "#477744")],
            units: [
                (id: "headquarters", sprite: "hq.png", size: 4, max_health: 20, radius: 0.5, speed: 0, building: true),
                (id: "worker", sprite: "worker.png", size: 1, max_health: 30, radius: 0.25, speed: 10),
                (id: "barracks", sprite: "barracks.png", size: 4, max_health: 50, radius: 0.5, speed: 0, building: true),
                (id: "soldier", sprite: "soldier.png", size: 2, max_health: 10, radius: 0.4, speed: 2,
                 weapon: Some((range: 2, damage: 10, cooldown: 0.5))),
                (id: "scout", sprite: "scout.png", size: 1, max_health: 10, radius: 0.3, speed: 4, sight: 8,
                 weapon: Some((range: 2, damage: 1, cooldown: 1))),
            ],
            economy: Some((
                initial: (wood: 80, gold: 80),
                worker_cost: (wood: 4, gold: 5),
                barracks_cost: (wood: 10, gold: 12),
                rifle_cost: (wood: 14, gold: 12),
                worker_ticks: 4, barracks_ticks: 5, rifle_ticks: 6, harvest_ticks: 3, carry_capacity: 8,
                scout: Some((cost: (wood: 3, gold: 2), ticks: 4)),
            )),
        )"##.into(),
        map: r#"(
            name: "Skirmish test",
            players: [(start: (0.5, 0.5)), (start: (5.5, 0.5))],
            tiles: ["......", "......", "......", "......"],
            units: [
                (kind: "headquarters", owner: 0, pos: (0.5, 0.5)),
                (kind: "headquarters", owner: 1, pos: (5.5, 0.5)),
                (kind: "soldier", owner: 0, pos: (1.5, 0.5)),
                (kind: "worker", owner: 0, pos: (1.5, 1.5)),
                (kind: "barracks", owner: 0, pos: (1.5, 3.5)),
            ],
            skirmish: Some((deposits: [
                (kind: Wood, pos: (2.5, 1.5), remaining: 11),
                (kind: Gold, pos: (2.5, 2.5), remaining: 13),
            ])),
        )"#.into(),
        seed: 0,
        players: 2,
    }
}

#[cfg(test)]
mod tests {
    use bevy::state::app::StatesPlugin;

    use super::*;

    fn activate(app: &mut App, action: EconomyAction) {
        let entity = app
            .world_mut()
            .query::<(Entity, &EconomyAction)>()
            .iter(app.world())
            .find_map(|(entity, &candidate)| (candidate == action).then_some(entity))
            .unwrap();
        app.world_mut().trigger(Activate { entity });
        app.update();
    }

    #[test]
    fn phone_economy_controls_show_balances_and_train_from_both_producers() {
        let sim = test_setup().simulation().unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, AssetPlugin::default()))
            .init_asset::<Image>()
            .insert_state(AppState::InGame)
            .insert_resource(Match::new(sim, rts_base::PlayerId(0), true))
            .insert_resource(GameFont(Handle::default()))
            .init_resource::<Selection>()
            .init_resource::<UiScale>()
            .init_resource::<PhoneHud>()
            .init_resource::<Targeting>()
            .init_resource::<PauseMenu>()
            .init_resource::<Menu>()
            .add_plugins(SkirmishPlugin);
        let mut window = Window::default();
        window.resolution.set(390.0, 844.0);
        app.world_mut().spawn((window, PrimaryWindow));
        app.world_mut()
            .resource_mut::<Selection>()
            .units
            .extend([UnitId(0), UnitId(4)]);
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&DepositSprite>()
                .iter(app.world())
                .count(),
            2
        );
        let loc = Localization::default();
        assert!(
            app.world_mut()
                .query::<(&ResourceLabel, &Text)>()
                .iter(app.world())
                .all(|(label, text)| text.0 == format!("{} 80", resource_label(label.0, &loc)))
        );

        assert_eq!(
            visible_actions(app.world().resource(), app.world().resource()),
            3
        );
        activate(&mut app, EconomyAction::Worker);
        activate(&mut app, EconomyAction::RifleSquad);
        activate(&mut app, EconomyAction::Scout);
        let mut game = app.world_mut().resource_mut::<Match>();
        let commands = std::mem::take(&mut game.pending);
        assert_eq!(
            commands,
            [
                Command::TrainWorker {
                    units: vec![UnitId(0)]
                },
                Command::TrainRifleSquad {
                    units: vec![UnitId(4)]
                },
                Command::TrainScout {
                    units: vec![UnitId(4)]
                },
            ]
        );
        game.step_with(
            commands
                .into_iter()
                .map(|command| (rts_base::PlayerId(0), command)),
        );
        app.update();
        let mut labels = app.world_mut().query::<(&ResourceLabel, &Text)>();
        assert!(
            labels
                .iter(app.world())
                .any(|(label, text)| label.0 == ResourceKind::Wood && text.0 == "Wood 59")
        );
        assert!(
            labels
                .iter(app.world())
                .any(|(label, text)| label.0 == ResourceKind::Gold && text.0 == "Gold 61")
        );
        let hint = app
            .world_mut()
            .query_filtered::<&Text, With<EconomyHint>>()
            .single(app.world())
            .unwrap();
        assert_eq!(hint.0, "Worker · 1s · 1 queued");

        app.world_mut().resource_mut::<Selection>().units = [UnitId(3)].into();
        app.update();
        activate(&mut app, EconomyAction::Barracks);
        assert_eq!(*app.world().resource::<Targeting>(), Targeting::Barracks);
        activate(&mut app, EconomyAction::Barracks);
        assert_eq!(*app.world().resource::<Targeting>(), Targeting::None);

        app.world_mut().resource_mut::<PauseMenu>().open = true;
        activate(&mut app, EconomyAction::Barracks);
        assert_eq!(*app.world().resource::<Targeting>(), Targeting::None);
        assert_eq!(app.world().resource::<Match>().pending, []);
    }

    #[test]
    fn scout_action_follows_the_rules() {
        let sim = test_setup().simulation().unwrap();
        let mut selection = Selection::default();
        selection.units.extend([UnitId(4)]);
        let game = Match::new(sim, rts_base::PlayerId(0), true);
        assert_eq!(
            EconomyAction::Scout
                .units(&game, &selection)
                .collect::<Vec<_>>(),
            [UnitId(4)]
        );
        let loc = Localization::default();
        assert_eq!(
            economy_hint(&game, &selection, Targeting::None, &loc).as_deref(),
            Some("Train rifle squads and scouts at the barracks.")
        );

        let mut setup = test_setup();
        setup.rules = setup
            .rules
            .replace("scout: Some((cost: (wood: 3, gold: 2), ticks: 4)),", "");
        let game = Match::new(setup.simulation().unwrap(), rts_base::PlayerId(0), true);
        assert_eq!(EconomyAction::Scout.units(&game, &selection).next(), None);
        assert_eq!(visible_actions(&game, &selection), 1);
        assert_eq!(
            economy_hint(&game, &selection, Targeting::None, &loc).as_deref(),
            Some("Train rifle squads at the barracks.")
        );
    }

    #[test]
    fn remembered_deposits_are_not_gather_targets() {
        let mut setup = test_setup();
        setup.map = r#"(
            name: "Remembered deposit",
            players: [(start: (0.5, 0.5)), (start: (29.5, 1.5))],
            tiles: ["..............................", ".............................."],
            units: [
                (kind: "headquarters", owner: 0, pos: (0.5, 0.5)),
                (kind: "headquarters", owner: 1, pos: (29.5, 1.5)),
                (kind: "worker", owner: 0, pos: (12.5, 0.5)),
            ],
            skirmish: Some((deposits: [
                (kind: Wood, pos: (13.5, 0.5), remaining: 11),
                (kind: Gold, pos: (1.5, 1.5), remaining: 13),
            ])),
        )"#
        .into();
        let mut game = Match::new(setup.simulation().unwrap(), rts_base::PlayerId(0), true);
        let (id, deposit) = game
            .view
            .deposits()
            .find(|(_, deposit)| deposit.kind == ResourceKind::Wood)
            .unwrap();
        let position = to_world(deposit.pos);
        assert_eq!(deposit_at(&game, position, 1.0), Some(id));

        let worker = workers(
            &game,
            &Selection {
                units: game.view.units().map(|(id, _)| id).collect(),
                ..Selection::default()
            },
        )
        .next()
        .unwrap();
        game.step_with([(
            rts_base::PlayerId(0),
            Command::Move {
                units: vec![worker],
                target: SimPos::new(25 * rts_base::TILE + rts_base::TILE / 2, rts_base::TILE / 2),
            },
        )]);
        let tile = rts_base::tile_of(deposit.pos);
        for _ in 0..200 {
            if game.view.visibility(tile) != rts_base::TileVisibility::Visible {
                break;
            }
            game.step();
        }
        assert_eq!(
            game.view.visibility(tile),
            rts_base::TileVisibility::Remembered
        );
        assert!(game.view.deposits().any(|(remembered, _)| remembered == id));
        assert_eq!(deposit_at(&game, position, 1.0), None);
    }
}
