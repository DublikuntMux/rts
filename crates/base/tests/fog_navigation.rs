use rts_base::{
    Command, IVec2, MapData, PlayerId, PlayerKnowledge, Rules, Sim, TILE, TileVisibility, UnitId,
};

const RULES: &str = include_str!("../../../assets/data/core.rules.ron");
const DESTINATION: IVec2 = IVec2::new(26 * TILE + TILE / 2, TILE / 2);

fn corridor(remote_tile: char) -> Sim {
    let row = format!("{}{remote_tile}{}", ".".repeat(26), ".".repeat(5));
    let map = format!(
        r#"(name: "Recon corridor", players: [(start: (1.5, 0.5)), (start: (31.5, 0.5))],
            tiles: ["{row}"],
            units: [(kind: "scout", owner: 0, pos: (1.5, 0.5))])"#
    );
    let mut sim = Sim::new(
        Rules::from_ron(RULES).unwrap(),
        &MapData::from_ron(&map).unwrap(),
    )
    .unwrap();
    sim.enforce_fog();
    sim
}

#[test]
fn exploration_moves_without_revealing_passability_before_scouts_see_it() {
    let (mut open, mut blocked) = (corridor('.'), corridor('#'));
    let player = PlayerId(0);
    let (mut open_view, mut blocked_view) = (
        PlayerKnowledge::new(&open, player),
        PlayerKnowledge::new(&blocked, player),
    );
    assert_eq!(open_view.observe(&open), blocked_view.observe(&blocked));
    let scout = UnitId(0);
    let destination = Command::Move {
        units: vec![scout],
        target: DESTINATION,
    };
    open.step_with_commands([(player, destination.clone())]);
    blocked.step_with_commands([(player, destination)]);
    assert!(open.move_target(scout).is_some());
    let goal_tile = IVec2::new(26, 0);
    let start = open.players()[0].start;
    let mut revealed = false;
    for _ in 0..240 {
        let a = open_view.observe(&open);
        let b = blocked_view.observe(&blocked);
        if open_view.visibility().tile(goal_tile) != TileVisibility::Unexplored {
            assert_eq!(
                blocked_view.visibility().tile(goal_tile),
                TileVisibility::Visible
            );
            assert!(open.unit(scout).unwrap().pos.x > start.x);
            revealed = true;
            break;
        }
        assert_eq!(a, b, "hidden terrain changed the player's frame");
        open.step();
        blocked.step();
    }
    assert!(revealed, "the scout did not reach sight of the destination");
    for _ in 0..240 {
        open.step();
        blocked.step();
    }
    assert_eq!(open.unit(scout).unwrap().pos, DESTINATION);
    assert!(blocked.unit(scout).unwrap().pos.x < DESTINATION.x);
    assert_eq!(blocked.move_target(scout), None);
}

#[test]
fn stopping_a_scout_discards_its_exploration_goal() {
    let mut sim = corridor('.');
    let scout = UnitId(0);
    sim.step_with_commands([(
        PlayerId(0),
        Command::Move {
            units: vec![scout],
            target: DESTINATION,
        },
    )]);
    let stop = sim.unit(scout).unwrap().pos;
    assert!(sim.move_target(scout).is_some());
    sim.step_with_commands([(PlayerId(0), Command::Stop { units: vec![scout] })]);
    for _ in 0..80 {
        sim.step();
    }
    assert_eq!(sim.unit(scout).unwrap().pos, stop);
    assert_eq!(sim.move_target(scout), None);
}
