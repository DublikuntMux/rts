use rts_base::{Command, IVec2, PlayerId, Resources, TILE, UnitId};
use rts_net::{MatchSetup, PlayerInput, Replay, Turn};

const RULES: &str = include_str!("../../../assets/data/core.rules.ron");
const MAP: &str = r#"(
    name: "Scout run",
    players: [(start: (0.5, 0.5)), (start: (11.5, 0.5))],
    tiles: ["............", "............", "............"],
    units: [
        (kind: "headquarters", owner: 0, pos: (0.5, 0.5)),
        (kind: "headquarters", owner: 1, pos: (11.5, 0.5)),
        (kind: "barracks", owner: 0, pos: (1.5, 1.5)),
    ],
    skirmish: Some((deposits: [
        (kind: Wood, pos: (5.5, 2.5), remaining: 50), (kind: Gold, pos: (6.5, 2.5), remaining: 50),
    ])),
)"#;

fn record(rules: &str) -> (Replay, rts_base::Sim) {
    let setup = MatchSetup {
        rules: rules.to_owned(),
        map: MAP.to_owned(),
        players: 2,
        seed: 0,
    };
    let mut replay = Replay::new(setup.clone()).unwrap();
    let mut sim = setup.simulation().unwrap();
    let barracks = UnitId(2);
    for tick in 1..=200 {
        let commands = match tick {
            1 => vec![Command::TrainScout {
                units: vec![barracks],
            }],
            100 => vec![Command::Move {
                units: vec![UnitId(3)],
                target: IVec2::new(9 * TILE + TILE / 2, 2 * TILE + TILE / 2),
            }],
            _ => vec![],
        };
        let turn = Turn {
            tick,
            inputs: vec![
                PlayerInput {
                    player: PlayerId(0),
                    commands,
                },
                PlayerInput {
                    player: PlayerId(1),
                    commands: vec![],
                },
            ],
        };
        turn.apply(&mut sim).unwrap();
        replay.push(turn, sim.state_hash()).unwrap();
    }
    (replay, sim)
}

#[test]
fn bundled_scouts_train_scout_ahead_and_replay() {
    let (replay, sim) = record(RULES);
    let scout = sim.unit(UnitId(3)).expect("the barracks trained a scout");
    let definition = sim.rules().unit(scout.kind);
    assert_eq!(definition.id, "scout");
    assert_eq!(definition.sight, 8);
    assert_eq!(
        scout.pos,
        IVec2::new(9 * TILE + TILE / 2, 2 * TILE + TILE / 2)
    );
    assert_eq!(
        sim.resources(PlayerId(0)),
        Some(Resources {
            wood: 110 - 35,
            gold: 85 - 25
        })
    );
    let (_, verified) = Replay::from_ron_with_sim(&replay.to_ron().unwrap()).unwrap();
    assert_eq!(verified.state_hash(), sim.state_hash());
}

#[test]
fn rules_without_scouts_replay_scout_orders_as_no_ops() {
    let rules = RULES.replace("scout: Some((cost: (wood: 35, gold: 25), ticks: 70)),", "");
    assert_ne!(rules, RULES);
    let (replay, sim) = record(&rules);
    assert!(sim.unit(UnitId(3)).is_none());
    assert_eq!(
        sim.resources(PlayerId(0)),
        Some(Resources {
            wood: 110,
            gold: 85
        })
    );
    Replay::from_ron(&replay.to_ron().unwrap()).unwrap();
}
