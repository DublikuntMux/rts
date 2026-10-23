use std::collections::BTreeMap;

use rts_base::{Command, IVec2, PlayerId, Sim, TILE, UnitId, tile_of};
use rts_net::{MatchSetup, PlayerInput, Replay, Turn};

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

fn assert_capacity(sim: &Sim) {
    let mut usage = BTreeMap::<(i32, i32), u32>::new();
    for (_, unit) in sim.units() {
        let tile = tile_of(unit.pos);
        *usage.entry((tile.x, tile.y)).or_default() += u32::from(sim.rules().unit(unit.kind).size);
    }
    assert!(
        usage
            .values()
            .all(|&used| used <= u32::from(sim.rules().tile_capacity()))
    );
}

#[test]
fn replay_verifies_convergence_and_blockers_moved_by_commands() {
    let setup = MatchSetup {
        rules: r##"(tile_capacity: 4, terrain: [
            (id: "grass", symbol: '.', speed_modifier: 1.0, color: "#ffffff"),
            (id: "rock", symbol: '#', speed_modifier: 0.0, color: "#000000"),
        ], units: [
            (id: "worker", sprite: "textures/units/worker.png", size: 1, max_health: 40, radius: 0.75, speed: 20.0),
            (id: "soldier", sprite: "textures/units/soldier.png", size: 2, max_health: 80, radius: 0.75, speed: 20.0),
        ])"##
            .into(),
        map: r###"(name: "occupancy replay", players: [(start: (0.5, 0.5))],
            tiles: [".....", "##.##"], units: [
                (kind: "worker", owner: 0, pos: (0.5, 0.5)),
                (kind: "worker", owner: 0, pos: (0.5, 0.5)),
                (kind: "worker", owner: 0, pos: (4.5, 0.5)),
                (kind: "worker", owner: 0, pos: (4.5, 0.5)),
                (kind: "soldier", owner: 0, pos: (2.5, 0.5)),
                (kind: "soldier", owner: 0, pos: (2.5, 0.5)),
            ])"###
            .into(),
        seed: 17,
        players: 1,
    };
    let mut sim = setup.simulation().unwrap();
    let mut replay = Replay::new(setup).unwrap();
    let goal = center(2, 0);
    for tick in 1..=8 {
        let commands = match tick {
            1 => vec![Command::Move {
                units: (0..4).map(UnitId).collect(),
                target: goal,
            }],
            3 => vec![Command::Move {
                units: vec![UnitId(4), UnitId(5)],
                target: center(2, 1),
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
        assert_capacity(&sim);
        if tick == 2 {
            for id in (0..4).map(UnitId) {
                assert_ne!(tile_of(sim.unit(id).unwrap().pos), tile_of(goal));
                assert_eq!(sim.move_target(id), Some(goal));
            }
        }
        replay.push(turn, sim.state_hash()).unwrap();
    }
    for id in (0..4).map(UnitId) {
        assert_eq!(sim.unit(id).unwrap().pos, goal);
        assert_eq!(sim.move_target(id), None);
    }
    for id in [UnitId(4), UnitId(5)] {
        assert_eq!(sim.unit(id).unwrap().pos, center(2, 1));
    }
    let loaded = Replay::from_ron(&replay.to_ron().unwrap()).unwrap();
    let playback = loaded.playback().unwrap();
    assert_eq!(loaded.final_hash, sim.state_hash());
    assert_eq!(playback.state_hash(), sim.state_hash());
    assert_capacity(&playback);
}

#[test]
fn mixed_speed_group_replay_reproduces_every_hash_through_membership_changes() {
    let setup = MatchSetup {
        rules: r##"(tile_capacity: 4, terrain: [
            (id: "grass", symbol: '.', speed_modifier: 1.0, color: "#ffffff"),
        ], units: [
            (id: "worker", sprite: "worker.png", size: 1, max_health: 40, radius: 0.3, speed: 2.0),
            (id: "soldier", sprite: "soldier.png", size: 2, max_health: 80, radius: 0.4, speed: 1.0),
        ])"##.into(),
        map: r#"(name: "group replay", players: [(start: (0.5, 0.5))],
            tiles: ["........"], units: [
                (kind: "worker", owner: 0, pos: (0.5, 0.5)),
                (kind: "soldier", owner: 0, pos: (0.5, 0.5)),
            ])"#.into(),
        seed: 17,
        players: 1,
    };
    let mut sim = setup.simulation().unwrap();
    let mut replay = Replay::new(setup).unwrap();
    let mut hashes = Vec::new();
    for tick in 1..=180 {
        let commands = match tick {
            1 | 24 | 40 => vec![Command::Move {
                units: vec![UnitId(0), UnitId(1)],
                target: center(5, 0),
            }],
            16 => vec![Command::Stop {
                units: vec![UnitId(1)],
            }],
            20 => vec![Command::Move {
                units: vec![UnitId(1)],
                target: center(7, 0),
            }],
            32 => vec![Command::Move {
                units: vec![UnitId(1)],
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
        assert_capacity(&sim);
        if tick < 16 {
            assert_eq!(
                sim.unit(UnitId(0)).unwrap().pos,
                sim.unit(UnitId(1)).unwrap().pos
            );
        }
        let hash = sim.state_hash();
        hashes.push(hash);
        replay.push(turn, hash).unwrap();
    }
    for id in [UnitId(0), UnitId(1)] {
        assert_eq!(sim.unit(id).unwrap().pos, center(5, 0));
        assert_eq!(sim.move_target(id), None);
    }
    let loaded = Replay::from_ron(&replay.to_ron().unwrap()).unwrap();
    let mut playback = loaded.setup.simulation().unwrap();
    for (turn, hash) in loaded.turns.iter().zip(hashes) {
        turn.apply(&mut playback).unwrap();
        assert_eq!(playback.state_hash(), hash, "tick {}", turn.tick);
    }
    assert_eq!(loaded.playback().unwrap().state_hash(), sim.state_hash());
}
