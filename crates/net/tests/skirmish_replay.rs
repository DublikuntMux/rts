use rts_base::{Command, IVec2, MatchOutcome, PlayerId, TILE, UnitId};
use rts_net::{MatchSetup, PlayerInput, Replay, Turn};

#[test]
fn terminal_skirmish_turns_advance_in_order_and_replay_with_frozen_gameplay() {
    let setup = MatchSetup {
        rules: r#"(
            tile_capacity: 4,
            terrain: [(id: "grass", symbol: '.', speed_modifier: 1, color: "green")],
            units: [
                (id: "headquarters", sprite:"hq.png", size: 4, max_health: 10, radius: 0.5,
                 speed: 0, building: true),
                (id: "barracks", sprite:"barracks.png", size: 4, max_health: 10, radius: 0.5,
                 speed: 0, building: true),
                (id: "worker", sprite:"worker.png", size: 1, max_health: 10, radius: 0.25, speed: 2),
                (id: "soldier", sprite:"soldier.png", size: 2, max_health: 10, radius: 0.25, speed: 2,
                 weapon: Some((range: 2, damage: 10, cooldown: 1))),
            ],
            economy: Some((
                initial: (wood: 10, gold: 10), worker_cost: (wood: 1, gold: 1),
                barracks_cost: (wood: 1, gold: 1), rifle_cost: (wood: 1, gold: 1),
                worker_ticks: 1, barracks_ticks: 1, rifle_ticks: 1, harvest_ticks: 1, carry_capacity: 1,
            )),
        )"#
        .to_owned(),
        map: r#"(
            name: "HQ Finale",
            players: [(start: (0.5, 0.5)),(start: (3.5, 0.5))], tiles: ["....", "...."],
            units: [
                (kind:"headquarters",owner:0,pos:(0.5,0.5)),
                (kind:"headquarters",owner:1,pos:(3.5,0.5)),
                (kind:"soldier",owner:0,pos:(2.5,0.5)),
            ],
            skirmish: Some((deposits: [
                (kind:Wood,pos:(1.5,1.5),remaining:5), (kind:Gold,pos:(2.5,1.5),remaining:5),
            ])),
        )"#
        .to_owned(),
        players: 2,
        seed: 0,
    };
    let mut replay = Replay::new(setup).unwrap();
    let mut sim = replay.simulation().unwrap();
    let winner = PlayerId(0);
    let hq = sim.headquarters(winner).unwrap();
    let initial = sim.resources(winner).unwrap();
    for tick in 1..=3 {
        let turn = Turn {
            tick,
            inputs: vec![
                PlayerInput {
                    player: winner,
                    commands: if tick == 1 {
                        vec![Command::Attack {
                            units: vec![UnitId(2)],
                            target: UnitId(1),
                        }]
                    } else {
                        vec![
                            Command::TrainWorker { units: vec![hq] },
                            Command::Move {
                                units: vec![UnitId(2)],
                                target: IVec2::splat(TILE / 2),
                            },
                        ]
                    },
                },
                PlayerInput {
                    player: PlayerId(1),
                    commands: vec![],
                },
            ],
        };
        let previous_hash = sim.state_hash();
        turn.apply(&mut sim).unwrap();
        assert_eq!(sim.tick(), tick);
        assert_eq!(sim.outcome(), Some(MatchOutcome::Winner(winner)));
        assert_eq!(sim.resources(winner), Some(initial));
        assert_eq!(sim.units().count(), 2);
        assert_eq!(sim.move_target(UnitId(2)), None);
        assert_ne!(sim.state_hash(), previous_hash);
        replay.push(turn, sim.state_hash()).unwrap();
    }

    let (_, verified) = Replay::from_ron_with_sim(&replay.to_ron().unwrap()).unwrap();
    assert_eq!(verified.state_hash(), sim.state_hash());
    assert_eq!(verified.outcome(), sim.outcome());
    assert_eq!(verified.tick(), sim.tick());
}

#[test]
fn restarting_rejects_unsupported_replay_versions() {
    let setup = MatchSetup {
        rules: include_str!("../../../assets/data/core.rules.ron").into(),
        map: include_str!("../../../assets/maps/frontier.map.ron").into(),
        seed: 0,
        players: 2,
    };
    let mut replay = Replay::new(setup).unwrap();
    for version in [0, 7, u32::MAX] {
        replay.version = version;
        assert!(replay.restarted().is_err());
    }
}
