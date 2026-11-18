//! The opponent through its public interface: fog knowledge in, translated commands out.

use rts_ai::{Difficulty, Opponent};
use rts_base::{Command, MatchOutcome, PlayerId, PlayerKnowledge, PlayerView, ResourceKind, Sim};

const RULES: &str = include_str!("../../../assets/data/core.rules.ron");
const FRONTIER: &str = include_str!("../../../assets/maps/frontier.map.ron");
/// Thirty minutes of play.
const MAX_TEST_TICKS: u32 = 20 * 60 * 30;

fn simulation(map: &str) -> Sim {
    rts_net::MatchSetup {
        rules: RULES.into(),
        map: map.into(),
        seed: 0,
        players: 2,
    }
    .simulation()
    .map(|mut sim| {
        sim.enforce_fog();
        sim
    })
    .expect("valid setup")
}

struct Seat {
    player: PlayerId,
    knowledge: PlayerKnowledge,
    view: PlayerView,
    opponent: Opponent,
}

impl Seat {
    fn new(sim: &Sim, player: PlayerId, difficulty: Difficulty) -> Self {
        let (knowledge, view) = PlayerView::observe(sim, player);
        let opponent = Opponent::new(&view, difficulty);
        Self {
            player,
            knowledge,
            view,
            opponent,
        }
    }

    fn commands(&mut self, sim: &Sim) -> Vec<Command> {
        let commands = self.opponent.commands(&self.view);
        rts_net::validate_commands(&commands).expect("AI observes input limits");
        commands
            .iter()
            .filter_map(|command| self.knowledge.translate_command(sim, command))
            .collect()
    }
}

#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent milestones of one match"
)]
struct Progress {
    built: bool,
    trained_scout: bool,
    /// A trained scout was sent toward unexplored ground.
    scouted: bool,
    attacked: bool,
}

fn play(sim: &mut Sim, seats: &mut [Seat]) -> Vec<Progress> {
    let mut progress: Vec<_> = seats.iter().map(|_| Progress::default()).collect();
    while sim.outcome().is_none() && sim.tick() < MAX_TEST_TICKS {
        let mut turn = Vec::new();
        for (seat, progress) in seats.iter_mut().zip(&mut progress) {
            let commands = seat.commands(sim);
            let scout = sim.rules().unit_kind("scout");
            for command in &commands {
                match command {
                    Command::BuildBarracks { .. } => progress.built = true,
                    Command::TrainScout { .. } => progress.trained_scout = true,
                    Command::Attack { .. } => progress.attacked = true,
                    Command::Move { units, .. }
                        if units.iter().any(|&id| {
                            sim.unit(id).is_some_and(|unit| Some(unit.kind) == scout)
                        }) =>
                    {
                        progress.scouted = true;
                    }
                    _ => {}
                }
            }
            turn.extend(commands.into_iter().map(|command| (seat.player, command)));
        }
        sim.step_with_commands(turn);
        for seat in seats.iter_mut() {
            seat.view.apply(seat.knowledge.observe(sim)).unwrap();
        }
    }
    progress
}

#[test]
fn sandbox_maps_get_no_orders() {
    let sim = simulation(include_str!("../../../assets/maps/proving_grounds.map.ron"));
    for difficulty in Difficulty::ALL {
        let mut seat = Seat::new(&sim, PlayerId(1), difficulty);
        assert_eq!(seat.commands(&sim), []);
    }
}

#[test]
fn every_difficulty_finds_and_beats_a_passive_opponent_from_either_slot() {
    for difficulty in Difficulty::ALL {
        for player in [PlayerId(0), PlayerId(1)] {
            let mut sim = simulation(FRONTIER);
            let mut seats = [Seat::new(&sim, player, difficulty)];
            let [progress] = &play(&mut sim, &mut seats)[..] else {
                unreachable!()
            };
            assert!(
                progress.built && progress.trained_scout && progress.scouted && progress.attacked,
                "{difficulty:?} in slot {}",
                player.0
            );
            assert_eq!(
                sim.outcome(),
                Some(MatchOutcome::Winner(player)),
                "{difficulty:?} in slot {}",
                player.0
            );
        }
    }
}

#[test]
fn frontier_opponents_play_to_a_result() {
    for (first, second) in [
        (Difficulty::Hard, Difficulty::Easy),
        (Difficulty::Normal, Difficulty::Normal),
        (Difficulty::Easy, Difficulty::Hard),
    ] {
        let mut sim = simulation(FRONTIER);
        let mut seats = [
            Seat::new(&sim, PlayerId(0), first),
            Seat::new(&sim, PlayerId(1), second),
        ];
        let progress = play(&mut sim, &mut seats);
        assert!(
            progress
                .iter()
                .all(|seat| seat.built && seat.trained_scout && seat.scouted),
            "{first:?} against {second:?}"
        );
        let outcome = sim.outcome();
        assert!(
            outcome.is_some(),
            "{first:?} against {second:?} exceeded {MAX_TEST_TICKS} ticks"
        );
        if first != second {
            let hard = PlayerId(u8::from(second == Difficulty::Hard));
            assert_eq!(outcome, Some(MatchOutcome::Winner(hard)));
        }
    }
}

#[test]
fn frontier_opponents_open_by_gathering_both_resources_near_home() {
    let sim = simulation(FRONTIER);
    let midpoint = sim.terrain().world_size().x / 2;
    for player in [PlayerId(0), PlayerId(1)] {
        let mut seat = Seat::new(&sim, player, Difficulty::Normal);
        let mut wood = false;
        let mut gold = false;
        for order in seat.commands(&sim) {
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
