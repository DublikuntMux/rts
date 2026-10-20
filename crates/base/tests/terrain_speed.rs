use rts_base::{
    Command, IVec2, MapData, PlayerId, PlayerSlot, Rules, Sim, TILE, UnitId, UnitPlacement, tile_of,
};

const MOVER: UnitId = UnitId(0);
const PLAYER: PlayerId = PlayerId(0);

fn rules(speed: &str) -> Rules {
    Rules::from_ron(&format!(
        r##"(tile_capacity: 4, terrain: [
            (id: "normal", symbol: '.', speed_modifier: 1.0, color: "#ffffff"),
            (id: "slow", symbol: 's', speed_modifier: 0.5, color: "#777777"),
            (id: "fast", symbol: 'f', speed_modifier: 2.0, color: "#00ff00"),
            (id: "blocked", symbol: '#', speed_modifier: 0.0, color: "#000000"),
        ], units: [
            (id: "worker", size: 1, max_health: 40, radius: 0.25, speed: {speed}),
            (id: "soldier", size: 2, max_health: 80, radius: 0.25, speed: {speed}),
        ])"##
    ))
    .unwrap()
}

fn map(tiles: &[&str], start: IVec2) -> MapData {
    MapData {
        name: "terrain speed".into(),
        tiles: tiles.iter().map(|row| (*row).into()).collect(),
        players: vec![PlayerSlot { start }],
        units: vec![UnitPlacement {
            kind: "worker".into(),
            owner: 0,
            pos: start,
        }],
    }
}

fn sim(tiles: &[&str], start: IVec2, speed: &str) -> Sim {
    Sim::new(rules(speed), &map(tiles, start)).unwrap()
}

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

fn pos(sim: &Sim) -> IVec2 {
    sim.unit(MOVER).unwrap().pos
}

fn move_to(sim: &mut Sim, target: IVec2) {
    sim.step_with_commands([(
        PLAYER,
        Command::Move {
            units: vec![MOVER],
            target,
        },
    )]);
}

fn steps(sim: &mut Sim, count: u32) {
    for _ in 0..count {
        sim.step();
    }
}

#[test]
fn terrain_lookup_uses_q10_and_blocks_every_outside_edge() {
    let sim = sim(&[".sf#"], center(0, 0), "1.0");
    for (x, expected) in [(0, 1024), (1, 512), (2, 2048), (3, 0)] {
        assert_eq!(
            sim.terrain().speed_modifier(center(x, 0), sim.rules()),
            expected
        );
    }
    for outside in [
        IVec2::new(-1, 0),
        IVec2::new(0, -1),
        IVec2::new(4 * TILE, 0),
        IVec2::new(0, TILE),
    ] {
        assert_eq!(sim.terrain().speed_modifier(outside, sim.rules()), 0);
    }
    assert_eq!(sim.rules().unit(sim.unit(MOVER).unwrap().kind).speed, TILE);
}

#[test]
fn zero_modifier_rejects_player_starts_and_unit_placements() {
    let mut data = map(&[".#"], center(0, 0));
    data.players[0].start = center(1, 0);
    assert!(Sim::new(rules("1.0"), &data).is_err());
    data.players[0].start = center(0, 0);
    data.units[0].pos = center(1, 0);
    assert!(Sim::new(rules("1.0"), &data).is_err());
}

#[test]
fn blocked_or_unreachable_targets_preserve_the_current_order_at_high_speed() {
    for invalid in [center(2, 0), center(3, 0), IVec2::new(-1, TILE / 2)] {
        let mut sim = sim(&["fs#f"], center(0, 0), "1.0");
        let target = center(1, 0);
        move_to(&mut sim, target);
        let mut expected = sim.clone();
        move_to(&mut sim, invalid);
        expected.step();
        assert_eq!(sim.state_hash(), expected.state_hash());
        assert_eq!(sim.move_target(MOVER), Some(target));
    }
    let mut sim = sim(&["f#f"], center(0, 0), "2097151.0");
    move_to(&mut sim, center(2, 0));
    assert_eq!(pos(&sim), center(0, 0));
    assert_eq!(sim.move_target(MOVER), None);

    // A route around zero terrain must still visit the lower corridor, even at high speed.
    let mut around = self::sim(&["f#f", "fff"], center(0, 0), "20.0");
    move_to(&mut around, center(2, 0));
    assert_eq!(pos(&around), center(1, 1));
    around.step();
    assert_eq!(pos(&around), center(2, 0));
}

#[test]
fn uniform_terrain_has_exact_half_normal_and_double_distance() {
    for (row, distance) in [
        ("ssssssssss", TILE / 2),
        ("..........", TILE),
        ("ffffffffff", 2 * TILE),
    ] {
        let start = center(0, 0);
        let mut sim = sim(&[row], start, "1.0");
        move_to(&mut sim, center(9, 0));
        steps(&mut sim, 19);
        assert_eq!(pos(&sim), start + IVec2::new(distance, 0), "{row}");
        assert_eq!(sim.rules().unit(sim.unit(MOVER).unwrap().kind).speed, TILE);
    }
}

#[test]
fn boundary_time_uses_each_tile_half_in_both_directions_and_axes() {
    for (source, destination, first_distance) in [
        ('s', '.', TILE / 2),
        ('.', 's', 3 * TILE / 4),
        ('f', 's', 7 * TILE / 8),
        ('s', 'f', TILE / 2),
    ] {
        for direction in [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            let vertical = direction.y != 0;
            let reverse = direction.x + direction.y < 0;
            let symbols = if reverse {
                [destination, source]
            } else {
                [source, destination]
            };
            let rows = if vertical {
                vec![symbols[0].to_string(), symbols[1].to_string()]
            } else {
                vec![symbols.iter().collect()]
            };
            let rows: Vec<_> = rows.iter().map(String::as_str).collect();
            let start = center(
                i32::from(reverse && !vertical),
                i32::from(reverse && vertical),
            );
            let target = start + direction * TILE;
            let mut sim = sim(&rows, start, "20.0");
            move_to(&mut sim, target);
            assert_eq!(
                pos(&sim),
                start + direction * first_distance,
                "{source}->{destination}, {direction:?}"
            );
            assert_eq!(sim.move_target(MOVER), Some(target));
            sim.step();
            assert_eq!(pos(&sim), target, "{source}->{destination}, {direction:?}");
            assert_eq!(sim.move_target(MOVER), None);
        }
    }
}

#[test]
fn one_tick_shares_its_time_across_multiple_mixed_speed_segments() {
    for (row, start, target, first) in [
        (
            "sf.sff",
            center(0, 0),
            center(5, 0),
            IVec2::new(15 * TILE / 4, TILE / 2),
        ),
        (
            "ffs.fs",
            center(5, 0),
            center(0, 0),
            IVec2::new(9 * TILE / 4, TILE / 2),
        ),
    ] {
        let mut sim = sim(&[row], start, "80.0");
        // The first tick supplies 4 tile-seconds at normal speed: slow half (1),
        // fast tile (0.5), normal tile (1), then 0.75 of the next slow tile (1.5).
        move_to(&mut sim, target);
        assert_eq!(pos(&sim), first, "{row}");
        assert_eq!(sim.move_target(MOVER), Some(target));
        sim.step();
        assert_eq!(pos(&sim), target, "{row}");
        assert_eq!(sim.move_target(MOVER), None);
    }
}

#[test]
fn slow_fractional_speed_keeps_sub_world_unit_progress_without_drift() {
    for (row, expected) in [("ss", 512), ("..", 1024), ("ff", 2048)] {
        let start = center(0, 0);
        let mut sim = sim(&[row, row, row], start, "0.125");
        let target = center(0, 2);
        move_to(&mut sim, target);
        steps(&mut sim, 159);
        assert_eq!(pos(&sim), start + IVec2::new(0, expected), "{row}");
    }
    let start = center(0, 0);
    let mut tiny = sim(&["s"], start, "0.0009765625");
    let target = start + IVec2::new(-16, 0);
    move_to(&mut tiny, target);
    steps(&mut tiny, 38);
    assert_eq!(pos(&tiny), start);
    tiny.step();
    assert_eq!(pos(&tiny), start + IVec2::NEG_X);
    steps(&mut tiny, 360);
    assert_eq!(pos(&tiny), start + IVec2::new(-10, 0));
    assert_eq!(tiny.move_target(MOVER), Some(target));
}

#[test]
fn a_longer_fast_road_beats_the_direct_slow_route() {
    let start = center(0, 1);
    let target = center(4, 1);
    let mut sim = sim(&["fffff", "sssss"], start, "20.0");
    // Direct: 4 / 0.5 = 8 tile-seconds. Road: two slow halves (2)
    // plus five fast tiles (2.5), so the six-edge road arrives on tick 5.
    move_to(&mut sim, target);
    assert_eq!(pos(&sim), start + IVec2::new(0, -TILE / 2));
    steps(&mut sim, 3);
    assert_ne!(pos(&sim), target);
    sim.step();
    assert_eq!(pos(&sim), target);
    assert_eq!(sim.move_target(MOVER), None);
}

#[test]
fn a_normal_shortcut_beats_a_fast_road_with_too_large_a_detour() {
    let mut sim = sim(&["fffff", "f###f", "....."], center(0, 2), "20.0");
    let target = center(4, 2);
    // Normal shortcut takes 4 tile-seconds; the eight-edge road takes 4.5.
    move_to(&mut sim, target);
    assert_eq!(pos(&sim), center(1, 2));
    steps(&mut sim, 3);
    assert_eq!(pos(&sim), target);
    assert_eq!(sim.move_target(MOVER), None);
}

#[test]
fn removing_a_capacity_blocker_reopens_the_faster_route_next_tick() {
    let start = center(0, 1);
    let target = center(4, 1);
    let mut data = map(&["fffff", "sssss"], start);
    for _ in 0..2 {
        data.units.push(UnitPlacement {
            kind: "soldier".into(),
            owner: 0,
            pos: center(2, 0),
        });
    }
    let mut sim = Sim::new(rules("20.0"), &data).unwrap();
    move_to(&mut sim, target);
    assert_eq!(pos(&sim), start + IVec2::new(TILE / 2, 0));
    sim.remove_unit(UnitId(1)).unwrap();
    assert_eq!(
        sim.units().map(|(id, _)| id).collect::<Vec<_>>(),
        [MOVER, UnitId(2)]
    );
    // Keep the partial segment, then take the freshly available road from its endpoint.
    sim.step();
    assert_eq!(pos(&sim), center(1, 1));
    sim.step();
    assert_eq!(pos(&sim), IVec2::new(3 * TILE / 2, TILE));
    for _ in 0..8 {
        sim.step();
        let mut used = std::collections::BTreeMap::new();
        for (_, unit) in sim.units() {
            let tile = tile_of(unit.pos);
            *used.entry((tile.x, tile.y)).or_insert(0_u32) +=
                u32::from(sim.rules().unit(unit.kind).size);
        }
        assert!(used.values().all(|&size| size <= 4));
    }
    assert_eq!(pos(&sim), target);
    assert_eq!(sim.move_target(MOVER), None);
}
