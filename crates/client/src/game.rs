//! Match lifecycle: load data, start the simulation, and run it at the fixed tick rate.

use bevy::prelude::*;
use rts_base::{PlayerId, Sim, TICK_RATE};

use rts_assets::{MapAsset, RulesAsset};

const RULES: &str = "data/core.rules.ron";
const MAP: &str = "maps/proving_grounds.map.ron";

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Loading,
    InGame,
}

/// The running match.
#[derive(Resource)]
pub struct Match {
    pub sim: Sim,
    /// The player this client controls.
    pub local_player: PlayerId,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .insert_resource(Time::<Fixed>::from_hz(f64::from(TICK_RATE)))
            .add_systems(Startup, load_match_data)
            .add_systems(Update, start_match.run_if(resource_exists::<PendingMatch>))
            .add_systems(FixedUpdate, step.run_if(in_state(AppState::InGame)));
    }
}

/// Data files for the match being loaded.
#[derive(Resource)]
struct PendingMatch {
    rules: Handle<RulesAsset>,
    map: Handle<MapAsset>,
}

fn load_match_data(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(PendingMatch {
        rules: assets.load(RULES),
        map: assets.load(MAP),
    });
}

fn start_match(
    mut commands: Commands,
    pending: Res<PendingMatch>,
    rules: Res<Assets<RulesAsset>>,
    maps: Res<Assets<MapAsset>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let (Some(rules), Some(map)) = (rules.get(&pending.rules), maps.get(&pending.map)) else {
        return;
    };
    commands.remove_resource::<PendingMatch>();
    match Sim::new(rules.0.clone(), &map.0) {
        Ok(sim) => {
            info!(
                "starting `{}` with {} units",
                map.0.name,
                sim.units().count()
            );
            commands.insert_resource(Match {
                sim,
                local_player: PlayerId(0),
            });
            next_state.set(AppState::InGame);
        }
        Err(error) => error!("cannot start `{MAP}`: {error}"),
    }
}

fn step(mut game: ResMut<Match>) {
    game.sim.step();
}
