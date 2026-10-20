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
            (id: "grass", symbol: '.', walkable: true, color: "#ffffff"),
            (id: "rock", symbol: '#', walkable: false, color: "#000000"),
        ], units: [
            (id: "worker", size: 1, max_health: 40, radius: 0.75, speed: 20.0),
            (id: "soldier", size: 2, max_health: 80, radius: 0.75, speed: 20.0),
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
