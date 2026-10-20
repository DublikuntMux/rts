use std::collections::BTreeMap;

use rts_base::{
    Command, IVec2, MapData, PlayerId, PlayerSlot, Rules, Sim, TILE, UnitId, UnitPlacement, tile_of,
};

const PLAYER: PlayerId = PlayerId(0);
const MOVER: UnitId = UnitId(0);

fn rules(speed: &str) -> Rules {
    Rules::from_ron(&format!(
        r##"(tile_capacity: 4, terrain: [
            (id: "grass", symbol: '.', speed_modifier: 1.0, color: "#ffffff"),
            (id: "rock", symbol: '#', speed_modifier: 0.0, color: "#000000"),
        ], units: [
            (id: "worker", size: 1, max_health: 40, radius: 0.75, speed: {speed}),
            (id: "soldier", size: 2, max_health: 80, radius: 0.75, speed: {speed}),
        ])"##
    ))
    .unwrap()
}

fn map(tiles: &[&str], units: &[(&str, IVec2)]) -> MapData {
    MapData {
        name: "occupancy".into(),
        tiles: tiles.iter().map(|row| (*row).into()).collect(),
        players: vec![PlayerSlot { start: units[0].1 }],
        units: units
            .iter()
            .map(|&(kind, pos)| UnitPlacement {
                kind: kind.into(),
                owner: 0,
                pos,
            })
            .collect(),
    }
}

fn sim(tiles: &[&str], units: &[(&str, IVec2)], speed: &str) -> Sim {
    Sim::new(rules(speed), &map(tiles, units)).unwrap()
}

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

fn pos(sim: &Sim, id: UnitId) -> IVec2 {
    sim.unit(id).unwrap().pos
}

const fn move_command(units: Vec<UnitId>, target: IVec2) -> (PlayerId, Command) {
    (PLAYER, Command::Move { units, target })
}

fn assert_capacity(sim: &Sim) {
    let mut usage = BTreeMap::<(i32, i32), u32>::new();
    for (_, unit) in sim.units() {
        assert!(sim.terrain().speed_modifier(unit.pos, sim.rules()) > 0);
        let tile = tile_of(unit.pos);
        *usage.entry((tile.x, tile.y)).or_default() += u32::from(sim.rules().unit(unit.kind).size);
    }
    for (tile, used) in usage {
        assert!(
            used <= u32::from(sim.rules().tile_capacity()),
            "tick {}: tile {tile:?} uses {used} capacity",
            sim.tick()
        );
    }
}

fn step(sim: &mut Sim) {
    sim.step();
    assert_capacity(sim);
}

#[test]
fn exact_capacity_groups_can_overlap_initially_and_after_movement() {
    for kinds in [
        vec!["worker"; 4],
        vec!["soldier"; 2],
        vec!["worker", "worker", "soldier"],
    ] {
        let units: Vec<_> = kinds.iter().map(|&kind| (kind, center(0, 0))).collect();
        let mut sim = sim(&["..."], &units, "20.0");
        assert_eq!(sim.rules().tile_capacity(), 4);
        assert_capacity(&sim);
        let ids: Vec<_> = sim.units().map(|(id, _)| id).collect();
        let target = center(2, 0);
        sim.step_with_commands([move_command(ids.clone(), target)]);
        assert_capacity(&sim);
        step(&mut sim);
        for id in ids {
            assert_eq!(pos(&sim, id), target, "{kinds:?}");
            assert_eq!(sim.move_target(id), None);
        }
    }
}

#[test]
fn initially_over_capacity_groups_are_rejected() {
    for kinds in [
        vec!["worker"; 5],
        vec!["soldier"; 3],
        vec!["worker", "worker", "worker", "soldier"],
    ] {
        let units: Vec<_> = kinds.iter().map(|&kind| (kind, center(0, 0))).collect();
        let error = Sim::new(rules("20.0"), &map(&["."], &units)).unwrap_err();
        assert!(error.to_string().contains("capacity"), "{error}");
    }
}

#[test]
fn capacity_counts_only_the_tile_containing_each_unit_center() {
    let left = IVec2::new(TILE - 1, TILE / 2);
    let right = IVec2::new(TILE, TILE / 2);
    let sim = sim(
        &[".."],
        &[
            ("soldier", left),
            ("soldier", left),
            ("soldier", right),
            ("soldier", right),
        ],
        "20.0",
    );
    assert_capacity(&sim);
    assert_eq!(sim.units().count(), 4);
}

#[test]
fn stable_ids_resolve_contention_even_when_command_ids_are_reversed() {
    let target = center(1, 1);
    let mut sim = sim(
        &["...", "...", "..."],
        &[
            ("soldier", center(0, 1)),
            ("soldier", center(2, 1)),
            ("soldier", center(1, 0)),
        ],
        "20.0",
    );
    sim.step_with_commands([move_command(vec![UnitId(2), UnitId(1), MOVER], target)]);
    assert_capacity(&sim);
    assert_eq!(pos(&sim, MOVER), target);
    assert_eq!(pos(&sim, UnitId(1)), target);
    assert_ne!(tile_of(pos(&sim, UnitId(2))), tile_of(target));
    assert_eq!(sim.move_target(UnitId(2)), Some(target));
}

#[test]
fn converging_units_never_exceed_capacity_at_fractional_or_high_speeds() {
    for speed in ["1.25", "20.0", "200.0"] {
        let mut sim = sim(
            &[".....", ".....", "....."],
            &[
                ("soldier", center(0, 1)),
                ("worker", center(4, 1)),
                ("worker", center(2, 0)),
                ("soldier", center(2, 2)),
            ],
            speed,
        );
        // Distinct sub-tile destinations still compete for the same tile's capacity.
        let targets = [
            center(2, 1) + IVec2::new(-400, 0),
            center(2, 1) + IVec2::new(400, 0),
            center(2, 1) + IVec2::new(0, -400),
            center(2, 1) + IVec2::new(0, 400),
        ];
        let commands: Vec<_> = sim
            .units()
            .zip(targets)
            .map(|((id, _), target)| move_command(vec![id], target))
            .collect();
        sim.step_with_commands(commands);
        assert_capacity(&sim);
        for _ in 0..80 {
            step(&mut sim);
        }
        let used: u32 = sim
            .units()
            .filter(|(_, unit)| tile_of(unit.pos) == IVec2::new(2, 1))
            .map(|(_, unit)| u32::from(sim.rules().unit(unit.kind).size))
            .sum();
        assert_eq!(used, 4, "speed {speed}");
        for (id, unit) in sim.units() {
            if tile_of(unit.pos) != IVec2::new(2, 1) {
                assert_eq!(
                    sim.move_target(id),
                    Some(targets[usize::try_from(id.0).unwrap()])
                );
            }
        }
    }
}

#[test]
fn full_goal_is_approached_without_entering_then_resumes_after_removal() {
    for (start, goal, waiting) in [(0, 3, 2), (3, 0, 1)] {
        for speed in ["20.0", "200.0"] {
            let target = center(goal, 0);
            let mut sim = sim(
                &["...."],
                &[
                    ("worker", center(start, 0)),
                    ("soldier", target),
                    ("soldier", target),
                ],
                speed,
            );
            sim.step_with_commands([move_command(vec![MOVER], target)]);
            assert_capacity(&sim);
            for _ in 0..3 {
                step(&mut sim);
            }
            assert_eq!(pos(&sim, MOVER), center(waiting, 0));
            assert_eq!(sim.move_target(MOVER), Some(target));
            assert!(sim.remove_unit(UnitId(1)).is_some());
            step(&mut sim);
            assert_eq!(pos(&sim, MOVER), target);
            assert_eq!(sim.move_target(MOVER), None);
        }
    }
}

#[test]
fn a_dynamically_blocked_corridor_waits_and_resumes_when_blockers_move() {
    let target = center(4, 0);
    let mut sim = sim(
        &[".....", "##.##"],
        &[
            ("worker", center(0, 0)),
            ("soldier", center(2, 0)),
            ("soldier", center(2, 0)),
        ],
        "20.0",
    );
    sim.step_with_commands([move_command(vec![MOVER], target)]);
    for _ in 0..3 {
        step(&mut sim);
        assert_eq!(pos(&sim, MOVER), center(0, 0));
        assert_eq!(sim.move_target(MOVER), Some(target));
    }
    sim.step_with_commands([move_command(vec![UnitId(1), UnitId(2)], center(2, 1))]);
    assert_capacity(&sim);
    assert_eq!(pos(&sim, MOVER), center(0, 0));
    step(&mut sim);
    assert_eq!(pos(&sim, MOVER), center(1, 0));
    for _ in 0..3 {
        step(&mut sim);
    }
    assert_eq!(pos(&sim, MOVER), target);
    assert_eq!(sim.move_target(MOVER), None);
}

#[test]
fn static_unreachable_replacements_are_ignored_but_full_goals_are_accepted() {
    let full_goal = center(2, 0);
    let mut sim = sim(
        &["...#.", "...#."],
        &[
            ("worker", center(0, 0)),
            ("soldier", full_goal),
            ("soldier", full_goal),
        ],
        "1.0",
    );
    let original = center(1, 1);
    sim.step_with_commands([move_command(vec![MOVER], original)]);
    sim.step_with_commands([move_command(vec![MOVER], center(4, 0))]);
    assert_capacity(&sim);
    assert_eq!(sim.move_target(MOVER), Some(original));
    sim.step_with_commands([move_command(vec![MOVER], full_goal)]);
    assert_capacity(&sim);
    assert_eq!(sim.move_target(MOVER), Some(full_goal));
}

#[test]
fn removing_a_blocker_replaces_the_old_detour_on_the_next_tick() {
    for (start, goal, first) in [(0, 4, 1), (4, 0, 3)] {
        let target = center(goal, 1);
        let mut sim = sim(
            &[".....", ".....", "....."],
            &[
                ("worker", center(start, 1)),
                ("soldier", center(2, 1)),
                ("soldier", center(2, 1)),
            ],
            "20.0",
        );
        sim.step_with_commands([move_command(vec![MOVER], target)]);
        assert_capacity(&sim);
        assert_eq!(pos(&sim, MOVER), center(first, 1));
        sim.remove_unit(UnitId(1)).unwrap();
        sim.remove_unit(UnitId(2)).unwrap();
        step(&mut sim);
        assert_eq!(pos(&sim, MOVER), center(2, 1));
        for _ in 0..2 {
            step(&mut sim);
        }
        assert_eq!(pos(&sim, MOVER), target);
    }
}

#[test]
fn removal_returns_the_unit_clears_its_order_and_ignores_queued_stale_ids() {
    let mut sim = sim(&["...."], &[("worker", center(0, 0))], "1.0");
    let target = center(3, 0);
    sim.step_with_commands([move_command(vec![MOVER], target)]);
    let removed_pos = pos(&sim, MOVER);
    let (_, queued) = move_command(vec![MOVER], target);
    sim.queue_command(PLAYER, queued);
    let removed = sim.remove_unit(MOVER).unwrap();
    assert_eq!(removed.pos, removed_pos);
    assert_eq!(removed.owner, PLAYER);
    assert_eq!(sim.move_target(MOVER), None);
    assert!(sim.remove_unit(MOVER).is_none());
    for _ in 0..3 {
        step(&mut sim);
    }
    assert_eq!(sim.units().count(), 0);
    assert_eq!(sim.move_target(MOVER), None);
}

#[test]
fn slow_fractional_movement_stays_monotonic_when_crossing_left_or_up() {
    for (start, target) in [(center(1, 1), center(0, 1)), (center(1, 1), center(1, 0))] {
        let mut sim = sim(&["..", ".."], &[("worker", start)], "0.0625");
        sim.step_with_commands([move_command(vec![MOVER], target)]);
        assert_capacity(&sim);
        let mut previous = pos(&sim, MOVER);
        for _ in 1..320 {
            step(&mut sim);
            let current = pos(&sim, MOVER);
            assert!(current.cmple(previous).all(), "{previous:?} -> {current:?}");
            assert!(current.cmpge(target).all());
            previous = current;
        }
        assert_eq!(pos(&sim, MOVER), target);
        assert_eq!(sim.move_target(MOVER), None);
    }
}
