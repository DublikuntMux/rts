use rts_base::{Command, PlayerId, PlayerKnowledge, TILE};
use rts_net::{MatchSetup, Recording, Replay, ViewReplay};

fn setup() -> MatchSetup {
    MatchSetup {
        rules: include_str!("../../../assets/data/core.rules.ron").to_owned(),
        map: include_str!("../../../assets/maps/frontier.map.ron").to_owned(),
        seed: 0,
        players: 2,
    }
}

fn record(player: PlayerId, ticks: u32) -> ViewReplay {
    let setup = setup();
    let mut sim = setup.simulation().unwrap();
    let mut knowledge = PlayerKnowledge::new(&sim, player);
    let mut replay =
        ViewReplay::new("Frontier".into(), setup.rules, knowledge.intro(&sim)).unwrap();
    replay.push(knowledge.observe(&sim)).unwrap();
    let workers: Vec<_> = sim
        .units()
        .filter(|(_, unit)| unit.owner == PlayerId(0) && unit.kind.0 == 0)
        .map(|(id, _)| id)
        .collect();
    for tick in 1..=ticks {
        let commands = (tick == 1).then(|| {
            (
                PlayerId(0),
                Command::Move {
                    units: workers.clone(),
                    target: rts_base::IVec2::new(17 * TILE, 12 * TILE),
                },
            )
        });
        sim.step_with_commands(commands);
        replay.push(knowledge.observe(&sim)).unwrap();
    }
    replay
}

#[test]
fn player_perspective_replays_roundtrip_and_replay_the_view() {
    let replay = record(PlayerId(0), 200);
    let text = Recording::View(replay.clone()).to_ron().unwrap();
    assert!(
        !text.contains("seed"),
        "the setup never enters a view replay"
    );
    let (decoded, sim) = Recording::from_ron(&text).unwrap();
    assert!(sim.is_none());
    assert_eq!(decoded, Recording::View(replay.clone()));
    assert_eq!(decoded.ticks(), 200);
    let view = replay.playback().unwrap();
    assert_eq!(view.tick(), 200);
    assert!(view.units().all(|(_, unit)| unit.owner == PlayerId(0)));
    // The workers walked east and explored beyond the base.
    assert!(view.terrain(rts_base::IVec2::new(16, 12)).is_some());
}

#[test]
fn corrupt_view_replays_are_rejected() {
    let replay = record(PlayerId(1), 20);
    for index in 0..4 {
        let mut bad = replay.clone();
        match index {
            0 => bad.frames[3].tick += 1,
            1 => bad.frames.swap(2, 3),
            2 => bad.version = 8,
            _ => bad.frames[0].units[0].unit.soldiers.push(1),
        }
        assert!(bad.playback().is_err(), "accepted corruption {index}");
        assert!(Recording::View(bad.clone()).to_ron().is_err());
        assert!(Recording::from_ron(&ron::to_string(&bad).unwrap()).is_err());
    }
}

#[test]
fn full_replays_still_load_as_recordings() {
    let mut replay = Replay::new(setup()).unwrap();
    let mut sim = setup().simulation().unwrap();
    for tick in 1..=5 {
        let turn = rts_net::Turn {
            tick,
            inputs: (0..2)
                .map(|player| rts_net::PlayerInput {
                    player: PlayerId(player),
                    commands: Vec::new(),
                })
                .collect(),
        };
        turn.apply(&mut sim).unwrap();
        replay.push(turn, sim.state_hash()).unwrap();
    }
    let (decoded, sim) = Recording::from_ron(&replay.to_ron().unwrap()).unwrap();
    assert_eq!(decoded, Recording::Match(replay));
    assert_eq!(sim.unwrap().tick(), 5);
}
