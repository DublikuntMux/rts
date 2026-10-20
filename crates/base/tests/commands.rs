use rts_base::{Command, INPUT_DELAY_TICKS, IVec2, MapData, PlayerId, Rules, Sim, TILE, UnitId};

fn sim() -> Sim {
    let rules = Rules::from_ron(include_str!("../../../assets/data/core.rules.ron")).unwrap();
    let map = MapData::from_ron(
        r#"(name: "commands", players: [(start: (0.5, 0.5)), (start: (1.5, 0.5))],
        tiles: ["..~", "..."], units: [
            (kind: "worker", owner: 0, pos: (0.5, 0.5)),
            (kind: "soldier", owner: 1, pos: (1.5, 0.5)),
        ])"#,
    )
    .unwrap();
    Sim::new(rules, &map).unwrap()
}

const fn move_order(units: Vec<UnitId>, target: IVec2) -> Command {
    Command::Move { units, target }
}

#[test]
fn commands_wait_for_input_delay_and_preserve_submission_order() {
    let mut sim = sim();
    let target = IVec2::new(TILE / 2, TILE * 3 / 2);
    sim.queue_command(PlayerId(0), move_order(vec![UnitId(0)], target));
    for _ in 1..INPUT_DELAY_TICKS {
        sim.step();
        assert_eq!(sim.move_target(UnitId(0)), None);
    }
    sim.step();
    assert_eq!(sim.move_target(UnitId(0)), Some(target));
    sim.queue_command(
        PlayerId(0),
        Command::Stop {
            units: vec![UnitId(0)],
        },
    );
    sim.queue_command(PlayerId(0), move_order(vec![UnitId(0)], target));
    for _ in 0..INPUT_DELAY_TICKS {
        sim.step();
    }
    assert_eq!(sim.move_target(UnitId(0)), Some(target));
    sim.queue_command(
        PlayerId(0),
        Command::Stop {
            units: vec![UnitId(0)],
        },
    );
    for _ in 0..INPUT_DELAY_TICKS {
        sim.step();
    }
    assert_eq!(sim.move_target(UnitId(0)), None);
}

#[test]
fn hostile_unknown_and_invalid_commands_do_not_change_orders() {
    let mut sim = sim();
    let initial = sim.clone();
    let target = IVec2::new(TILE / 2, TILE * 3 / 2);
    sim.queue_command(PlayerId(0), move_order(vec![UnitId(1), UnitId(99)], target));
    sim.queue_command(PlayerId(99), move_order(vec![UnitId(0)], target));
    sim.queue_command(PlayerId(0), move_order(vec![UnitId(0)], IVec2::new(-1, 0)));
    sim.queue_command(
        PlayerId(0),
        move_order(vec![UnitId(0)], IVec2::new(TILE * 2, 0)),
    );
    for target in [
        IVec2::new(i32::MIN, i32::MAX),
        IVec2::new(i32::MAX, i32::MIN),
        IVec2::new(TILE * 3, 0),
        IVec2::new(0, TILE * 2),
    ] {
        sim.queue_command(PlayerId(0), move_order(vec![UnitId(0)], target));
    }
    let mut expected = initial;
    for _ in 0..INPUT_DELAY_TICKS {
        sim.step();
        expected.step();
    }
    assert_eq!(sim.state_hash(), expected.state_hash());
}

#[test]
fn replacement_and_mixed_unit_lists_respect_ownership() {
    let mut sim = sim();
    let first = IVec2::new(TILE / 2, TILE * 3 / 2);
    let last = IVec2::new(TILE * 3 / 2, TILE * 3 / 2);
    sim.queue_command(PlayerId(0), move_order(vec![UnitId(0)], first));
    sim.queue_command(
        PlayerId(0),
        move_order(vec![UnitId(1), UnitId(0), UnitId(99)], last),
    );
    sim.queue_command(
        PlayerId(1),
        Command::Stop {
            units: vec![UnitId(0)],
        },
    );
    for _ in 0..INPUT_DELAY_TICKS {
        sim.step();
    }
    assert_eq!(sim.move_target(UnitId(0)), Some(last));
    assert_eq!(sim.move_target(UnitId(1)), None);
    sim.queue_command(PlayerId(0), move_order(vec![UnitId(0)], IVec2::new(-1, 0)));
    for _ in 0..INPUT_DELAY_TICKS {
        sim.step();
    }
    assert_eq!(sim.move_target(UnitId(0)), Some(last));
}

#[test]
fn queued_player_order_and_command_kind_affect_the_hash() {
    let original = sim();
    let target = IVec2::new(TILE / 2, TILE * 3 / 2);
    let move_command = move_order(vec![UnitId(0)], target);
    let stop_command = Command::Stop {
        units: vec![UnitId(0)],
    };
    let mut a = original.clone();
    let mut b = original.clone();
    a.queue_command(PlayerId(0), move_command.clone());
    b.queue_command(PlayerId(1), move_command.clone());
    assert_ne!(a.state_hash(), b.state_hash());
    let mut c = original.clone();
    c.queue_command(PlayerId(0), stop_command.clone());
    assert_ne!(a.state_hash(), c.state_hash());
    let mut d = original;
    d.queue_command(PlayerId(0), stop_command.clone());
    d.queue_command(PlayerId(0), move_command);
    a.queue_command(PlayerId(0), stop_command);
    assert_ne!(a.state_hash(), d.state_hash());
}

#[test]
fn pending_commands_are_part_of_state_hash() {
    let mut a = sim();
    let mut b = a.clone();
    let order = move_order(vec![UnitId(0)], IVec2::new(TILE / 2, TILE * 3 / 2));
    a.queue_command(PlayerId(0), order.clone());
    assert_ne!(a.state_hash(), b.state_hash());
    b.queue_command(PlayerId(0), order);
    assert_eq!(a.state_hash(), b.state_hash());
    for _ in 0..8 {
        a.step();
        b.step();
        assert_eq!(a.state_hash(), b.state_hash());
    }
}
