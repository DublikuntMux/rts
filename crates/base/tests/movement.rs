use rts_base::{
    Command, INPUT_DELAY_TICKS, IVec2, MapData, PlayerId, PlayerSlot, Rules, Sim, TILE, UnitId,
    UnitPlacement, tile_of,
};

const UNIT: UnitId = UnitId(0);
const PLAYER: PlayerId = PlayerId(0);

fn sim(tiles: &[&str], start: IVec2, speed: &str) -> Sim {
    let rules = Rules::from_ron(&format!(
        r##"(tile_capacity: 4, terrain: [
            (id: "grass", symbol: '.', speed_modifier: 1.0, color: "#ffffff"),
            (id: "rock", symbol: '#', speed_modifier: 0.0, color: "#000000"),
        ], units: [(id: "worker", sprite: "textures/units/worker.png", size: 1, max_health: 40, radius: 0.3, speed: {speed})])"##
    ))
    .unwrap();
    Sim::new(
        rules,
        &MapData {
            name: "movement".into(),
            tiles: tiles.iter().map(|row| (*row).into()).collect(),
            players: vec![PlayerSlot { start }],
            units: vec![UnitPlacement {
                kind: "worker".into(),
                owner: 0,
                pos: start,
            }],
        },
    )
    .unwrap()
}

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

fn pos(sim: &Sim) -> IVec2 {
    sim.unit(UNIT).unwrap().pos
}

fn move_to(sim: &mut Sim, target: IVec2) {
    sim.queue_command(
        PLAYER,
        Command::Move {
            units: vec![UNIT],
            target,
        },
    );
}

fn steps(sim: &mut Sim, count: u32) {
    for _ in 0..count {
        sim.step();
    }
}

#[test]
fn moves_on_the_application_tick_and_reaches_exact_noncenter_target() {
    let start = center(0, 0);
    let target = IVec2::new(2 * TILE + 177, 83);
    let mut sim = sim(&["..."], start, "1.0");
    move_to(&mut sim, target);
    steps(&mut sim, INPUT_DELAY_TICKS - 1);
    assert_eq!(pos(&sim), start);
    sim.step();
    assert_eq!(pos(&sim), start + IVec2::new(TILE / 20, 0));
    assert_eq!(sim.move_target(UNIT), Some(target));
    steps(&mut sim, 100);
    assert_eq!(pos(&sim), target);
    assert_eq!(sim.move_target(UNIT), None);
    let completed = pos(&sim);
    steps(&mut sim, 10);
    assert_eq!(pos(&sim), completed);
}

#[test]
fn astar_uses_the_shortest_route_and_breaks_equal_cost_ties_by_tile_index() {
    let mut sim = sim(&[".....", ".###.", "....."], center(0, 1), "20.0");
    let target = center(4, 1);
    move_to(&mut sim, target);
    steps(&mut sim, INPUT_DELAY_TICKS - 1);
    for expected in [
        center(0, 0),
        center(1, 0),
        center(2, 0),
        center(3, 0),
        center(4, 0),
        target,
    ] {
        sim.step();
        assert_eq!(pos(&sim), expected);
        assert!(sim.terrain().speed_modifier(pos(&sim), sim.rules()) > 0);
    }
    assert_eq!(sim.move_target(UNIT), None);
}

#[test]
fn arbitrary_start_visits_its_center_before_neighbor_centers() {
    let start = IVec2::new(TILE * 9 / 10, TILE * 11 / 10);
    let mut sim = sim(&["#..", "..."], start, "1.0");
    let target = center(2, 0);
    move_to(&mut sim, target);
    steps(&mut sim, INPUT_DELAY_TICKS);
    assert!(pos(&sim).x < start.x);
    assert!(pos(&sim).y > start.y);
    assert_eq!(tile_of(pos(&sim)), tile_of(start));
    for _ in 0..150 {
        sim.step();
        assert!(sim.terrain().speed_modifier(pos(&sim), sim.rules()) > 0);
    }
    assert_eq!(pos(&sim), target);
}

#[test]
fn disconnected_and_diagonally_adjacent_tiles_are_unreachable() {
    for (tiles, target) in [
        (vec!["..#..", "..#.."], center(3, 0)),
        (vec![".#", "#."], center(1, 1)),
    ] {
        let mut sim = sim(&tiles, center(0, 0), "1.0");
        let mut expected = sim.clone();
        move_to(&mut sim, target);
        steps(&mut sim, 30);
        steps(&mut expected, 30);
        assert_eq!(sim.move_target(UNIT), None);
        assert_eq!(sim.state_hash(), expected.state_hash());
    }
}

#[test]
fn unreachable_replacement_preserves_a_valid_active_route() {
    let mut sim = sim(&["..#..", "..#.."], center(0, 0), "1.0");
    let target = center(1, 1);
    move_to(&mut sim, target);
    steps(&mut sim, INPUT_DELAY_TICKS);
    let mut expected = sim.clone();
    move_to(&mut sim, center(3, 0));
    steps(&mut sim, INPUT_DELAY_TICKS);
    steps(&mut expected, INPUT_DELAY_TICKS);
    assert_eq!(sim.move_target(UNIT), Some(target));
    assert_eq!(sim.state_hash(), expected.state_hash());
}

#[test]
fn delayed_stop_prevents_movement_on_its_application_tick_and_can_be_replaced() {
    let mut sim = sim(&["....."], center(0, 0), "1.0");
    move_to(&mut sim, center(4, 0));
    steps(&mut sim, INPUT_DELAY_TICKS);
    sim.queue_command(PLAYER, Command::Stop { units: vec![UNIT] });
    steps(&mut sim, INPUT_DELAY_TICKS - 1);
    let stopped = pos(&sim);
    steps(&mut sim, 5);
    assert_eq!(pos(&sim), stopped);
    assert_eq!(sim.move_target(UNIT), None);
    let target = IVec2::new(TILE / 4, TILE / 2);
    move_to(&mut sim, target);
    steps(&mut sim, INPUT_DELAY_TICKS);
    assert!(pos(&sim).x < stopped.x);
    steps(&mut sim, 20);
    assert_eq!(pos(&sim), target);
}

#[test]
fn speed_budget_keeps_fractional_units_between_ticks() {
    let start = center(0, 0);
    let mut sim = sim(&["..."], start, "0.125");
    move_to(&mut sim, center(2, 0));
    steps(&mut sim, INPUT_DELAY_TICKS - 1);
    steps(&mut sim, 20);
    assert_eq!(pos(&sim), start + IVec2::new(TILE / 8, 0));
    steps(&mut sim, 20);
    assert_eq!(pos(&sim), start + IVec2::new(TILE / 4, 0));
}

#[test]
fn one_unit_per_second_eventually_finishes_a_tiny_diagonal() {
    let start = center(0, 0);
    let mut sim = sim(&["."], start, "0.0009765625");
    let target = start + IVec2::splat(3);
    move_to(&mut sim, target);
    steps(&mut sim, INPUT_DELAY_TICKS - 1);
    steps(&mut sim, 99);
    assert_ne!(pos(&sim), target);
    sim.step();
    assert_eq!(pos(&sim), target);
    assert_eq!(sim.move_target(UNIT), None);
}

#[test]
fn fractional_motion_remainder_affects_hash_before_positions_differ() {
    let start = center(0, 0);
    let mut early = sim(&["."], start, "0.0009765625");
    let mut late = early.clone();
    let target = start + IVec2::new(3, 0);
    move_to(&mut early, target);
    early.step();
    late.step();
    move_to(&mut late, target);
    steps(&mut early, INPUT_DELAY_TICKS);
    steps(&mut late, INPUT_DELAY_TICKS);
    assert_eq!(pos(&early), pos(&late));
    assert_eq!(early.move_target(UNIT), late.move_target(UNIT));
    assert_ne!(early.state_hash(), late.state_hash());
    steps(&mut early, 18);
    steps(&mut late, 18);
    assert_ne!(pos(&early), pos(&late));
}

#[test]
fn high_speed_crosses_multiple_waypoints_and_stops_exactly() {
    let mut sim = sim(&["....", "###.", "...."], center(0, 0), "2097151.0");
    let target = IVec2::new(29, 2 * TILE + 77);
    move_to(&mut sim, target);
    steps(&mut sim, INPUT_DELAY_TICKS);
    assert_eq!(pos(&sim), target);
    assert_eq!(sim.move_target(UNIT), None);
}

#[test]
fn ordered_replay_matches_hashes_through_moves_replacements_and_stops() {
    let mut a = sim(&[".....", ".###.", "....."], center(0, 1), "1.125");
    let mut b = a.clone();
    for tick in 0..200 {
        let command = match tick {
            0 => Some(Command::Move {
                units: vec![UNIT],
                target: center(4, 1),
            }),
            20 => Some(Command::Move {
                units: vec![UNIT],
                target: center(2, 2),
            }),
            40 => Some(Command::Stop { units: vec![UNIT] }),
            50 => Some(Command::Move {
                units: vec![UNIT],
                target: center(4, 0),
            }),
            _ => None,
        };
        if let Some(command) = command {
            a.queue_command(PLAYER, command.clone());
            b.queue_command(PLAYER, command);
        }
        assert_eq!(a.state_hash(), b.state_hash());
        a.step();
        b.step();
        assert_eq!(a.state_hash(), b.state_hash());
        assert!(a.terrain().speed_modifier(pos(&a), a.rules()) > 0);
    }
    assert_eq!(pos(&a), center(4, 0));
}
