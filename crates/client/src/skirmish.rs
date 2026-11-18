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
        app.add_systems(OnEnter(AppState::InGame), spawn_skirmish)
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
}

impl EconomyAction {
    const ALL: [Self; 3] = [Self::Barracks, Self::Worker, Self::RifleSquad];

    const fn label(self) -> &'static str {
        match self {
            Self::Barracks => "Build barracks",
            Self::Worker => "Train worker",
            Self::RifleSquad => "Rifle squad",
        }
    }

    fn cost(self, game: &Match) -> Option<Resources> {
        let rules = game.sim.rules().economy()?;
        Some(match self {
            Self::Barracks => rules.barracks_cost,
            Self::Worker => rules.worker_cost,
            Self::RifleSquad => rules.rifle_cost,
        })
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
                Self::RifleSquad => "barracks",
            },
        )
    }

    fn ready_units<'a>(
        self,
        game: &'a Match,
        selection: &'a Selection,
    ) -> impl Iterator<Item = UnitId> + 'a {
        self.units(game, selection).filter(move |&id| {
            self == Self::Barracks
                || game
                    .sim
                    .production(id)
                    .is_some_and(|queue| queue.len() < PRODUCTION_QUEUE_LIMIT)
        })
    }

    fn affordable(self, game: &Match) -> bool {
        self.cost(game)
            .zip(game.sim.resources(game.local_player))
            .is_some_and(|(cost, funds)| funds.can_afford(cost))
    }
}

const fn resource_label(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Wood => "Wood",
        ResourceKind::Gold => "Gold",
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
) {
    if !game.sim.is_skirmish() {
        return;
    }
    for (id, deposit) in game.sim.deposits() {
        commands.spawn((
            Name::new(resource_label(deposit.kind)),
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
    spawn_balances(&mut commands, &font.0, &assets);
    spawn_economy_card(&mut commands, &font.0, &game);
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

fn spawn_economy_card(commands: &mut Commands, font: &Handle<Font>, game: &Match) {
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
                    spawn_economy_button(row, action, ActionLayout::Phone, font, game);
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
) {
    if game.sim.is_skirmish()
        && let Some(&action) = index
            .checked_sub(3)
            .and_then(|offset| EconomyAction::ALL.get(offset))
    {
        spawn_economy_button(slot, action, ActionLayout::Desktop, font, game);
    }
}

fn spawn_economy_button(
    row: &mut ChildSpawnerCommands,
    action: EconomyAction,
    layout: ActionLayout,
    font: &Handle<Font>,
    game: &Match,
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
        let label = if desktop {
            match action {
                EconomyAction::Barracks => "Barracks",
                EconomyAction::Worker => "Worker",
                EconomyAction::RifleSquad => "Rifles",
            }
        } else {
            action.label()
        };
        button.spawn((
            Text::new(label),
            text_font(font, 11.0),
            TextColor(INK),
            Pickable::IGNORE,
        ));
        let cost = action.cost(game).unwrap_or_default();
        let cost_label = if desktop {
            format!("{}W {}G", cost.wood, cost.gold)
        } else {
            format!("{} W · {} G", cost.wood, cost.gold)
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
    size: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    mut cards: Query<&mut Node, EconomyCardFilter>,
    mut resource_bars: Query<&mut Node, (With<ResourceBar>, Without<EconomyAction>)>,
    mut buttons: Query<EconomyButton>,
    mut balances: Query<(&ResourceLabel, &mut Text), Without<EconomyHint>>,
    mut hints: Query<&mut Text, With<EconomyHint>>,
    mut inks: Query<(&mut TextColor, Has<ActionCost>)>,
) {
    let Some(resources) = game.sim.resources(game.local_player) else {
        return;
    };
    for (label, mut text) in &mut balances {
        text.set_if_neq(Text::new(format!(
            "{} {}",
            resource_label(label.0),
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
            economy_hint(&game, &selection, *targeting).unwrap_or_default(),
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

pub fn economy_hint(game: &Match, selection: &Selection, targeting: Targeting) -> Option<String> {
    if targeting == Targeting::Barracks {
        return Some(targeting.prompt().into());
    }
    if let Some(work) = selection
        .units
        .iter()
        .find_map(|&unit| economic_activity(game, unit))
    {
        return Some(work.description());
    }
    if workers(game, selection).next().is_some() && game.sim.is_skirmish() {
        return Some(
            if selection.touch_mode {
                "Tap a deposit to gather"
            } else {
                "Right-click a deposit to gather"
            }
            .into(),
        );
    }
    if game.sim.is_skirmish()
        && EconomyAction::Worker
            .units(game, selection)
            .next()
            .is_some()
    {
        return Some("Train workers at headquarters.".into());
    }
    if game.sim.is_skirmish()
        && EconomyAction::RifleSquad
            .units(game, selection)
            .next()
            .is_some()
    {
        return Some("Train rifle squads at the barracks.".into());
    }
    None
}

pub fn economic_state(game: &Match, unit: UnitId) -> Option<&'static str> {
    economic_activity(game, unit).map(|activity| activity.state())
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
    const fn state(&self) -> &'static str {
        match self {
            Self::ApproachingSite => "En route to build",
            Self::Building(_) => "Building",
            Self::Delivering(..) => "Delivering",
            Self::Carrying(..) => "Carrying",
            Self::Gathering(_) => "Gathering",
            Self::Training(training, _) if training.remaining == 0 => "Waiting for room",
            Self::Training(..) => "Training",
        }
    }

    fn description(&self) -> String {
        match *self {
            Self::ApproachingSite => "Worker en route to build".into(),
            Self::Building(ticks) => format!(
                "Building barracks · {}s",
                ticks.div_ceil(rts_base::TICK_RATE)
            ),
            Self::Delivering(kind, amount) | Self::Carrying(kind, amount) => {
                let state = self.state();
                format!("{state} {amount} {}", resource_label(kind).to_lowercase())
            }
            Self::Gathering(kind) => format!("Gathering {}", resource_label(kind).to_lowercase()),
            Self::Training(training, queued) => {
                let name = match training.kind {
                    TrainingKind::Worker => "Worker",
                    TrainingKind::RifleSquad => "Rifle squad",
                    TrainingKind::Scout => "Scout",
                };
                let progress = if training.remaining == 0 {
                    "Waiting for room".into()
                } else {
                    format!("{}s", training.remaining.div_ceil(rts_base::TICK_RATE))
                };
                format!("{name} · {progress} · {queued} queued")
            }
        }
    }
}

fn economic_activity(game: &Match, unit: UnitId) -> Option<EconomicActivity> {
    if let Some(remaining) = game.sim.construction_remaining(unit) {
        return Some(if game.sim.move_target(unit).is_some() {
            EconomicActivity::ApproachingSite
        } else {
            EconomicActivity::Building(remaining)
        });
    }
    let gathering = game
        .sim
        .gather_target(unit)
        .and_then(|id| game.sim.deposit(id));
    if let Some((kind, amount)) = game.sim.cargo(unit) {
        return Some(if gathering.is_some() {
            EconomicActivity::Delivering(kind, amount)
        } else {
            EconomicActivity::Carrying(kind, amount)
        });
    }
    if let Some(deposit) = gathering {
        return Some(EconomicActivity::Gathering(deposit.kind));
    }
    let queue = game.sim.production(unit)?;
    queue
        .first()
        .map(|job| EconomicActivity::Training(*job, queue.len()))
}

fn update_deposits(game: Res<Match>, mut sprites: Query<(&DepositSprite, &mut Visibility)>) {
    for (sprite, mut visibility) in &mut sprites {
        let visible = game
            .sim
            .deposit(sprite.0)
            .is_some_and(|deposit| deposit.remaining > 0);
        visibility.set_if_neq(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

fn selected_kind<'a>(
    game: &'a Match,
    selection: &'a Selection,
    kind: &'a str,
) -> impl Iterator<Item = UnitId> + 'a {
    let kind = game.sim.rules().unit_kind(kind);
    selection.units.iter().copied().filter(move |&id| {
        game.sim
            .unit(id)
            .is_some_and(|unit| Some(unit.kind) == kind && unit.owner == game.local_player)
    })
}

pub fn workers<'a>(game: &'a Match, selection: &'a Selection) -> impl Iterator<Item = UnitId> + 'a {
    selected_kind(game, selection, "worker")
}

pub fn deposit_at(game: &Match, position: Vec2, scale: f32) -> Option<DepositId> {
    let radius = (DEPOSIT_SIZE / 2.0).max(14.0 * scale);
    game.sim
        .deposits()
        .filter(|(_, deposit)| deposit.remaining > 0)
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
    EconomyAction::Barracks.affordable(game)
        && game.sim.can_place_barracks(game.local_player, target)
}

pub fn draw_construction(
    gizmos: &mut Gizmos,
    game: &Match,
    selection: &Selection,
    style: &SelectionStyle,
    preview: Option<SimPos>,
) {
    if !game.sim.is_skirmish() {
        return;
    }
    let size = Vec2::splat(to_world_distance(rts_base::TILE));
    for site in workers(game, selection).filter_map(|worker| game.sim.build_target(worker)) {
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
    if !game.sim.is_skirmish() {
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

#[expect(clippy::cast_precision_loss, reason = "only three actions")]
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
    #[expect(clippy::cast_precision_loss, reason = "only three actions")]
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
            ],
            economy: Some((
                initial: (wood: 80, gold: 80),
                worker_cost: (wood: 4, gold: 5),
                barracks_cost: (wood: 10, gold: 12),
                rifle_cost: (wood: 14, gold: 12),
                worker_ticks: 4, barracks_ticks: 5, rifle_ticks: 6, harvest_ticks: 3, carry_capacity: 8,
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
        assert!(
            app.world_mut()
                .query::<(&ResourceLabel, &Text)>()
                .iter(app.world())
                .all(|(label, text)| text.0 == format!("{} 80", resource_label(label.0)))
        );

        activate(&mut app, EconomyAction::Worker);
        activate(&mut app, EconomyAction::RifleSquad);
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
            ]
        );
        game.sim.step_with_commands(
            commands
                .into_iter()
                .map(|command| (rts_base::PlayerId(0), command)),
        );
        app.update();
        let mut labels = app.world_mut().query::<(&ResourceLabel, &Text)>();
        assert!(
            labels
                .iter(app.world())
                .any(|(label, text)| label.0 == ResourceKind::Wood && text.0 == "Wood 62")
        );
        assert!(
            labels
                .iter(app.world())
                .any(|(label, text)| label.0 == ResourceKind::Gold && text.0 == "Gold 63")
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
}
