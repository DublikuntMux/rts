//! Match lifecycle: load data, start the simulation, and run it at the fixed tick rate.

use std::collections::BTreeMap;

use bevy::prelude::*;
use rts_base::{Command, IVec2 as SimPos, PlayerId, Sim, TICK_RATE, UnitId};
use rts_net::{MAX_COMMANDS_PER_TURN, MatchSetup};

use rts_assets::{MapAsset, RulesAsset};

const RULES: &str = "data/core.rules.ron";
const MAP: &str = "maps/proving_grounds.map.ron";

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
    let setup = MatchSetup {
        rules: rules.1.clone(),
        map: map.1.clone(),
        seed: 0,
        players: u8::try_from(map.0.players.len()).unwrap_or(0),
    };
    match setup.simulation() {
        Ok(_) => {
            commands.insert_resource(MatchData(setup));
            next_state.set(AppState::Menu);
        }
        Err(error) => error!("cannot start `{MAP}`: {error}"),
    }
}
