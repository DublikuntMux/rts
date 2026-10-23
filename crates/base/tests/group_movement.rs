use std::collections::BTreeMap;

use rts_base::{
    Command, IVec2, MapData, PlayerId, PlayerSlot, Rules, Sim, TILE, UnitId, UnitPlacement, tile_of,
};

const PLAYER: PlayerId = PlayerId(0);

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

fn rules(worker_speed: &str) -> Rules {
    Rules::from_ron(&format!(
        r##"(tile_capacity: 4, terrain: [
            (id: "grass", symbol: '.', speed_modifier: 1.0, color: "#ffffff"),
            (id: "rock", symbol: '#', speed_modifier: 0.0, color: "#000000"),
        ], units: [
            (id: "worker", sprite: "worker.png", size: 1, max_health: 40, radius: 0.3, speed: {worker_speed}),
            (id: "soldier", sprite: "soldier.png", size: 2, max_health: 80, radius: 0.4, speed: 1.0),
        ])"##
    ))
    .unwrap()
}

fn sim(tiles: &[&str], units: &[(&str, IVec2)], worker_speed: &str) -> Sim {
    Sim::new(
        rules(worker_speed),
        &MapData {
            name: "groups".into(),
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
        },
    )
    .unwrap()
}

fn move_units(sim: &mut Sim, units: Vec<UnitId>, target: IVec2) {
    sim.step_with_commands([(PLAYER, Command::Move { units, target })]);
    assert_capacity(sim);
}

fn pos(sim: &Sim, id: u32) -> IVec2 {
    sim.unit(UnitId(id)).unwrap().pos
}

fn assert_capacity(sim: &Sim) {
    let mut usage = BTreeMap::<(i32, i32), u32>::new();
    for (_, unit) in sim.units() {
        assert!(sim.terrain().speed_modifier(unit.pos, sim.rules()) > 0);
        let tile = tile_of(unit.pos);
        *usage.entry((tile.x, tile.y)).or_default() += u32::from(sim.rules().unit(unit.kind).size);
    }
    assert!(usage.values().all(|&used| used <= 4));
}

#[test]
fn crowd_queues_on_its_route_instead_of_splitting_into_open_side_lanes() {
    for tiles in [
        vec!["......", "......", "......"],
        vec!["######", "......", "######"],
    ] {
        let units: Vec<_> = (0..8)
            .map(|id| ("worker", center(i32::from(id >= 4), 1)))
            .collect();
        let mut sim = sim(&tiles, &units, "20.0");
        let target = center(5, 1);
        move_units(&mut sim, (0..8).map(UnitId).collect(), target);
        for id in 0..4 {
            assert_eq!(pos(&sim, id), center(0, 1));
        }
        for _ in 0..12 {
            sim.step();
            assert_capacity(&sim);
            for (_, unit) in sim.units() {
                assert_eq!(unit.pos.y, target.y);
            }
        }
        // The destination only holds half the crowd; the rest wait immediately behind it.
        let mut counts = BTreeMap::new();
        for (_, unit) in sim.units() {
            *counts.entry(unit.pos.x).or_insert(0) += 1;
        }
        assert_eq!(counts, BTreeMap::from([(center(4, 1).x, 4), (target.x, 4)]));
    }
}

#[test]
fn mixed_speed_selection_travels_together_around_static_obstacles() {
    let mut sim = sim(
        &[".....", ".###.", "....."],
        &[("worker", center(0, 1)), ("soldier", center(0, 1))],
        "2.0",
    );
    let target = center(4, 1);
    move_units(&mut sim, vec![UnitId(1), UnitId(0)], target);
    for _ in 0..160 {
        assert_eq!(pos(&sim, 0), pos(&sim, 1));
        assert_capacity(&sim);
        sim.step();
    }
    assert_eq!(pos(&sim, 0), target);
    assert_eq!(sim.move_target(UnitId(0)), None);
    assert_eq!(sim.move_target(UnitId(1)), None);
}

#[test]
fn independent_selections_with_the_same_target_keep_their_own_speed() {
    let mut sim = sim(
        &["......"],
        &[("worker", center(0, 0)), ("soldier", center(0, 0))],
        "2.0",
    );
    sim.step_with_commands([0, 1].map(|id| {
        (
            PLAYER,
            Command::Move {
                units: vec![UnitId(id)],
                target: center(5, 0),
            },
        )
    }));
    assert_eq!(pos(&sim, 0).x - center(0, 0).x, TILE / 10);
    assert_eq!(pos(&sim, 1).x - center(0, 0).x, TILE / 20);
}

#[test]
fn unrelated_stationary_units_still_make_a_group_route_around_them() {
    let mut sim = sim(
        &[".....", ".....", "....."],
        &[
            ("worker", center(0, 1)),
            ("worker", center(0, 1)),
            ("soldier", center(2, 1)),
            ("soldier", center(2, 1)),
        ],
        "20.0",
    );
    let target = center(4, 1);
    move_units(&mut sim, vec![UnitId(0), UnitId(1)], target);
    sim.step();
    for id in 0..2 {
        assert_eq!(pos(&sim, id), center(1, 0));
    }
    for _ in 0..6 {
        sim.step();
        assert_capacity(&sim);
    }
    for id in 0..2 {
        assert_eq!(pos(&sim, id), target);
    }
}

#[test]
fn separate_multi_unit_commands_on_one_tick_have_independent_paces() {
    let mut sim = sim(
        &["......", "......"],
        &[
            ("worker", center(0, 0)),
            ("worker", center(0, 0)),
            ("worker", center(0, 1)),
            ("soldier", center(0, 1)),
        ],
        "2.0",
    );
    sim.step_with_commands(
        [vec![UnitId(0), UnitId(1)], vec![UnitId(2), UnitId(3)]].map(|units| {
            (
                PLAYER,
                Command::Move {
                    units,
                    target: center(5, 0),
                },
            )
        }),
    );
    assert_eq!(pos(&sim, 0), pos(&sim, 1));
    assert_eq!(pos(&sim, 2), pos(&sim, 3));
    assert_eq!(pos(&sim, 0).x - center(0, 0).x, TILE / 10);
    assert_eq!(center(0, 1).y - pos(&sim, 2).y, TILE / 20);
    assert_capacity(&sim);
}

#[test]
fn arriving_slow_member_releases_the_group_pace_on_the_next_tick() {
    let mut sim = sim(
        &["......"],
        &[("worker", center(0, 0)), ("soldier", center(2, 0))],
        "2.0",
    );
    move_units(&mut sim, vec![UnitId(0), UnitId(1)], center(2, 0));
    assert_eq!(sim.move_target(UnitId(1)), None);
    let before = pos(&sim, 0);
    sim.step();
    assert_eq!(pos(&sim, 0).x - before.x, TILE / 10);
    assert_capacity(&sim);
}

#[test]
fn rejected_slow_members_neither_join_nor_slow_the_accepted_group() {
    let mut sim = Sim::new(
        rules("2.0"),
        &MapData {
            name: "rejected group members".into(),
            tiles: vec!["..#..".into()],
            players: vec![
                PlayerSlot {
                    start: center(0, 0)
                };
                2
            ],
            units: [
                ("worker", 0, center(0, 0)),
                ("worker", 0, center(0, 0)),
                ("soldier", 1, center(0, 0)),
                ("soldier", 0, center(3, 0)),
            ]
            .into_iter()
            .map(|(kind, owner, pos)| UnitPlacement {
                kind: kind.into(),
                owner,
                pos,
            })
            .collect(),
        },
    )
    .unwrap();
    let mut expected = sim.clone();
    let target = center(1, 0);
    move_units(
        &mut sim,
        vec![UnitId(0), UnitId(1), UnitId(2), UnitId(3), UnitId(99)],
        target,
    );
    move_units(&mut expected, vec![UnitId(0), UnitId(1)], target);
    assert_eq!(sim.state_hash(), expected.state_hash());
    assert_eq!(pos(&sim, 0).x - center(0, 0).x, TILE / 10);
    assert_eq!(sim.move_target(UnitId(2)), None);
    assert_eq!(sim.move_target(UnitId(3)), None);
}

#[test]
fn shared_tile_discounts_group_members_but_keeps_outsiders_capacity() {
    let mut sim = sim(
        &[".....", ".....", "....."],
        &[
            ("soldier", center(0, 1)),
            ("worker", center(1, 1)),
            ("soldier", center(1, 1)),
            ("worker", center(1, 1)),
        ],
        "20.0",
    );
    let target = center(4, 1);
    move_units(&mut sim, vec![UnitId(0), UnitId(1)], target);
    // Even after the group's worker leaves, three outsider capacity units remain.
    // The size-two soldier must detour, while its size-one group mate can move on.
    assert_eq!(pos(&sim, 0).x, center(0, 1).x);
    assert!(pos(&sim, 0).y < center(0, 1).y);
    assert!(pos(&sim, 1).x > center(1, 1).x);
    for _ in 0..160 {
        sim.step();
        assert_capacity(&sim);
    }
    for id in 0..2 {
        assert_eq!(pos(&sim, id), target);
    }
}

#[test]
fn stopping_replacing_or_removing_the_slow_member_releases_the_group_pace() {
    for action in 0..3 {
        let mut sim = sim(
            &["......", "......"],
            &[("worker", center(0, 0)), ("soldier", center(0, 0))],
            "2.0",
        );
        move_units(&mut sim, vec![UnitId(0), UnitId(1)], center(5, 0));
        let before = pos(&sim, 0);
        match action {
            0 => sim.step_with_commands([(
                PLAYER,
                Command::Stop {
                    units: vec![UnitId(1)],
                },
            )]),
            1 => move_units(&mut sim, vec![UnitId(1)], center(0, 1)),
            _ => {
                sim.remove_unit(UnitId(1)).unwrap();
                sim.step();
            }
        }
        // Fractional effort from the first tick is retained when the speed cap changes.
        assert_eq!(pos(&sim, 0).x - before.x, TILE / 10);
        assert_capacity(&sim);
    }
}

#[test]
fn waiting_for_group_capacity_never_rewinds_a_partly_travelled_segment() {
    let mut units = vec![("worker", center(0, 0) - IVec2::new(100, 0))];
    units.extend([("worker", center(1, 1)); 4]);
    let mut sim = sim(&[".....", "....."], &units, "1.0");
    let target = center(4, 0);
    move_units(&mut sim, (0..5).map(UnitId).collect(), target);
    let mut previous = pos(&sim, 0);
    let mut waited = false;
    for _ in 0..120 {
        sim.step();
        assert_capacity(&sim);
        let current = pos(&sim, 0);
        assert!(current.x >= previous.x, "{previous:?} -> {current:?}");
        if current.x > center(0, 0).x && current.x < TILE {
            waited |= current == previous;
        }
        previous = current;
    }
    assert!(
        waited,
        "the regression must exercise a blocked partial segment"
    );
    assert!(pos(&sim, 0).x >= center(3, 0).x);
}

#[test]
fn group_membership_is_canonical_and_affects_hash_before_positions_differ() {
    let original = sim(&["....."], &[("worker", center(0, 0)); 2], "1.0");
    let target = center(4, 0);
    let mut grouped = original.clone();
    let mut reversed = original.clone();
    let mut separate = original.clone();
    move_units(&mut grouped, vec![UnitId(0), UnitId(1)], target);
    move_units(&mut reversed, vec![UnitId(1), UnitId(0), UnitId(1)], target);
    separate.step_with_commands([0, 1].map(|id| {
        (
            PLAYER,
            Command::Move {
                units: vec![UnitId(id)],
                target,
            },
        )
    }));
    for id in 0..2 {
        assert_eq!(pos(&grouped, id), pos(&separate, id));
    }
    assert_ne!(grouped.state_hash(), separate.state_hash());
    assert_eq!(grouped.state_hash(), reversed.state_hash());
    for _ in 0..100 {
        grouped.step();
        reversed.step();
        assert_eq!(grouped.state_hash(), reversed.state_hash());
    }
    let mut duplicate = original.clone();
    let mut single = original;
    move_units(
        &mut duplicate,
        vec![UnitId(0), UnitId(0), UnitId(99)],
        target,
    );
    move_units(&mut single, vec![UnitId(0)], target);
    assert_eq!(duplicate.state_hash(), single.state_hash());
}
