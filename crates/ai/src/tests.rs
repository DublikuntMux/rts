use rts_base::{
    Command, MapData, PlayerId, PlayerKnowledge, PlayerView, Rules, Sim, UnitId, tile_of,
};

use crate::{Difficulty, Opponent, distance_squared};

const RULES: &str = include_str!("../../../assets/data/core.rules.ron");
const FRONTIER: &str = include_str!("../../../assets/maps/frontier.map.ron");

fn simulation(map: &str) -> Sim {
    let mut sim = Sim::new(
        Rules::from_ron(RULES).unwrap(),
        &MapData::from_ron(map).unwrap(),
    )
    .unwrap();
    sim.enforce_fog();
    sim
}

/// One AI player: its fog knowledge on the authority side, its view, and its opponent.
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

    /// Decides in view IDs and translates for the authority, like the live session.
    fn decide(&mut self, sim: &Sim) -> (Vec<Command>, Vec<(PlayerId, Command)>) {
        let orders = self.opponent.commands(&self.view);
        let translated = orders
            .iter()
            .filter_map(|order| self.knowledge.translate_command(sim, order))
            .map(|order| (self.player, order))
            .collect();
        (orders, translated)
    }

    fn observe(&mut self, sim: &Sim) {
        self.view.apply(self.knowledge.observe(sim)).unwrap();
    }
}

fn is_scout(view: &PlayerView, id: UnitId) -> bool {
    view.unit(id)
        .is_some_and(|unit| Some(unit.kind) == view.rules().unit_kind("scout"))
}

/// Frontier with changes player 1 cannot see from its base: player 0's headquarters and start,
/// a deposit stock near player 0, and terrain in the far corner.
fn hidden_variant() -> String {
    let variant = FRONTIER
        .replacen("(start: (5.5, 11.5))", "(start: (4.5, 11.5))", 1)
        .replacen(
            "(kind: \"headquarters\", owner: 0, pos: (5.5, 11.5))",
            "(kind: \"headquarters\", owner: 0, pos: (4.5, 11.5))",
            1,
        )
        .replacen(
            "(kind: Wood, pos: (3.5, 8.5), remaining: 1000)",
            "(kind: Wood, pos: (3.5, 8.5), remaining: 400)",
            1,
        )
        .replacen("\":...........:..:~~~:", "\"............:..:~~~:", 1);
    assert_ne!(variant, FRONTIER);
    variant
}

#[test]
fn hidden_world_differences_do_not_change_decisions() {
    for difficulty in Difficulty::ALL {
        let mut worlds = [simulation(FRONTIER), simulation(&hidden_variant())];
        assert_ne!(worlds[0].state_hash(), worlds[1].state_hash());
        let mut seats = worlds
            .each_ref()
            .map(|sim| Seat::new(sim, PlayerId(1), difficulty));
        let mut scouted = false;
        let mut trained_scout = false;
        let mut same_ticks = 0;
        for _ in 0..4_000 {
            let [(orders, translated), (other, translated_other)] =
                [0, 1].map(|world| seats[world].decide(&worlds[world]));
            assert_eq!(
                orders,
                other,
                "{difficulty:?} decided differently on tick {}",
                worlds[0].tick()
            );
            trained_scout |= orders
                .iter()
                .any(|order| matches!(order, Command::TrainScout { .. }));
            scouted |= orders.iter().any(|order| {
                matches!(order, Command::Move { units, .. }
                    if units.iter().any(|&id| is_scout(&seats[0].view, id)))
            });
            worlds[0].step_with_commands(translated);
            worlds[1].step_with_commands(translated_other);
            let frames = [0, 1].map(|world| seats[world].knowledge.observe(&worlds[world]));
            if frames[0] != frames[1] {
                break;
            }
            for (seat, frame) in seats.iter_mut().zip(frames) {
                seat.view.apply(frame).unwrap();
            }
            same_ticks += 1;
        }
        assert!(
            trained_scout && scouted,
            "{difficulty:?} sent no scout in {same_ticks} ticks of identical knowledge"
        );
        assert!(
            same_ticks < 4_000,
            "{difficulty:?} never discovered the hidden differences"
        );
    }
}

#[test]
fn difficulties_differ_in_cadence_scouting_and_attack_timing() {
    let enemy = tile_of(simulation(FRONTIER).players()[0].start);
    let mut first_scout_targets = Vec::new();
    for difficulty in Difficulty::ALL {
        let mut sim = simulation(FRONTIER);
        let mut seat = Seat::new(&sim, PlayerId(1), difficulty);
        let mut first_scout = None;
        let mut first_attack = None;
        for _ in 0..6_000 {
            let tick = sim.tick();
            let (orders, translated) = seat.decide(&sim);
            if !orders.is_empty() {
                assert_eq!(tick % difficulty.cadence(), 0, "{difficulty:?} off cadence");
            }
            for order in &orders {
                match order {
                    Command::Move { units, target } if is_scout(&seat.view, units[0]) => {
                        first_scout.get_or_insert_with(|| tile_of(*target));
                    }
                    Command::Move { .. } | Command::Attack { .. } if seat.opponent.attacking => {
                        first_attack.get_or_insert(tick);
                    }
                    _ => {}
                }
            }
            sim.step_with_commands(translated);
            seat.observe(&sim);
        }
        let first_attack = first_attack.expect("an attack against a passive enemy");
        assert!(first_attack >= difficulty.first_attack_tick());
        first_scout_targets.push((difficulty, first_scout.expect("a scout"), first_attack));
    }
    let [
        (_, easy, easy_attack),
        (_, normal, normal_attack),
        (_, hard, hard_attack),
    ] = first_scout_targets[..]
    else {
        unreachable!()
    };
    assert!(easy != normal && normal != hard && easy != hard);
    let toward_enemy = |tile| distance_squared(tile, enemy);
    assert!(toward_enemy(hard) < toward_enemy(normal));
    assert!(toward_enemy(normal) < toward_enemy(easy));
    assert!(hard_attack < normal_attack && normal_attack < easy_attack);
}
