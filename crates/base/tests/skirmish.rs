use rts_base::{
    Command, DepositId, IVec2, MapData, MatchOutcome, PlayerId, ResourceKind, Resources, Rules,
    Sim, TILE, Training, TrainingKind, UnitId,
};

const RULES: &str = r##"(
    tile_capacity: 4,
    terrain: [(id: "grass", symbol: '.', speed_modifier: 1, color: "#00ff00")],
    units: [
        (id: "headquarters", sprite: "hq.png", size: 4, max_health: 20, radius: 0.5, speed: 0, building: true),
        (id: "worker", sprite: "worker.png", size: 1, max_health: 30, radius: 0.25, speed: 10),
        (id: "barracks", sprite: "barracks.png", size: 4, max_health: 50, radius: 0.5, speed: 0, building: true),
        (id: "soldier", sprite: "soldier.png", size: 2, max_health: 10, radius: 0.4, speed: 2,
         weapon: Some((range: 2, damage: 10, cooldown: 0.5))),
    ],
    economy: Some((
        initial: (wood: 20, gold: 20),
        worker_cost: (wood: 4, gold: 5),
        barracks_cost: (wood: 10, gold: 12),
        rifle_cost: (wood: 14, gold: 12),
        worker_ticks: 4, barracks_ticks: 5, rifle_ticks: 6, harvest_ticks: 3, carry_capacity: 8,
    )),
)"##;
const MAP: &str = r#"(
    name: "Skirmish test",
    players: [(start: (0.5, 0.5)), (start: (5.5, 0.5))],
    tiles: ["......", "......", "......"],
    units: [
        (kind: "headquarters", owner: 0, pos: (0.5, 0.5)),
        (kind: "headquarters", owner: 1, pos: (5.5, 0.5)),
        (kind: "soldier", owner: 0, pos: (1.5, 0.5)),
        (kind: "worker", owner: 0, pos: (0.5, 1.5)),
        (kind: "worker", owner: 0, pos: (0.5, 2.5)),
    ],
    skirmish: Some((deposits: [
        (kind: Wood, pos: (2.5, 1.5), remaining: 11),
        (kind: Gold, pos: (2.5, 2.5), remaining: 13),
    ])),
)"#;

fn simulation(map: &str) -> Sim {
    Sim::new(
        Rules::from_ron(RULES).unwrap(),
        &MapData::from_ron(map).unwrap(),
    )
    .unwrap()
}

#[test]
fn a_producer_rallies_new_units_and_keeps_its_point_private() {
    let mut sim = simulation(MAP);
    let player = PlayerId(0);
    let hq = sim.headquarters(player).unwrap();
    let enemy = sim.headquarters(PlayerId(1)).unwrap();
    let target = IVec2::new(3 * TILE + 190, TILE + 95);
    let previous = sim.state_hash();
    sim.step_with_commands([
        (
            player,
            Command::SetRallyPoint {
                units: vec![hq, enemy, UnitId(3)],
                target,
            },
        ),
        (player, Command::TrainWorker { units: vec![hq] }),
    ]);
    assert_eq!(sim.rally_point(hq), Some(target));
    assert_eq!(sim.rally_point(enemy), None);
    assert_eq!(sim.rally_point(UnitId(3)), None);
    assert_ne!(sim.state_hash(), previous);
    let own = rts_base::PlayerView::observe(&sim, player).1;
    assert_eq!(own.rally_point(own.headquarters().unwrap()), Some(target));
    let other = rts_base::PlayerView::observe(&sim, PlayerId(1)).1;
    assert!(other.units().all(|(id, _)| other.rally_point(id).is_none()));
    for _ in 0..40 {
        sim.step();
    }
    assert_eq!(sim.unit(UnitId(5)).unwrap().pos, target);
    sim.remove_unit(hq);
    assert_eq!(sim.rally_point(hq), None);
}

#[test]
fn rally_points_reject_remembered_obstacles_without_revealing_unexplored_ones() {
    let rules = RULES.replace(
        "speed_modifier: 1, color: \"#00ff00\")",
        "speed_modifier: 1, color: \"#00ff00\"), (id: \"rock\", symbol: '#', speed_modifier: 0, color: \"#555555\")",
    );
    let map = MapData::from_ron(
        r#"(
        name: "Fogged rally", players: [(start: (0.5,0.5)), (start: (15.5,0.5))],
        tiles: [".......#........", "................"],
        units: [
            (kind: "headquarters", owner: 0, pos: (0.5,0.5)),
            (kind: "headquarters", owner: 1, pos: (15.5,0.5)),
            (kind: "worker", owner: 0, pos: (6.5,0.5)),
        ],
        skirmish: Some((deposits: [
            (kind: Wood, pos: (2.5,1.5), remaining: 11),
            (kind: Gold, pos: (3.5,1.5), remaining: 13),
        ])),
    )"#,
    )
    .unwrap();
    let original = Sim::new(Rules::from_ron(&rules).unwrap(), &map).unwrap();
    let target = IVec2::new(7 * TILE + TILE / 2, TILE / 2);
    for known in [false, true] {
        let mut sim = original.clone();
        if !known {
            sim.remove_unit(UnitId(2));
        }
        sim.enforce_fog();
        sim.remove_unit(UnitId(2));
        let player = PlayerId(0);
        let hq = sim.headquarters(player).unwrap();
        sim.step_with_commands([(
            player,
            Command::SetRallyPoint {
                units: vec![hq],
                target,
            },
        )]);
        assert_eq!(sim.rally_point(hq), (!known).then_some(target));
    }
}

#[test]
fn headquarters_are_stationary_and_the_enemy_can_destroy_them() {
    let mut sim = simulation(MAP);
    let own = sim.headquarters(PlayerId(0)).unwrap();
    let enemy = sim.headquarters(PlayerId(1)).unwrap();
    let starting_pos = sim.unit(own).unwrap().pos;
    let soldier = UnitId(2);
    sim.step_with_commands([
        (
            PlayerId(0),
            Command::Move {
                units: vec![own],
                target: IVec2::new(TILE, 0),
            },
        ),
        (
            PlayerId(0),
            Command::Attack {
                units: vec![soldier],
                target: enemy,
            },
        ),
    ]);
    assert!(sim.is_skirmish());
    assert_eq!(sim.move_target(own), None);
    assert_eq!(sim.attack_target(soldier), Some(enemy));
    for _ in 0..80 {
        sim.step();
    }
    assert_eq!(sim.unit(own).unwrap().pos, starting_pos);
    assert!(sim.headquarters(PlayerId(1)).is_none());
    assert_eq!(sim.outcome(), Some(MatchOutcome::Winner(PlayerId(0))));

    let pos = sim.unit(soldier).unwrap().pos;
    sim.step_with_commands([(
        PlayerId(0),
        Command::Move {
            units: vec![soldier],
            target: starting_pos,
        },
    )]);
    assert_eq!(sim.unit(soldier).unwrap().pos, pos);
    assert_eq!(sim.outcome(), Some(MatchOutcome::Winner(PlayerId(0))));
}

#[test]
fn maps_without_an_objective_keep_their_sandbox_behavior() {
    let mut map = MapData::from_ron(MAP).unwrap();
    map.skirmish = None;
    let mut sim = Sim::new(Rules::from_ron(RULES).unwrap(), &map).unwrap();
    assert!(!sim.is_skirmish());
    sim.remove_unit(UnitId(1)).unwrap();
    sim.step();
    assert_eq!(sim.outcome(), None);
}

#[test]
fn a_skirmish_requires_one_headquarters_per_player() {
    let map = MapData::from_ron(
        &MAP.replace(r#"(kind: "headquarters", owner: 1, pos: (5.5, 0.5)),"#, ""),
    )
    .unwrap();
    let error = Sim::new(Rules::from_ron(RULES).unwrap(), &map).unwrap_err();
    assert!(error.to_string().contains("one headquarters"));
}

#[test]
fn simultaneous_headquarters_losses_are_a_draw() {
    let mut sim = simulation(MAP);
    sim.remove_unit(UnitId(0)).unwrap();
    sim.remove_unit(UnitId(1)).unwrap();
    sim.step();
    assert_eq!(sim.outcome(), Some(MatchOutcome::Draw));
}

const PLAYER: PlayerId = PlayerId(0);
const WORKER: UnitId = UnitId(3);
const OTHER_WORKER: UnitId = UnitId(4);

const fn center(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

fn issue(sim: &mut Sim, commands: impl IntoIterator<Item = Command>) {
    sim.step_with_commands(commands.into_iter().map(|command| (PLAYER, command)));
}

#[test]
fn workers_deliver_both_resources_and_fund_construction_and_training() {
    let mut sim = simulation(MAP);
    let headquarters = sim.headquarters(PLAYER).unwrap();
    let target = center(1, 1);
    issue(
        &mut sim,
        [
            Command::Gather {
                units: vec![WORKER],
                deposit: DepositId(0),
            },
            Command::Gather {
                units: vec![OTHER_WORKER],
                deposit: DepositId(1),
            },
        ],
    );
    while sim.cargo(WORKER).is_none() && sim.tick() < 20 {
        sim.step();
    }
    assert_eq!(sim.cargo(WORKER), Some((ResourceKind::Wood, 8)));
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 20, gold: 20 })
    );
    for _ in 0..50 {
        sim.step();
    }
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 31, gold: 33 })
    );
    assert_eq!(
        sim.resources(PlayerId(1)),
        Some(Resources { wood: 20, gold: 20 })
    );
    assert!(sim.deposits().all(|(_, deposit)| deposit.remaining == 0));
    assert_eq!(sim.gather_target(WORKER), None);
    assert_eq!(sim.gather_target(OTHER_WORKER), None);

    issue(
        &mut sim,
        [
            Command::BuildBarracks {
                units: vec![OTHER_WORKER, WORKER],
                target: target - IVec2::ONE,
            },
            Command::TrainWorker {
                units: vec![headquarters, headquarters],
            },
        ],
    );
    assert_eq!(sim.build_target(WORKER), Some(target));
    assert_eq!(sim.build_target(OTHER_WORKER), None);
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 17, gold: 16 })
    );
    assert_eq!(
        sim.production(headquarters).unwrap()[0].kind,
        TrainingKind::Worker
    );
    for _ in 0..10 {
        sim.step();
    }
    let barracks = sim
        .units()
        .find(|(_, unit)| sim.rules().unit(unit.kind).id == "barracks")
        .map(|(id, unit)| {
            assert_eq!(unit.pos, target);
            id
        })
        .unwrap();
    assert_eq!(sim.production(headquarters).unwrap(), []);
    assert_eq!(
        sim.units()
            .filter(|(_, unit)| sim.rules().unit(unit.kind).id == "worker")
            .count(),
        3,
    );

    issue(
        &mut sim,
        [Command::TrainRifleSquad {
            units: vec![headquarters, barracks],
        }],
    );
    assert_eq!(
        sim.production(barracks).unwrap()[0].kind,
        TrainingKind::RifleSquad
    );
    assert_eq!(sim.resources(PLAYER), Some(Resources { wood: 3, gold: 4 }));
    for _ in 0..5 {
        sim.step();
    }
    assert_eq!(sim.production(barracks).unwrap(), []);
    assert_eq!(
        sim.units()
            .filter(|(_, unit)| sim.rules().unit(unit.kind).id == "soldier")
            .count(),
        2,
    );
}

#[test]
fn invalid_and_conflicting_construction_orders_do_not_charge_twice() {
    let mut sim = simulation(MAP);
    let target = center(1, 1);
    issue(
        &mut sim,
        [
            Command::BuildBarracks {
                units: vec![WORKER],
                target: center(2, 1),
            },
            Command::BuildBarracks {
                units: vec![UnitId(2)],
                target,
            },
            Command::BuildBarracks {
                units: vec![WORKER],
                target,
            },
            Command::BuildBarracks {
                units: vec![OTHER_WORKER],
                target,
            },
        ],
    );
    assert_eq!(sim.resources(PLAYER), Some(Resources { wood: 10, gold: 8 }));
    assert_eq!(sim.build_target(WORKER), Some(target));
    assert_eq!(sim.build_target(OTHER_WORKER), None);
    assert!(!sim.can_place_barracks(PLAYER, target));

    issue(
        &mut sim,
        [Command::Stop {
            units: vec![WORKER],
        }],
    );
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 20, gold: 20 })
    );
    assert!(sim.can_place_barracks(PLAYER, target));
}

#[test]
fn both_resource_balances_are_required_and_enemy_producers_are_ignored() {
    let mut sim = simulation(MAP);
    let opponent = sim.headquarters(PlayerId(1)).unwrap();
    let hq = sim.headquarters(PLAYER).unwrap();
    issue(
        &mut sim,
        [
            Command::TrainWorker {
                units: vec![opponent, hq],
            },
            Command::TrainWorker { units: vec![hq] },
            Command::BuildBarracks {
                units: vec![WORKER],
                target: center(1, 1),
            },
        ],
    );
    // Enough wood to build, but not enough gold.
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 12, gold: 10 })
    );
    assert_eq!(sim.build_target(WORKER), None);
    assert_eq!(sim.production(hq).unwrap().len(), 2);
    assert_eq!(sim.production(opponent).unwrap(), []);
    assert_eq!(
        sim.resources(PlayerId(1)),
        Some(Resources { wood: 20, gold: 20 })
    );
}

#[test]
fn a_worker_can_deliver_cargo_after_its_gather_order_is_replaced() {
    let mut sim = simulation(MAP);
    issue(
        &mut sim,
        [Command::Gather {
            units: vec![WORKER],
            deposit: DepositId(0),
        }],
    );
    while sim.cargo(WORKER).is_none() && sim.tick() < 20 {
        sim.step();
    }
    assert_eq!(sim.cargo(WORKER), Some((ResourceKind::Wood, 8)));
    issue(
        &mut sim,
        [Command::Move {
            units: vec![WORKER],
            target: center(0, 1),
        }],
    );
    assert_eq!(sim.gather_target(WORKER), None);
    for _ in 0..10 {
        sim.step();
    }
    assert_eq!(sim.cargo(WORKER), None);
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 28, gold: 20 })
    );
}

fn scout_rules() -> String {
    RULES
        .replace(
            "    ],\n    economy",
            r#"        (id: "scout", sprite: "scout.png", size: 1, max_health: 25, radius: 0.3, speed: 4, sight: 8,
         weapon: Some((range: 2, damage: 2, cooldown: 1))),
    ],
    economy"#,
        )
        .replace(
            "carry_capacity: 8,",
            "carry_capacity: 8, scout: Some((cost: (wood: 7, gold: 6), ticks: 3)),",
        )
}

const BARRACKS_MAP: &str = r#"(
    name: "Scout test",
    players: [(start: (0.5, 0.5)), (start: (5.5, 0.5))],
    tiles: ["......", "......", "......"],
    units: [
        (kind: "headquarters", owner: 0, pos: (0.5, 0.5)),
        (kind: "headquarters", owner: 1, pos: (5.5, 0.5)),
        (kind: "barracks", owner: 0, pos: (2.5, 2.5)),
        (kind: "barracks", owner: 1, pos: (4.5, 2.5)),
    ],
    skirmish: Some((deposits: [
        (kind: Wood, pos: (2.5, 1.5), remaining: 11),
        (kind: Gold, pos: (3.5, 1.5), remaining: 13),
    ])),
)"#;

fn scouts(sim: &Sim) -> Vec<UnitId> {
    sim.units()
        .filter(|(_, unit)| sim.rules().unit(unit.kind).id == "scout")
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn barracks_train_fast_far_sighted_scouts_for_wood_and_gold() {
    let rules = Rules::from_ron(&scout_rules()).unwrap();
    let mut sim = Sim::new(rules, &MapData::from_ron(BARRACKS_MAP).unwrap()).unwrap();
    let headquarters = sim.headquarters(PLAYER).unwrap();
    let (barracks, enemy_barracks) = (UnitId(2), UnitId(3));
    issue(
        &mut sim,
        [Command::TrainScout {
            units: vec![headquarters, enemy_barracks, barracks, barracks],
        }],
    );
    assert_eq!(sim.production(headquarters).unwrap(), []);
    assert_eq!(sim.production(enemy_barracks).unwrap(), []);
    assert_eq!(
        sim.production(barracks).unwrap(),
        [Training {
            kind: TrainingKind::Scout,
            remaining: 2,
        }]
    );
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 13, gold: 14 })
    );
    assert_eq!(scouts(&sim), []);
    sim.step();
    sim.step();
    let [scout] = scouts(&sim)[..] else {
        panic!("one scout spawns");
    };
    let unit = sim.unit(scout).unwrap();
    assert_eq!(unit.owner, PLAYER);
    let definition = sim.rules().unit(unit.kind);
    assert_eq!(definition.sight, 8);
    assert!(definition.weapon.is_some());
    let soldier = sim.rules().unit_kind("soldier").unwrap();
    assert!(definition.speed > sim.rules().unit(soldier).speed);
    assert_eq!(sim.production(barracks).unwrap(), []);

    issue(
        &mut sim,
        [Command::Move {
            units: vec![scout],
            target: center(0, 2),
        }],
    );
    assert!(sim.move_target(scout).is_some());
}

#[test]
fn rules_without_scout_training_ignore_scout_orders() {
    let mut sim = simulation(MAP);
    let mut reference = simulation(MAP);
    let headquarters = sim.headquarters(PLAYER).unwrap();
    issue(
        &mut sim,
        [Command::TrainScout {
            units: vec![headquarters],
        }],
    );
    reference.step();
    assert_eq!(
        sim.resources(PLAYER),
        Some(Resources { wood: 20, gold: 20 })
    );
    assert_eq!(sim.state_hash(), reference.state_hash());
}

#[test]
fn scout_training_needs_an_armed_scout_and_a_full_price() {
    let map = MapData::from_ron(BARRACKS_MAP).unwrap();
    let load = |rules: String| Rules::from_ron(&rules).and_then(|rules| Sim::new(rules, &map));
    assert!(load(scout_rules()).is_ok());
    for rules in [
        scout_rules().replace(r#"(id: "scout""#, r#"(id: "spy""#),
        scout_rules().replace(
            ", sight: 8,\n         weapon: Some((range: 2, damage: 2, cooldown: 1))",
            ", sight: 8",
        ),
        scout_rules().replace("cost: (wood: 7, gold: 6)", "cost: (wood: 7, gold: 0)"),
        scout_rules().replace("ticks: 3)", "ticks: 0)"),
    ] {
        let error = load(rules).expect_err("invalid scout rules");
        assert!(error.to_string().contains("scout"), "{error}");
    }
}
