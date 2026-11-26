use super::*;
use crate::{MapData, TILE};

const RULES: &str = include_str!("../../../../assets/data/core.rules.ron");
const FRONTIER: &str = include_str!("../../../../assets/maps/frontier.map.ron");

/// A corridor with player 0 in the west and player 1 in the east.
fn corridor() -> Sim {
    let row = ".".repeat(40);
    let map = format!(
        r#"(name: "Corridor", players: [(start: (2.5, 2.5)), (start: (37.5, 2.5))],
            tiles: ["{row}", "{row}", "{row}", "{row}", "{row}"],
            units: [
                (kind: "soldier", owner: 0, pos: (2.5, 2.5)),
                (kind: "worker", owner: 0, pos: (1.5, 1.5)),
                (kind: "soldier", owner: 1, pos: (37.5, 2.5)),
                (kind: "worker", owner: 1, pos: (38.5, 3.5)),
            ])"#
    );
    Sim::new(
        Rules::from_ron(RULES).unwrap(),
        &MapData::from_ron(&map).unwrap(),
    )
    .unwrap()
}

fn frontier() -> Sim {
    Sim::new(
        Rules::from_ron(RULES).unwrap(),
        &MapData::from_ron(FRONTIER).unwrap(),
    )
    .unwrap()
}

fn tile(x: i32, y: i32) -> IVec2 {
    IVec2::new(x * TILE + TILE / 2, y * TILE + TILE / 2)
}

fn own_soldier(view: &PlayerView) -> UnitId {
    let soldier = view.rules().unit_kind("soldier").unwrap();
    view.units()
        .find(|(_, unit)| unit.owner == view.player() && unit.kind == soldier)
        .unwrap()
        .0
}

/// Steps the simulation once with `player`'s view-ID commands and refreshes the view.
fn step(
    sim: &mut Sim,
    knowledge: &mut PlayerKnowledge,
    view: &mut PlayerView,
    commands: &[Command],
) {
    let commands: Vec<_> = commands
        .iter()
        .filter_map(|command| knowledge.translate_command(sim, command))
        .map(|command| (knowledge.player(), command))
        .collect();
    sim.step_with_commands(commands);
    view.apply(knowledge.observe(sim)).unwrap();
}

#[test]
fn start_shows_only_own_surroundings() {
    let sim = corridor();
    let (_, view) = PlayerView::observe(&sim, PlayerId(0));
    assert_eq!(view.units().count(), 2);
    assert!(view.units().all(|(_, unit)| unit.owner == PlayerId(0)));
    assert_eq!(view.visibility(IVec2::new(2, 2)), TileVisibility::Visible);
    assert!(view.terrain(IVec2::new(2, 2)).is_some());
    assert_eq!(
        view.visibility(IVec2::new(37, 2)),
        TileVisibility::Unexplored
    );
    assert_eq!(view.terrain(IVec2::new(37, 2)), None);
    assert_eq!(view.start(), sim.players()[0].start);
    assert!(view.own(own_soldier(&view)).is_some());
}

#[test]
fn an_open_view_requires_complete_terrain_and_stays_open_without_observers() {
    let mut sim = corridor();
    let mut knowledge = PlayerKnowledge::with_fog(&sim, PlayerId(0), false);
    let mut view = PlayerView::new(sim.rules().clone(), knowledge.intro(&sim)).unwrap();
    let initial = knowledge.observe(&sim);
    let mut missing = initial.clone();
    missing.explored.pop();
    assert!(view.apply(missing).is_err());
    let mut unordered = initial.clone();
    unordered.explored.swap(0, 1);
    assert!(view.apply(unordered).is_err());
    view.apply(initial).unwrap();
    assert_eq!(view.units().count(), 4);
    let terrain_revision = view.terrain_revision();

    sim.remove_unit(UnitId(0));
    sim.remove_unit(UnitId(1));
    sim.step();
    let update = knowledge.observe(&sim);
    assert_eq!(update.explored, []);
    view.apply(update).unwrap();
    assert_eq!(view.units().count(), 2);
    assert_eq!(view.terrain_revision(), terrain_revision);
    assert_eq!(view.visibility(IVec2::new(37, 2)), TileVisibility::Visible);
}

#[test]
fn units_are_discovered_and_hidden_again_while_terrain_is_remembered() {
    let mut sim = corridor();
    let (mut knowledge, mut view) = PlayerView::observe(&sim, PlayerId(0));
    let soldier = own_soldier(&view);
    // Stop short of weapon range so the duel does not decide the test.
    step(
        &mut sim,
        &mut knowledge,
        &mut view,
        &[Command::Move {
            units: vec![soldier],
            target: tile(33, 2),
        }],
    );
    let mut seen = None;
    for _ in 0..400 {
        step(&mut sim, &mut knowledge, &mut view, &[]);
        if let Some((id, _)) = view.units().find(|(_, unit)| unit.owner == PlayerId(1)) {
            seen = Some(id);
            break;
        }
    }
    let enemy = seen.expect("the enemy comes into sight");
    let unit = view.unit(enemy).unwrap();
    assert_eq!(unit.cooldown, 0);
    assert!(view.own(enemy).is_none());
    assert_eq!(view.visibility(IVec2::new(35, 2)), TileVisibility::Visible);

    step(
        &mut sim,
        &mut knowledge,
        &mut view,
        &[Command::Move {
            units: vec![soldier],
            target: tile(2, 2),
        }],
    );
    for _ in 0..400 {
        step(&mut sim, &mut knowledge, &mut view, &[]);
    }
    assert!(view.unit(enemy).is_none(), "out of sight units vanish");
    assert_eq!(
        view.visibility(IVec2::new(35, 2)),
        TileVisibility::Remembered
    );
    assert!(view.terrain(IVec2::new(35, 2)).is_some());
    assert_eq!(
        view.visibility(IVec2::new(39, 0)),
        TileVisibility::Unexplored
    );
}

#[test]
fn hidden_activity_produces_identical_frames() {
    let mut quiet = frontier();
    let mut busy = quiet.clone();
    let mut quiet_knowledge = PlayerKnowledge::new(&quiet, PlayerId(0));
    let mut busy_knowledge = PlayerKnowledge::new(&busy, PlayerId(0));
    assert_eq!(
        quiet_knowledge.observe(&quiet),
        busy_knowledge.observe(&busy)
    );
    let enemy: Vec<_> = busy
        .units()
        .filter(|(_, unit)| unit.owner == PlayerId(1))
        .map(|(id, _)| id)
        .collect();
    let headquarters = busy.headquarters(PlayerId(1)).unwrap();
    let workers: Vec<_> = enemy
        .iter()
        .copied()
        .filter(|&id| id != headquarters)
        .collect();
    let (gold, _) = busy
        .deposits()
        .find(|(_, deposit)| deposit.pos.x > 25 * TILE)
        .unwrap();
    busy.step_with_commands([
        (
            PlayerId(1),
            Command::Gather {
                units: workers,
                deposit: gold,
            },
        ),
        (
            PlayerId(1),
            Command::TrainWorker {
                units: vec![headquarters],
            },
        ),
    ]);
    quiet.step();
    assert_ne!(quiet.state_hash(), busy.state_hash());
    assert_eq!(
        quiet_knowledge.observe(&quiet),
        busy_knowledge.observe(&busy)
    );
    for _ in 0..600 {
        busy.step();
        quiet.step();
        let (a, b) = (
            quiet_knowledge.observe(&quiet),
            busy_knowledge.observe(&busy),
        );
        assert_eq!(ron::to_string(&a).unwrap(), ron::to_string(&b).unwrap());
    }
    assert_ne!(quiet.state_hash(), busy.state_hash());
    assert_ne!(
        quiet.resources(PlayerId(1)),
        busy.resources(PlayerId(1)),
        "the hidden economy really diverged"
    );
}

#[test]
fn commands_are_bound_to_the_player_and_its_knowledge() {
    let sim = frontier();
    let (knowledge, view) = PlayerView::observe(&sim, PlayerId(0));
    let worker = view
        .units()
        .find(|(_, unit)| {
            unit.owner == PlayerId(0) && unit.kind == sim.rules().unit_kind("worker").unwrap()
        })
        .unwrap()
        .0;
    let enemy_hq = sim.headquarters(PlayerId(1)).unwrap();
    // Unknown and foreign IDs are dropped; a command with nothing left is rejected.
    assert_eq!(
        knowledge.translate_command(
            &sim,
            &Command::Stop {
                units: vec![UnitId(999)]
            }
        ),
        None
    );
    // Authority IDs of hidden enemies are not valid view IDs for attack targets.
    assert_eq!(
        knowledge.translate_command(
            &sim,
            &Command::Attack {
                units: vec![worker],
                target: enemy_hq,
            }
        ),
        None
    );
    let known = view.deposits().next().unwrap().0;
    let unknown = DepositId(u32::try_from(sim.deposits().count()).unwrap());
    assert!(
        knowledge
            .translate_command(
                &sim,
                &Command::Gather {
                    units: vec![worker],
                    deposit: known
                }
            )
            .is_some()
    );
    assert_eq!(
        knowledge.translate_command(
            &sim,
            &Command::Gather {
                units: vec![worker],
                deposit: unknown
            }
        ),
        None
    );
    assert_eq!(
        knowledge.translate_command(
            &sim,
            &Command::BuildBarracks {
                units: vec![worker],
                target: tile(30, 2),
            }
        ),
        None,
        "unexplored build sites are refused"
    );
    let moved = knowledge
        .translate_command(
            &sim,
            &Command::Move {
                units: vec![worker, UnitId(999)],
                target: tile(30, 2),
            },
        )
        .unwrap();
    let authority = moved.units()[0];
    assert_eq!(moved.units().len(), 1);
    assert_eq!(sim.unit(authority).unwrap().owner, PlayerId(0));
    // Player 1's knowledge cannot command player 0's units with the same view IDs.
    let (other, _) = PlayerView::observe(&sim, PlayerId(1));
    let translated = other
        .translate_command(
            &sim,
            &Command::Stop {
                units: vec![worker],
            },
        )
        .map(|command| command.units().to_vec());
    assert!(translated.is_none_or(|units| {
        units
            .iter()
            .all(|id| sim.unit(*id).unwrap().owner == PlayerId(1))
    }));
}

#[test]
fn views_withhold_enemy_economy_and_starts() {
    let sim = frontier();
    let (_, view) = PlayerView::observe(&sim, PlayerId(0));
    assert_eq!(view.resources(), sim.resources(PlayerId(0)));
    assert!(view.headquarters().is_some());
    assert!(
        view.deposits()
            .all(|(_, deposit)| deposit.pos.x < 17 * TILE)
    );
    assert!(view.deposits().count() < sim.deposits().count());
    let encoded = ron::to_string(&PlayerKnowledge::new(&sim, PlayerId(0)).observe(&sim)).unwrap();
    let enemy_start = sim.players()[1].start;
    assert!(!encoded.contains(&format!("{}", enemy_start.x)));
}

#[test]
fn views_reject_malformed_frames() {
    let sim = corridor();
    let (mut knowledge, view) = PlayerView::observe(&sim, PlayerId(0));
    let mut next = sim;
    next.step();
    let frame = knowledge.observe(&next);

    let mut late = view.clone();
    let mut skipped = frame.clone();
    skipped.tick += 1;
    assert!(late.apply(skipped).is_err());

    let mut bad = frame.clone();
    bad.removed.push(UnitId(77));
    assert!(late.apply(bad).is_err());

    let mut bad = frame.clone();
    bad.explored.push((0, TerrainId(0)));
    assert!(late.apply(bad).is_err(), "explored twice");

    let mut bad = frame.clone();
    let mut unit = view.units.values().next().unwrap().clone();
    unit.own = None;
    bad.units.push(unit);
    assert!(late.apply(bad).is_err(), "own units carry their orders");

    let mut bad = frame.clone();
    bad.resources = Some(Resources::default());
    assert!(late.apply(bad).is_err(), "no economy outside skirmish");

    late.apply(frame).unwrap();
    assert_eq!(late.tick(), 1);
}

#[test]
fn fog_discs_follow_sight_radius() {
    let mut fog = FogVisibility::new(IVec2::new(20, 20));
    let discovered = fog.update([(tile(10, 10), 2)]);
    assert!(discovered.contains(&(10 * 20 + 12)));
    assert_eq!(fog.tile(IVec2::new(12, 10)), TileVisibility::Visible);
    assert_eq!(fog.tile(IVec2::new(13, 10)), TileVisibility::Unexplored);
    assert_eq!(fog.tile(IVec2::new(12, 12)), TileVisibility::Unexplored);
    assert_eq!(fog.update([(tile(10, 10), 2)]), Vec::<u32>::new());
    fog.update([]);
    assert_eq!(fog.tile(IVec2::new(12, 10)), TileVisibility::Remembered);
}

#[test]
fn queued_attacks_are_revalidated_when_they_execute() {
    let mut sim = corridor();
    let (mut knowledge, mut view) = PlayerView::observe(&sim, PlayerId(0));
    let soldier = own_soldier(&view);
    step(
        &mut sim,
        &mut knowledge,
        &mut view,
        &[Command::Move {
            units: vec![soldier],
            target: tile(33, 2),
        }],
    );
    let enemy = (0..400)
        .find_map(|_| {
            step(&mut sim, &mut knowledge, &mut view, &[]);
            view.units()
                .find(|(_, unit)| unit.owner == PlayerId(1))
                .map(|(id, _)| id)
        })
        .expect("the enemy comes into sight");
    let bound = knowledge
        .translate_command(
            &sim,
            &Command::Attack {
                units: vec![soldier],
                target: enemy,
            },
        )
        .expect("a visible target is accepted");
    assert_eq!(
        knowledge.revalidate(&sim, bound.clone()),
        Some(bound.clone())
    );
    step(
        &mut sim,
        &mut knowledge,
        &mut view,
        &[Command::Move {
            units: vec![soldier],
            target: tile(2, 2),
        }],
    );
    for _ in 0..400 {
        step(&mut sim, &mut knowledge, &mut view, &[]);
    }
    assert!(view.unit(enemy).is_none());
    assert_eq!(
        knowledge.revalidate(&sim, bound),
        None,
        "a target that left sight during the input delay is refused"
    );
}

#[test]
fn gather_and_build_targets_need_current_sight() {
    let mut sim = frontier();
    let (mut knowledge, mut view) = PlayerView::observe(&sim, PlayerId(0));
    let worker_kind = sim.rules().unit_kind("worker").unwrap();
    let worker = view
        .units()
        .find(|(_, unit)| unit.owner == PlayerId(0) && unit.kind == worker_kind)
        .unwrap()
        .0;
    let go = |target| Command::Move {
        units: vec![worker],
        target,
    };
    step(&mut sim, &mut knowledge, &mut view, &[go(tile(16, 11))]);
    for _ in 0..160 {
        step(&mut sim, &mut knowledge, &mut view, &[]);
    }
    let (far, deposit) = view
        .deposits()
        .find(|(_, deposit)| tile_of(deposit.pos) == IVec2::new(16, 10))
        .expect("the worker saw the central gold");
    let site = tile(17, 12);
    assert!(view.can_place_barracks(site));
    let gather = Command::Gather {
        units: vec![worker],
        deposit: far,
    };
    let build = Command::BuildBarracks {
        units: vec![worker],
        target: site,
    };
    let bound_gather = knowledge.translate_command(&sim, &gather).unwrap();
    let bound_build = knowledge.translate_command(&sim, &build).unwrap();

    step(&mut sim, &mut knowledge, &mut view, &[go(tile(5, 9))]);
    for _ in 0..200 {
        step(&mut sim, &mut knowledge, &mut view, &[]);
    }
    assert_eq!(
        view.visibility(tile_of(deposit.pos)),
        TileVisibility::Remembered
    );
    assert!(
        view.deposit(far).is_some(),
        "the deposit is still remembered"
    );
    assert!(!view.can_place_barracks(site));
    assert_eq!(knowledge.translate_command(&sim, &gather), None);
    assert_eq!(knowledge.translate_command(&sim, &build), None);
    assert_eq!(knowledge.revalidate(&sim, bound_gather), None);
    assert_eq!(knowledge.revalidate(&sim, bound_build), None);
}
