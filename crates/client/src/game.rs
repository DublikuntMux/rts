//! Match lifecycle: load data, start the simulation, and run it at the fixed tick rate.

use std::collections::BTreeMap;

use bevy::{
    asset::{LoadState, LoadedFolder},
    prelude::*,
};
use rts_base::{Command, IVec2 as SimPos, PlayerId, Sim, TICK_RATE, UnitId};
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
    InGame,
}

/// The running match.
#[derive(Resource)]
pub struct Match {
    pub sim: Sim,
    /// The player this client controls.
    pub local_player: PlayerId,
    pub accepts_orders: bool,
    pub pending: Vec<Command>,
}

impl Match {
    pub const fn new(sim: Sim, local_player: PlayerId, accepts_orders: bool) -> Self {
        Self {
            sim,
            local_player,
            accepts_orders,
            pending: Vec::new(),
        }
    }

    /// Collects input for the next delayed local/network batch, never applying it optimistically.
    pub fn queue_command(&mut self, command: Command) {
        if self.accepts_orders && self.pending.len() < MAX_COMMANDS_PER_TURN {
            self.pending.push(command);
            if rts_net::validate_commands(&self.pending).is_err() {
                self.pending.pop();
            }
        }
    }
}

/// A bundled map ready for solo play.
pub struct CatalogMap {
    pub name: String,
    pub setup: MatchSetup,
    pub preview: Handle<Image>,
}

/// Bundled maps, sorted by name. Startup only opens the menu when at least one is valid.
#[derive(Resource)]
pub struct MapCatalog(pub Vec<CatalogMap>);

/// Presentation-only positions immediately before the latest simulated tick.
#[derive(Resource, Default)]
pub struct PreviousPositions(pub BTreeMap<UnitId, SimPos>);

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .init_resource::<PreviousPositions>()
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
        assert_eq!(names, ["Crossroads", "Proving Grounds"]);
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
