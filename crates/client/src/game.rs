//! Match lifecycle: load data, start the simulation, and run it at the fixed tick rate.

use bevy::{
    asset::{LoadState, LoadedFolder},
    prelude::*,
};
use rts_ai::{Difficulty, Opponent};
use rts_base::{Command, PlayerId, PlayerKnowledge, PlayerView, Sim, TICK_RATE, Volley};
use rts_net::{MAX_COMMANDS_PER_TURN, MatchSetup};

use rts_assets::{Graphics, MapAsset, RulesAsset};

use crate::preview::map_preview;

const RULES: &str = "data/core.rules.ron";
const MAPS: &str = "maps";
const GRAPHICS: &str = "data/client.graphics.ron";

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Loading,
    Menu,
    Restarting,
    InGame,
}

/// The running match as this client knows it.
///
/// Everything the interface draws comes from [`Match::view`], the local player's fog-filtered
/// knowledge. Local matches also hold the authoritative simulation in [`LocalAuthority`]; online
/// matches never see it.
#[derive(Resource)]
pub struct Match {
    pub view: PlayerView,
    local: Option<LocalAuthority>,
    replay_perspective: Option<ReplayPerspective>,
    /// The player this client controls.
    pub local_player: PlayerId,
    pub accepts_orders: bool,
    /// Orders in view IDs, waiting for the next delayed batch.
    pub pending: Vec<Command>,
    /// Volleys of stepped ticks, by tick, until the effects layer draws them.
    pub volleys: Vec<(u32, Volley)>,
}

/// The other display perspective for a full replay. Both views follow every replay tick.
struct ReplayPerspective {
    other: PlayerView,
    open_knowledge: PlayerKnowledge,
}

/// The authoritative simulation of a local match and what each player knows of it.
pub struct LocalAuthority {
    pub sim: Sim,
    /// The local player's knowledge; it produces [`Match::view`]'s frames.
    pub knowledge: PlayerKnowledge,
    /// Computer opponents, each limited to its own knowledge.
    pub opponents: Vec<Seat>,
}

/// A computer opponent and the view it plays from.
pub struct Seat {
    pub knowledge: PlayerKnowledge,
    pub view: PlayerView,
    pub opponent: Opponent,
}

impl Seat {
    pub fn new(sim: &Sim, player: PlayerId, difficulty: Difficulty) -> Self {
        let (knowledge, view) = PlayerView::observe(sim, player);
        let opponent = Opponent::new(&view, difficulty);
        Self {
            knowledge,
            view,
            opponent,
        }
    }

    /// This tick's orders, bound to the authority.
    pub fn commands(&mut self, sim: &Sim) -> Vec<Command> {
        self.opponent
            .commands(&self.view)
            .iter()
            .filter_map(|command| self.knowledge.translate_command(sim, command))
            .collect()
    }
}

impl Match {
    /// A local match on `sim`, seen by `local_player`.
    pub fn new(sim: Sim, local_player: PlayerId, accepts_orders: bool) -> Self {
        let (knowledge, view) = PlayerView::observe(&sim, local_player);
        Self::with_local(sim, knowledge, view, accepts_orders)
    }

    fn with_local(
        sim: Sim,
        knowledge: PlayerKnowledge,
        view: PlayerView,
        accepts_orders: bool,
    ) -> Self {
        let mut game = Self::online(view, accepts_orders);
        game.local = Some(LocalAuthority {
            sim,
            knowledge,
            opponents: Vec::new(),
        });
        game
    }

    pub fn full_replay(sim: Sim, player: PlayerId) -> Self {
        let (knowledge, view) = PlayerView::observe_full_replay(&sim, player, true);
        let (open_knowledge, other) = PlayerView::observe_full_replay(&sim, player, false);
        let mut game = Self::with_local(sim, knowledge, view, false);
        game.replay_perspective = Some(ReplayPerspective {
            other,
            open_knowledge,
        });
        game
    }

    /// A match whose frames come from elsewhere: a server or a player-perspective replay.
    pub const fn online(view: PlayerView, accepts_orders: bool) -> Self {
        let local_player = view.player();
        Self {
            view,
            local: None,
            replay_perspective: None,
            local_player,
            accepts_orders,
            pending: Vec::new(),
            volleys: Vec::new(),
        }
    }

    pub const fn local_mut(&mut self) -> Option<&mut LocalAuthority> {
        self.local.as_mut()
    }

    /// The authoritative simulation of a local match.
    pub fn local_sim(&self) -> Option<&Sim> {
        self.local.as_ref().map(|local| &local.sim)
    }

    pub const fn can_toggle_replay_fog(&self) -> bool {
        self.replay_perspective.is_some()
    }

    pub fn toggle_replay_fog(&mut self) {
        if let Some(perspective) = &mut self.replay_perspective {
            std::mem::swap(&mut self.view, &mut perspective.other);
            self.volleys.clear();
        }
    }

    pub const fn terrain_revision(&self) -> (u32, bool) {
        (self.view.terrain_revision(), self.view.intro().fog_disabled)
    }

    /// Updates every view after the local simulation stepped.
    pub fn observe_local(&mut self) {
        let Some(local) = &mut self.local else {
            return;
        };
        let frame = local.knowledge.observe(&local.sim);
        for seat in &mut local.opponents {
            seat.view
                .apply(seat.knowledge.observe(&local.sim))
                .expect("frames from the local authority");
        }
        let fogged = if let Some(perspective) = &mut self.replay_perspective {
            let (fogged, open) = if self.view.intro().fog_disabled {
                (&mut perspective.other, &mut self.view)
            } else {
                (&mut self.view, &mut perspective.other)
            };
            open.apply(perspective.open_knowledge.observe(&local.sim))
                .expect("frames from the local authority");
            fogged
        } else {
            &mut self.view
        };
        fogged
            .apply(frame)
            .expect("frames from the local authority");
        self.record_volleys();
    }

    /// Steps a local match with no orders.
    #[cfg(test)]
    pub fn step(&mut self) {
        if let Some(local) = &mut self.local {
            local.sim.step();
        }
        self.observe_local();
    }

    /// Steps a local match with orders in view IDs. For tests.
    #[cfg(test)]
    pub fn step_with(&mut self, commands: impl IntoIterator<Item = (PlayerId, Command)>) {
        if let Some(local) = &mut self.local {
            let bound: Vec<_> = commands
                .into_iter()
                .filter_map(|(player, command)| {
                    let command = if player == local.knowledge.player() {
                        local.knowledge.translate_command(&local.sim, &command)?
                    } else {
                        command
                    };
                    Some((player, command))
                })
                .collect();
            local.sim.step_with_commands(bound);
        }
        self.observe_local();
    }

    /// Removes a unit from a local match and steps, so every view sees it vanish. For tests.
    #[cfg(test)]
    pub fn remove_unit(&mut self, id: rts_base::UnitId) {
        if let Some(local) = &mut self.local {
            local.sim.remove_unit(id);
        }
        self.step();
    }

    /// Restarts a local match on another simulation. For tests.
    #[cfg(test)]
    pub fn replace_sim(&mut self, sim: Sim) {
        let accepts_orders = self.accepts_orders;
        *self = Self::new(sim, self.local_player, accepts_orders);
    }

    /// Keeps the volleys of the tick just applied. Several ticks may run before a frame
    /// draws them; only the newest are kept if nothing drains them.
    pub fn record_volleys(&mut self) {
        const LIMIT: usize = 256;
        let tick = self.view.tick();
        let volleys = self.view.volleys().iter().cloned();
        self.volleys.extend(volleys.map(|volley| (tick, volley)));
        let excess = self.volleys.len().saturating_sub(LIMIT);
        self.volleys.drain(..excess);
    }

    /// Returns whether input was collected for the next delayed local/network batch.
    pub fn queue_command(&mut self, command: Command) -> bool {
        if !self.accepts_orders || self.pending.len() >= MAX_COMMANDS_PER_TURN {
            return false;
        }
        self.pending.push(command);
        if rts_net::validate_commands(&self.pending).is_err() {
            self.pending.pop();
            return false;
        }
        true
    }
}

/// A bundled map ready for solo play.
pub struct CatalogMap {
    pub name: String,
    pub setup: MatchSetup,
    pub preview: Handle<Image>,
    pub skirmish: bool,
}

/// Bundled maps, sorted by name. Startup only opens the menu when at least one is valid.
#[derive(Resource)]
pub struct MapCatalog(pub Vec<CatalogMap>);

impl MapCatalog {
    pub fn preferred(&self) -> usize {
        self.0.iter().position(|map| map.skirmish).unwrap_or(0)
    }
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .insert_resource(Time::<Fixed>::from_hz(f64::from(TICK_RATE)))
            .add_systems(Startup, load_match_data)
            .add_systems(Update, start_match.run_if(resource_exists::<PendingMatch>));
    }
}

/// Data files being loaded at startup.
#[derive(Resource)]
struct PendingMatch {
    rules: Handle<RulesAsset>,
    maps: Handle<LoadedFolder>,
    graphics: Handle<Graphics>,
}

fn load_match_data(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(PendingMatch {
        rules: assets.load(RULES),
        maps: assets.load_folder(MAPS),
        graphics: assets.load(GRAPHICS),
    });
}

fn start_match(
    mut commands: Commands,
    pending: Res<PendingMatch>,
    rules: Res<Assets<RulesAsset>>,
    folders: Res<Assets<LoadedFolder>>,
    maps: Res<Assets<MapAsset>>,
    graphics: Res<Assets<Graphics>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    for (path, id) in [
        (RULES, pending.rules.id().untyped()),
        (MAPS, pending.maps.id().untyped()),
        (GRAPHICS, pending.graphics.id().untyped()),
    ] {
        if let Some(LoadState::Failed(error)) = assets.get_load_state(id) {
            error!("cannot load `{path}`: {error}");
            commands.remove_resource::<PendingMatch>();
            return;
        }
    }
    let (Some(rules), Some(folder), Some(graphics)) = (
        rules.get(&pending.rules),
        folders.get(&pending.maps),
        graphics.get(&pending.graphics),
    ) else {
        return;
    };
    let handles: Vec<Handle<MapAsset>> = folder
        .handles
        .iter()
        .filter_map(|handle| handle.clone().try_typed().ok())
        .collect();
    // A map that fails to load is skipped below; wait only for maps still loading.
    if handles.iter().any(|handle| {
        assets
            .get_load_state(handle)
            .is_some_and(|state| state.is_loading())
    }) {
        return;
    }
    commands.remove_resource::<PendingMatch>();
    let mut catalog: Vec<CatalogMap> = handles
        .iter()
        .filter_map(|handle| {
            let map = maps.get(handle)?;
            let setup = MatchSetup {
                rules: rules.1.clone(),
                map: map.1.clone(),
                seed: 0,
                players: u8::try_from(map.0.players.len()).ok()?,
            };
            match setup.simulation() {
                Ok(sim) => Some(CatalogMap {
                    name: map.0.name.clone(),
                    preview: images.add(map_preview(&sim, graphics)),
                    skirmish: sim.is_skirmish(),
                    setup,
                }),
                Err(error) => {
                    error!("skipping map `{}`: {error}", map.0.name);
                    None
                }
            }
        })
        .collect();
    if catalog.is_empty() {
        error!("no playable map in `{MAPS}`");
        return;
    }
    catalog.sort_by(|a, b| a.name.cmp(&b.name));
    commands.insert_resource(MapCatalog(catalog));
    commands.insert_resource(graphics.clone());
    next_state.set(AppState::Menu);
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    use bevy::{
        asset::io::{
            AssetSourceBuilder, AssetSourceId,
            memory::{Dir, MemoryAssetReader},
        },
        state::app::StatesPlugin,
    };
    use rts_assets::DataPlugin;

    use super::*;

    const BUNDLED_GRAPHICS: &str = include_str!("../../../assets/data/client.graphics.ron");

    fn loading_app(graphics: Option<&str>) -> App {
        loading_app_with_maps(
            graphics,
            &[
                (
                    "maps/proving_grounds.map.ron",
                    include_str!("../../../assets/maps/proving_grounds.map.ron"),
                ),
                (
                    "maps/crossroads.map.ron",
                    include_str!("../../../assets/maps/crossroads.map.ron"),
                ),
                (
                    "maps/frontier.map.ron",
                    include_str!("../../../assets/maps/frontier.map.ron"),
                ),
            ],
        )
    }

    fn loading_app_with_maps(graphics: Option<&str>, maps: &[(&str, &str)]) -> App {
        let dir = Dir::default();
        dir.insert_asset_text(
            Path::new(RULES),
            include_str!("../../../assets/data/core.rules.ron"),
        );
        for (path, text) in maps {
            dir.insert_asset_text(Path::new(path), text);
        }
        if let Some(graphics) = graphics {
            dir.insert_asset_text(Path::new(GRAPHICS), graphics);
        }
        let mut app = App::new();
        app.register_asset_source(
            AssetSourceId::Default,
            AssetSourceBuilder::new(move || Box::new(MemoryAssetReader { root: dir.clone() })),
        )
        .add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            DataPlugin,
            GamePlugin,
        ))
        .init_asset::<Image>();
        app
    }

    fn finish_loading(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.update();
            if !app.world().contains_resource::<PendingMatch>() {
                // Apply the state transition queued by start_match.
                app.update();
                return;
            }
            assert!(Instant::now() < deadline, "asset loading timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn startup_installs_graphics_before_opening_menu() {
        let custom = BUNDLED_GRAPHICS.replace("(0.20, 0.45, 0.90)", "(0.1, 0.2, 0.3)");
        let mut app = loading_app(Some(&custom));
        assert!(!app.world().contains_resource::<Graphics>());
        finish_loading(&mut app);
        assert_eq!(
            *app.world().resource::<State<AppState>>().get(),
            AppState::Menu
        );
        assert_eq!(
            app.world().resource::<Graphics>().player_color(0),
            Color::srgb(0.1, 0.2, 0.3)
        );
        let catalog = &app.world().resource::<MapCatalog>().0;
        let names: Vec<_> = catalog.iter().map(|map| map.name.as_str()).collect();
        assert_eq!(names, ["Crossroads", "Frontier", "Proving Grounds"]);
        assert!(catalog[1].skirmish);
        for map in catalog {
            assert!(map.setup.simulation().is_ok());
            assert!(
                app.world()
                    .resource::<Assets<Image>>()
                    .contains(&map.preview)
            );
            assert_eq!(
                map.setup.rules,
                include_str!("../../../assets/data/core.rules.ron")
            );
        }
        assert_eq!(catalog[0].setup.players, 4);
    }

    #[test]
    fn invalid_maps_are_skipped_and_no_valid_map_blocks_startup() {
        let broken = (
            "maps/broken.map.ron",
            "(name: \"Broken\", players: [(start: (0.0, 0.0))], tiles: [\"?\"])",
        );
        let valid = (
            "maps/proving_grounds.map.ron",
            include_str!("../../../assets/maps/proving_grounds.map.ron"),
        );
        let mut app = loading_app_with_maps(Some(BUNDLED_GRAPHICS), &[broken, valid]);
        finish_loading(&mut app);
        let names: Vec<_> = app
            .world()
            .resource::<MapCatalog>()
            .0
            .iter()
            .map(|map| map.name.clone())
            .collect();
        assert_eq!(names, ["Proving Grounds"]);

        let mut app = loading_app_with_maps(Some(BUNDLED_GRAPHICS), &[broken]);
        finish_loading(&mut app);
        assert_eq!(
            *app.world().resource::<State<AppState>>().get(),
            AppState::Loading
        );
        assert!(!app.world().contains_resource::<MapCatalog>());
    }

    #[test]
    fn invalid_or_missing_graphics_keep_startup_in_loading() {
        for graphics in [None, Some("()"), Some("not ron")] {
            let mut app = loading_app(graphics);
            finish_loading(&mut app);
            assert_eq!(
                *app.world().resource::<State<AppState>>().get(),
                AppState::Loading
            );
            assert!(!app.world().contains_resource::<Graphics>());
            assert!(!app.world().contains_resource::<MapCatalog>());
        }
    }
}
