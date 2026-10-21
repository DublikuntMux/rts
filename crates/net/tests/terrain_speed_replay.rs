use rts_base::{Command, IVec2, PlayerId, TILE, UnitId};
use rts_net::{MatchSetup, PlayerInput, Replay, Turn};

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

#[test]
fn mixed_terrain_replay_roundtrips_commands_and_fractional_movement() {
    let setup = MatchSetup {
        rules: r##"(tile_capacity: 4, terrain: [
            (id: "normal", symbol: '.', speed_modifier: 1.0, color: "#ffffff"),
            (id: "slow", symbol: 's', speed_modifier: 0.5, color: "#777777"),
            (id: "fast", symbol: 'f', speed_modifier: 2.0, color: "#00ff00"),
            (id: "blocked", symbol: '#', speed_modifier: 0.0, color: "#000000"),
        ], units: [(id: "worker", sprite: "textures/units/worker.png", size: 1, max_health: 40, radius: 0.25, speed: 1.125)])"##
            .into(),
        map: r#"(name: "terrain replay", players: [(start: (0.5, 1.5))],
            tiles: ["fffff", "s.#.s", "..s.."], units: [
                (kind: "worker", owner: 0, pos: (0.5, 1.5)),
                (kind: "worker", owner: 0, pos: (4.5, 1.5)),
            ])"#
        .into(),
        seed: 23,
        players: 1,
    };
    let mut sim = setup.simulation().unwrap();
    let mut replay = Replay::new(setup).unwrap();
    let mut hashes = Vec::new();
    for tick in 1..=160 {
        let commands = match tick {
            1 => vec![
                Command::Move {
                    units: vec![UnitId(0)],
                    target: center(4, 1),
                },
                Command::Move {
                    units: vec![UnitId(1)],
                    target: center(0, 1),
                },
            ],
            17 => vec![Command::Stop {
                units: vec![UnitId(1)],
            }],
            23 => vec![Command::Move {
                units: vec![UnitId(1)],
                target: center(1, 2),
            }],
            31 => vec![Command::Move {
                units: vec![UnitId(0)],
                target: center(4, 2),
            }],
            // Leave fractional movement active in the final hash.
            151 => vec![Command::Move {
                units: vec![UnitId(0)],
                target: center(0, 0),
            }],
            _ => vec![],
        };
        let turn = Turn {
            tick,
            inputs: vec![PlayerInput {
                player: PlayerId(0),
                commands,
            }],
        };
        turn.apply(&mut sim).unwrap();
        assert!(
            sim.units()
                .all(|(_, unit)| sim.terrain().speed_modifier(unit.pos, sim.rules()) > 0)
        );
        hashes.push(sim.state_hash());
        replay.push(turn, sim.state_hash()).unwrap();
    }
    assert_eq!(sim.unit(UnitId(1)).unwrap().pos, center(1, 2));
    assert_eq!(sim.move_target(UnitId(0)), Some(center(0, 0)));
    let loaded = Replay::from_ron(&replay.to_ron().unwrap()).unwrap();
    assert_eq!(loaded, replay);
    let mut reconstructed = loaded.setup.simulation().unwrap();
    for (turn, hash) in loaded.turns.iter().zip(hashes) {
        turn.apply(&mut reconstructed).unwrap();
        assert_eq!(reconstructed.state_hash(), hash, "tick {}", turn.tick);
    }
    assert_eq!(loaded.final_hash, sim.state_hash());
    assert_eq!(loaded.playback().unwrap().state_hash(), sim.state_hash());
}
