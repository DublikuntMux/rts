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
    assert!(
        !text.contains("fog_disabled"),
        "fogged recordings preserve the v9 intro layout"
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
fn open_view_replays_restore_the_server_fog_setting_and_owned_rallies() {
    let setup = setup();
    let mut sim = setup.simulation().unwrap();
    let player = PlayerId(0);
    let mut knowledge = PlayerKnowledge::with_fog(&sim, player, false);
    let mut replay =
        ViewReplay::new("Frontier".into(), setup.rules, knowledge.intro(&sim)).unwrap();
    replay.push(knowledge.observe(&sim)).unwrap();
    let target = rts_base::IVec2::new(9 * TILE, 12 * TILE);
    let command = Command::SetRallyPoint {
        units: vec![sim.headquarters(player).unwrap()],
        target,
    };
    let input = rts_net::ClientMessage::Input {
        tick: 1,
        commands: vec![command.clone()],
    };
    assert_eq!(
        rts_net::decode_client(&rts_net::encode_client(&input).unwrap()).unwrap(),
        input
    );
    sim.step_with_commands([(player, command)]);
    let frame = knowledge.observe(&sim);
    let update = rts_net::ServerMessage::View {
        frame: frame.clone(),
    };
    assert_eq!(
        rts_net::decode_server(&rts_net::encode_server(&update).unwrap()).unwrap(),
        update
    );
    replay.push(frame).unwrap();

    let text = Recording::View(replay.clone()).to_ron().unwrap();
    let (decoded, _) = Recording::from_ron(&text).unwrap();
    assert_eq!(decoded, Recording::View(replay.clone()));
    let view = replay.playback().unwrap();
    assert!(view.intro().fog_disabled);
    assert!(view.terrain_tiles().iter().all(Option::is_some));
    assert_eq!(view.rally_point(view.headquarters().unwrap()), Some(target));
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
    let mut sim = replay.simulation().unwrap();
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
