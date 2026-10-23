use rts_base::{Command, IVec2, MapData, PlayerId, Rules, Sim, TILE, UnitId};

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

/// Pin the existing hash stream while exercising route ties, terrain costs, live occupancy,
/// group membership, replacement orders, and fractional movement. A replay produced by the
/// unchanged simulation must continue to verify after implementation-only optimizations.
#[test]
fn movement_hash_stream_matches_v6_baseline() {
    let rules = Rules::from_ron(
        r##"(tile_capacity: 4, terrain: [
            (id: "normal", symbol: '.', speed_modifier: 1, color: "#ffffff"),
            (id: "fast", symbol: 'f', speed_modifier: 2, color: "#ffffff"),
            (id: "slow", symbol: 's', speed_modifier: 0.5, color: "#ffffff"),
            (id: "blocked", symbol: '#', speed_modifier: 0, color: "#000000"),
        ], units: [
            (id: "worker", sprite: "worker.png", size: 1, max_health: 40, radius: 0.25, speed: 1.125),
            (id: "soldier", sprite: "soldier.png", size: 2, max_health: 80, radius: 0.25, speed: 0.75),
        ])"##,
    )
    .unwrap();
    let map = MapData::from_ron(
        r#"(name: "hash compatibility", players: [(start: (0.5, 1.5))],
            tiles: ["ffffff", "..s.s.", "......"], units: [
                (kind: "worker", owner: 0, pos: (0.5, 1.5)),
                (kind: "soldier", owner: 0, pos: (0.5, 1.5)),
                (kind: "soldier", owner: 0, pos: (2.5, 1.5)),
                (kind: "soldier", owner: 0, pos: (2.5, 1.5)),
            ])"#,
    )
    .unwrap();
    let mut sim = Sim::new(rules, &map).unwrap();
    let mut fingerprint = 0xcbf2_9ce4_8422_2325_u64;
    for tick in 0..=180 {
        match tick {
            0 | 24 => sim.queue_command(
                PlayerId(0),
                Command::Move {
                    units: vec![UnitId(0), UnitId(1)],
                    target: center(5, 1),
                },
            ),
            9 => {
                sim.remove_unit(UnitId(2)).unwrap();
            }
            16 => sim.queue_command(
                PlayerId(0),
                Command::Stop {
                    units: vec![UnitId(1)],
                },
            ),
            40 | 160 => sim.queue_command(
                PlayerId(0),
                Command::Move {
                    units: vec![UnitId(0)],
                    target: center(1, 2) + IVec2::new(37, -91),
                },
            ),
            _ => {}
        }
        for byte in sim.state_hash().to_le_bytes() {
            fingerprint = (fingerprint ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        if tick < 180 {
            sim.step();
        }
    }
    assert_eq!(fingerprint, 18_304_470_849_813_970_820);
}
