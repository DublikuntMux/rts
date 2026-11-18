use rts_base::PlayerId;

#[test]
fn passive_ai_returns_no_commands() {
    let setup = rts_net::MatchSetup {
        rules: include_str!("../../../assets/data/core.rules.ron").into(),
        map: include_str!("../../../assets/maps/proving_grounds.map.ron").into(),
        seed: 0,
        players: 2,
    };
    let sim = setup.simulation().expect("valid setup");
    assert_eq!(rts_ai::commands(&sim, PlayerId(1)), []);
}

const FRONTIER: &str = include_str!("../../../assets/maps/frontier.map.ron");
const RULES: &str = include_str!("../../../assets/data/core.rules.ron");
const MAX_TEST_TICKS: u32 = 4_000;

fn frontier() -> rts_base::Sim {
    rts_net::MatchSetup {
        rules: RULES.into(),
        map: FRONTIER.into(),
        seed: 0,
        players: 2,
    }
    .simulation()
    .expect("valid Frontier setup")
}

fn run_frontier(commanders: &[PlayerId]) -> (rts_base::Sim, [bool; 2], [bool; 2]) {
    use rts_base::Command;

    let mut sim = frontier();
    let mut built = [false; 2];
    let mut deployed = [false; 2];
    while sim.outcome().is_none() && sim.tick() < MAX_TEST_TICKS {
        let mut turn = Vec::new();
        for &player in commanders {
            let slot = usize::from(player.0);
            built[slot] |= sim.units().any(|(_, unit)| {
                unit.owner == player && sim.rules().unit(unit.kind).id == "barracks"
            });
            let commands = rts_ai::commands(&sim, player);
            deployed[slot] |= commands
                .iter()
                .any(|command| matches!(command, Command::Attack { .. }));
            rts_net::validate_commands(&commands).expect("AI observes input limits");
            turn.extend(commands.into_iter().map(|command| (player, command)));
        }
        sim.step_with_commands(turn);
    }
    (sim, built, deployed)
}

#[test]
fn either_frontier_slot_can_win_a_skirmish_against_a_passive_opponent() {
    use rts_base::MatchOutcome;

    for player in [PlayerId(0), PlayerId(1)] {
        let (sim, built, deployed) = run_frontier(&[player]);
        let slot = usize::from(player.0);
        assert!(built[slot], "slot {slot} did not finish a barracks");
        assert!(deployed[slot], "slot {slot} did not deploy rifle squads");
        assert_eq!(sim.outcome(), Some(MatchOutcome::Winner(player)));
    }
}

#[test]
fn two_frontier_opponents_both_develop_and_reach_a_terminal_outcome() {
    let (sim, built, deployed) = run_frontier(&[PlayerId(0), PlayerId(1)]);
    assert_eq!(built, [true, true]);
    assert_eq!(deployed, [true, true]);
    assert!(
        sim.outcome().is_some(),
        "AI match exceeded {MAX_TEST_TICKS} ticks"
    );
}

#[test]
fn frontier_opponents_open_by_gathering_both_resources_near_home() {
    use rts_base::{Command, ResourceKind};

    let sim = frontier();
    let midpoint = sim.terrain().world_size().x / 2;
    for player in [PlayerId(0), PlayerId(1)] {
        let assignments = rts_ai::commands(&sim, player);
        let mut wood = false;
        let mut gold = false;
        for order in assignments {
            if let Command::Gather { deposit, .. } = order {
                let deposit = sim.deposit(deposit).unwrap();
                assert_eq!(deposit.pos.x < midpoint, player == PlayerId(0));
                match deposit.kind {
                    ResourceKind::Wood => wood = true,
                    ResourceKind::Gold => gold = true,
                }
            }
        }
        assert!(wood && gold);
    }
}
