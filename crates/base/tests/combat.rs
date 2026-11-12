use rts_base::{Command, IVec2, MapData, PlayerId, Rules, Sim, TILE, UnitId};

/// Two-by-two squads with 10 health per soldier and one 5-damage shot per soldier every
/// half second, reaching two tiles.
const RULES: &str = r##"(tile_capacity: 4, terrain: [
    (id: "grass", symbol: '.', speed_modifier: 1, color: "#00ff00"),
    (id: "rock", symbol: '#', speed_modifier: 0, color: "#000000"),
], units: [
    (id: "rifles", sprite: "r.png", size: 2, squad: 4, max_health: 10, radius: 0.4, speed: 2,
     weapon: Some((range: 2, damage: 5, cooldown: 0.5))),
    (id: "worker", sprite: "w.png", size: 1, max_health: 10, radius: 0.25, speed: 2),
])"##;

fn sim(tiles: &str, units: &str) -> Sim {
    let map = MapData::from_ron(&format!(
        r#"(name: "combat", players: [(start: (0.5, 0.5)), (start: (0.5, 0.5))],
            tiles: [{tiles}], units: [{units}])"#
    ))
    .unwrap();
    Sim::new(Rules::from_ron(RULES).unwrap(), &map).unwrap()
}

fn soldiers(sim: &Sim, id: u32) -> Vec<i32> {
    sim.unit(UnitId(id)).unwrap().soldiers.clone()
}

#[test]
fn idle_squads_fire_one_shot_per_soldier_at_the_nearest_rank() {
    let mut sim = sim(
        r#"".....""#,
        r#"(kind: "rifles", owner: 0, pos: (0.5, 0.5)), (kind: "rifles", owner: 1, pos: (2.0, 0.5))"#,
    );
    sim.step();
    // Unit 0 fires first: four shots kill the two soldiers of the facing (left) column.
    assert_eq!(soldiers(&sim, 1), [0, 10, 0, 10]);
    // Unit 1 answers with its two survivors, killing one soldier of unit 0's right column.
    assert_eq!(soldiers(&sim, 0), [10, 0, 10, 10]);
    // Nothing fires again until the half-second cooldown has passed.
    for _ in 0..9 {
        sim.step();
        assert_eq!(soldiers(&sim, 1), [0, 10, 0, 10]);
    }
    sim.step();
    // Three shots: two kill soldier 1 and the third wounds soldier 3, whose single shot
    // answers at unit 0's nearest soldier.
    assert_eq!(soldiers(&sim, 1), [0, 0, 0, 5]);
    assert_eq!(soldiers(&sim, 0), [10, 0, 10, 5]);
    for _ in 0..10 {
        sim.step();
    }
    assert!(sim.unit(UnitId(1)).is_none());
    assert_eq!(sim.units().count(), 1);
    assert_eq!(soldiers(&sim, 0), [10, 0, 10, 5]);
}

#[test]
fn idle_units_ignore_allies_and_enemies_out_of_range() {
    let mut sim = sim(
        r#""......""#,
        r#"(kind: "rifles", owner: 0, pos: (0.5, 0.5)), (kind: "rifles", owner: 0, pos: (1.5, 0.5)),
           (kind: "rifles", owner: 1, pos: (4.5, 0.5))"#,
    );
    let hash = sim.state_hash();
    for _ in 0..40 {
        sim.step();
    }
    assert!(sim.units().all(|(_, unit)| unit.health() == 40));
    assert_ne!(sim.state_hash(), hash);
}

#[test]
fn attack_orders_pursue_fire_and_end_with_the_target() {
    let mut sim = sim(
        r#""..........""#,
        r#"(kind: "rifles", owner: 0, pos: (0.5, 0.5)), (kind: "worker", owner: 1, pos: (8.5, 0.5))"#,
    );
    sim.queue_command(
        PlayerId(0),
        Command::Attack {
            units: vec![UnitId(0)],
            target: UnitId(1),
        },
    );
    sim.step();
    sim.step();
    assert_eq!(sim.attack_target(UnitId(0)), Some(UnitId(1)));
    assert!(sim.move_target(UnitId(0)).is_some());
    let mut fired = None;
    for tick in 0..200 {
        sim.step();
        if sim.unit(UnitId(1)).is_none() {
            fired = Some(tick);
            break;
        }
    }
    assert!(fired.is_some(), "the worker was never destroyed");
    let attacker = sim.unit(UnitId(0)).unwrap();
    // It stopped at weapon range instead of walking onto the target.
    assert!(attacker.pos.x >= 8 * TILE + TILE / 2 - 2 * TILE);
    assert!(attacker.pos.x < 8 * TILE);
    sim.step();
    assert_eq!(sim.attack_target(UnitId(0)), None);
    assert_eq!(sim.move_target(UnitId(0)), None);
}

#[test]
fn move_and_stop_orders_cancel_attacks() {
    let mut sim = sim(
        r#""......", "......""#,
        r#"(kind: "rifles", owner: 0, pos: (0.5, 0.5)), (kind: "rifles", owner: 1, pos: (5.5, 1.5))"#,
    );
    sim.queue_command(
        PlayerId(0),
        Command::Attack {
            units: vec![UnitId(0)],
            target: UnitId(1),
        },
    );
    sim.step();
    sim.step();
    assert_eq!(sim.attack_target(UnitId(0)), Some(UnitId(1)));
    sim.queue_command(
        PlayerId(0),
        Command::Move {
            units: vec![UnitId(0)],
            target: IVec2::new(TILE / 2, TILE + TILE / 2),
        },
    );
    sim.step();
    sim.step();
    assert_eq!(sim.attack_target(UnitId(0)), None);
    for _ in 0..10 {
        sim.step();
    }
    sim.queue_command(
        PlayerId(0),
        Command::Attack {
            units: vec![UnitId(0)],
            target: UnitId(1),
        },
    );
    sim.step();
    sim.step();
    sim.queue_command(
        PlayerId(0),
        Command::Stop {
            units: vec![UnitId(0)],
        },
    );
    sim.step();
    sim.step();
    assert_eq!(sim.attack_target(UnitId(0)), None);
    assert_eq!(sim.move_target(UnitId(0)), None);
}

#[test]
fn attack_orders_need_an_armed_owned_attacker_and_an_enemy_target() {
    let mut sim = sim(
        r#""......""#,
        r#"(kind: "rifles", owner: 0, pos: (0.5, 0.5)), (kind: "worker", owner: 0, pos: (1.5, 0.5)),
           (kind: "worker", owner: 1, pos: (5.5, 0.5))"#,
    );
    for (player, units, target) in [
        (0, vec![UnitId(0)], UnitId(1)),
        (0, vec![UnitId(1)], UnitId(2)),
        (1, vec![UnitId(0)], UnitId(2)),
        (0, vec![UnitId(0)], UnitId(9)),
    ] {
        sim.queue_command(PlayerId(player), Command::Attack { units, target });
    }
    sim.step();
    sim.step();
    for id in 0..3 {
        assert_eq!(sim.attack_target(UnitId(id)), None);
    }
}

#[test]
fn unreachable_targets_end_the_attack_order() {
    let mut sim = sim(
        r#"".....#..""#,
        r#"(kind: "rifles", owner: 0, pos: (0.5, 0.5)), (kind: "worker", owner: 1, pos: (7.5, 0.5))"#,
    );
    sim.queue_command(
        PlayerId(0),
        Command::Attack {
            units: vec![UnitId(0)],
            target: UnitId(1),
        },
    );
    sim.step();
    sim.step();
    assert_eq!(sim.attack_target(UnitId(0)), None);
    assert_eq!(sim.move_target(UnitId(0)), None);
}

#[test]
fn promotion_restores_the_squad_and_raises_damage() {
    let rules = format!(
        r#"{}, ranks: [(name: "Veteran", kills: 3, damage_bonus: 100)])"#,
        RULES.strip_suffix(')').unwrap()
    );
    let map = MapData::from_ron(
        r#"(name: "ranks", players: [(start: (0.5, 0.5)), (start: (0.5, 0.5))],
            tiles: ["....."], units: [(kind: "rifles", owner: 0, pos: (0.5, 0.5)),
            (kind: "rifles", owner: 1, pos: (2.0, 0.5))])"#,
    )
    .unwrap();
    let rules = Rules::from_ron(&rules).unwrap();
    let mut sim = Sim::new(rules, &map).unwrap();
    sim.step();
    assert_eq!(sim.unit(UnitId(0)).unwrap().kills, 2);
    assert_eq!(soldiers(&sim, 0), [10, 0, 10, 10]);
    for _ in 0..10 {
        sim.step();
    }
    // The third kill promotes unit 0 and restores its fallen soldier before the reply.
    let veteran = sim.unit(UnitId(0)).unwrap();
    assert_eq!((veteran.kills, sim.rules().rank(veteran.kills)), (3, 1));
    assert_eq!(soldiers(&sim, 1), [0, 0, 0, 5]);
    assert_eq!(soldiers(&sim, 0), [10, 5, 10, 10]);
    for _ in 0..10 {
        sim.step();
    }
    assert!(sim.unit(UnitId(1)).is_none());
    // A second rank-up never happens without a further rank.
    assert_eq!(soldiers(&sim, 0), [10, 5, 10, 10]);
    assert_eq!(sim.unit(UnitId(0)).unwrap().kills, 4);
}
