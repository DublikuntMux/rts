//! Match lifecycle: load data, start the simulation, and run it at the fixed tick rate.

use std::collections::BTreeMap;

use bevy::{asset::LoadState, prelude::*};
use rts_base::{Command, IVec2 as SimPos, PlayerId, Sim, TICK_RATE, UnitId};
use rts_net::{MAX_COMMANDS_PER_TURN, MatchSetup};

use rts_assets::{Graphics, MapAsset, RulesAsset};

const RULES: &str = "data/core.rules.ron";
const MAP: &str = "maps/proving_grounds.map.ron";
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

#[derive(Resource)]
pub struct MatchData(pub MatchSetup);

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

/// Data files for the match being loaded.
#[derive(Resource)]
struct PendingMatch {
    rules: Handle<RulesAsset>,
    map: Handle<MapAsset>,
    graphics: Handle<Graphics>,
}

fn load_match_data(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(PendingMatch {
        rules: assets.load(RULES),
        map: assets.load(MAP),
        graphics: assets.load(GRAPHICS),
    });
}

fn start_match(
    mut commands: Commands,
    pending: Res<PendingMatch>,
    rules: Res<Assets<RulesAsset>>,
    maps: Res<Assets<MapAsset>>,
    graphics: Res<Assets<Graphics>>,
    assets: Res<AssetServer>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    for (path, id) in [
        (RULES, pending.rules.id().untyped()),
        (MAP, pending.map.id().untyped()),
        (GRAPHICS, pending.graphics.id().untyped()),
    ] {
        if let Some(LoadState::Failed(error)) = assets.get_load_state(id) {
            error!("cannot load `{path}`: {error}");
            commands.remove_resource::<PendingMatch>();
            return;
        }
    }
    let (Some(rules), Some(map), Some(graphics)) = (
        rules.get(&pending.rules),
        maps.get(&pending.map),
        graphics.get(&pending.graphics),
    ) else {
        return;
    };
    commands.remove_resource::<PendingMatch>();
    let setup = MatchSetup {
        rules: rules.1.clone(),
        map: map.1.clone(),
        seed: 0,
        players: u8::try_from(map.0.players.len()).unwrap_or(0),
    };
    match setup.simulation() {
        Ok(_) => {
            commands.insert_resource(MatchData(setup));
            commands.insert_resource(graphics.clone());
            next_state.set(AppState::Menu);
        }
        Err(error) => error!("cannot start `{MAP}`: {error}"),
    }
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
        let dir = Dir::default();
        dir.insert_asset_text(
            Path::new(RULES),
            include_str!("../../../assets/data/core.rules.ron"),
        );
        dir.insert_asset_text(
            Path::new(MAP),
            include_str!("../../../assets/maps/proving_grounds.map.ron"),
        );
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
        ));
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
        let setup = &app.world().resource::<MatchData>().0;
        assert!(setup.simulation().is_ok());
        assert_eq!(
            setup.rules,
            include_str!("../../../assets/data/core.rules.ron")
        );
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
            assert!(!app.world().contains_resource::<MatchData>());
        }
    }
}
