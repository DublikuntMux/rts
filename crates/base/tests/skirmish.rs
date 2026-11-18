use rts_base::{Command, IVec2, MapData, MatchOutcome, PlayerId, Rules, Sim, TILE, UnitId};

const RULES: &str = r##"(
    tile_capacity: 4,
    terrain: [(id: "grass", symbol: '.', speed_modifier: 1, color: "#00ff00")],
    units: [
        (id: "headquarters", sprite: "hq.png", size: 4, max_health: 20, radius: 0.5, speed: 0, building: true),
        (id: "soldier", sprite: "soldier.png", size: 2, max_health: 10, radius: 0.4, speed: 2,
         weapon: Some((range: 2, damage: 10, cooldown: 0.5))),
    ],
)"##;
const MAP: &str = r#"(
    name: "Skirmish test",
    players: [(start: (0.5, 0.5)), (start: (5.5, 0.5))],
    tiles: ["......"],
    units: [
        (kind: "headquarters", owner: 0, pos: (0.5, 0.5)),
        (kind: "headquarters", owner: 1, pos: (5.5, 0.5)),
        (kind: "soldier", owner: 0, pos: (1.5, 0.5)),
    ],
    skirmish: Some(()),
)"#;

fn simulation(map: &str) -> Sim {
    Sim::new(
        Rules::from_ron(RULES).unwrap(),
        &MapData::from_ron(map).unwrap(),
    )
    .unwrap()
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
    let mut sim = simulation(&MAP.replace("skirmish: Some(()),", ""));
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
