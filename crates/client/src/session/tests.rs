use rts_base::{IVec2, TILE, UnitId};

use super::*;

fn setup() -> MatchSetup {
    MatchSetup {
        rules: include_str!("../../../../assets/data/core.rules.ron").into(),
        map: include_str!("../../../../assets/maps/proving_grounds.map.ron").into(),
        seed: 0,
        players: 2,
    }
}

#[test]
fn local_input_delay_and_replay_share_the_same_turns() {
    let mut session = Session::default();
    let mut game = session
        .begin(setup(), Mode::Offline, PlayerId(0), 0.0)
        .unwrap();
    let mut previous = PreviousPositions::default();
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    let target = IVec2::new(5 * TILE, TILE * 10);
    game.queue_command(Command::Move {
        units: vec![UnitId(0)],
        target,
    });
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.05,
    )
    .unwrap();
    assert!(game.sim.move_target(UnitId(0)).is_none());
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.10,
    )
    .unwrap();
    assert_eq!(game.sim.move_target(UnitId(0)), Some(target));
    game.queue_command(Command::Stop {
        units: vec![UnitId(0)],
    });
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.15,
    )
    .unwrap();
    assert!(game.sim.move_target(UnitId(0)).is_some());
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.20,
    )
    .unwrap();
    assert!(game.sim.move_target(UnitId(0)).is_none());
    let replay = session.replay().unwrap();
    assert_eq!(replay.turns.len(), 4);
    assert_eq!(
        replay.playback().unwrap().state_hash(),
        game.sim.state_hash()
    );
}

#[test]
fn absent_network_turn_stalls_without_consuming_input_or_repeating_interpolation() {
    let mut session = Session::default();
    let mut game = session
        .begin(setup(), Mode::Online, PlayerId(0), 0.0)
        .unwrap();
    game.queue_command(Command::Stop {
        units: vec![UnitId(0)],
    });
    let mut previous = PreviousPositions(BTreeMap::from([(UnitId(0), IVec2::ZERO)]));
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    let hash = game.sim.state_hash();
    advance(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        0.1,
    )
    .unwrap();
    assert_eq!(game.sim.state_hash(), hash);
    assert_eq!(game.pending.len(), 1);
    assert!(previous.0.is_empty());
    assert!(
        advance(
            &mut game,
            &mut previous,
            &mut session,
            &mut status,
            &mut network,
            16.0
        )
        .is_err()
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn headless() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .insert_state(AppState::Menu)
        .insert_resource(Time::<()>::default())
        .insert_resource(MatchData(setup()))
        .init_resource::<Selection>()
        .init_resource::<PreviousPositions>()
        .add_plugins(SessionPlugin);
    app
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn completed_replay_load_cannot_replace_a_match_starting_this_frame() {
    let mut app = headless();
    let text = Replay::new(setup()).unwrap().to_ron().unwrap();
    app.world().resource::<ReplayIo>().complete_for_test(text);
    app.world_mut().write_message(SessionAction::Solo);
    app.update();
    assert!(app.world().resource::<Session>().mode == Mode::Offline);
    assert!(app.world().resource::<Match>().accepts_orders);
    assert!(
        app.world()
            .resource::<SessionStatus>()
            .message
            .contains("Leave the current match")
    );
    app.update();
    assert_eq!(
        *app.world().resource::<State<AppState>>().get(),
        AppState::InGame
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test(flavor = "current_thread")]
#[expect(
    clippy::too_many_lines,
    reason = "one scripted real-socket match from lobby through replay verification and disconnect"
)]
async fn real_client_sessions_use_server_mods_verify_turns_and_replays() {
    use std::time::Duration;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let mod_setup = MatchSetup {
        rules: setup()
            .rules
            .replace("worker", "mod_worker")
            .replace("speed: 2.5", "speed: 3.5")
            .replace(
                "radius: 0.3,",
                "radius: 0.3, visual_bounds: Some((offset: (0.125, -0.25), size: (0.75, 0.5))),",
            ),
        map: setup()
            .map
            .replace("worker", "mod_worker")
            .replace("Proving Grounds", "Server mod"),
        ..setup()
    };
    assert_ne!(mod_setup.checksum(), setup().checksum());
    let initial_hash = mod_setup.simulation().unwrap().state_hash();
    let config = rts_server::Config {
        setup: mod_setup.clone(),
        ..default()
    };
    let server = tokio::spawn(rts_server::serve_with_config(listener, config));
    let mut host = headless();
    let mut guest = headless();
    host.world_mut()
        .write_message(SessionAction::Host { url: url.clone() });
    let mut joined = false;
    let mut started = false;
    let mut moved = false;
    let mut stopped = false;
    let mut host_hashes = BTreeMap::new();
    let mut guest_hashes = BTreeMap::new();
    host_hashes.insert(0, initial_hash);
    guest_hashes.insert(0, initial_hash);
    let mut host_audit = mod_setup.simulation().unwrap();
    let mut guest_audit = mod_setup.simulation().unwrap();
    for _ in 0..1000 {
        host.update();
        guest.update();
        if !joined && let Some(code) = host.world().resource::<SessionStatus>().room_code.clone() {
            guest.world_mut().write_message(SessionAction::Join {
                url: url.clone(),
                code,
            });
            joined = true;
        }
        if !started && host.world().resource::<SessionStatus>().can_start {
            host.world_mut().write_message(SessionAction::Start);
            started = true;
        }
        for app in [&mut host, &mut guest] {
            let tick = app
                .world()
                .get_resource::<Match>()
                .map_or(0, |game| game.sim.tick());
            if tick < 40 {
                app.world_mut().run_schedule(FixedUpdate);
            }
        }
        if let Some(game) = host.world().get_resource::<Match>() {
            let tick = game.sim.tick();
            if tick >= 3 && !moved {
                host.world_mut()
                    .resource_mut::<Match>()
                    .queue_command(Command::Move {
                        units: vec![UnitId(0)],
                        target: IVec2::new(TILE * 5, TILE * 10),
                    });
                moved = true;
            }
            if tick >= 18 && !stopped {
                host.world_mut()
                    .resource_mut::<Match>()
                    .queue_command(Command::Stop {
                        units: vec![UnitId(0)],
                    });
                stopped = true;
            }
        }
        for (app, audit, hashes) in [
            (&host, &mut host_audit, &mut host_hashes),
            (&guest, &mut guest_audit, &mut guest_hashes),
        ] {
            if let Some(replay) = app.world().resource::<Session>().recording.as_ref() {
                for turn in replay.turns.iter().skip(audit.tick() as usize) {
                    turn.apply(audit).unwrap();
                    hashes.insert(audit.tick(), audit.state_hash());
                }
            }
        }
        if host_hashes.contains_key(&40) && guest_hashes.contains_key(&40) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(moved && stopped);
    for tick in 0..=40 {
        assert_eq!(
            host_hashes.get(&tick),
            guest_hashes.get(&tick),
            "tick {tick}"
        );
    }
    let game = host.world().resource::<Match>();
    assert!(
        game.sim.tick() >= 40,
        "{}",
        host.world().resource::<SessionStatus>().message
    );
    assert_ne!(
        game.sim.unit(UnitId(0)).unwrap().pos,
        IVec2::new(TILE * 9 / 2, TILE * 8)
    );
    assert!(
        game.sim.move_target(UnitId(0)).is_none(),
        "Stop must be applied by the server"
    );
    let replay = host.world().resource::<Session>().replay().unwrap();
    assert_eq!(replay.setup, mod_setup.minified());
    assert_eq!(
        guest.world().resource::<Session>().replay().unwrap().setup,
        mod_setup.minified()
    );
    assert_eq!(
        host.world().resource::<MatchData>().0,
        setup(),
        "solo configuration stays local"
    );
    let text = replay.to_ron().unwrap();
    assert_eq!(
        Replay::from_ron(&text)
            .unwrap()
            .playback()
            .unwrap()
            .state_hash(),
        game.sim.state_hash()
    );
    guest.world_mut().write_message(SessionAction::Leave);
    guest.update();
    for _ in 0..200 {
        host.update();
        if host.world().resource::<Session>().mode == Mode::Stopped {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(host.world().resource::<Session>().mode == Mode::Stopped);
    assert!(!host.world().resource::<Match>().accepts_orders);
    assert!(host.world().resource::<SessionStatus>().room_code.is_some());
    server.abort();
}

#[cfg(not(target_arch = "wasm32"))]
fn deliver(app: &mut App, message: ServerMessage) -> Result<Option<ClientMessage>, String> {
    use bevy::ecs::system::SystemState;

    let mut state = SystemState::<(
        ResMut<Session>,
        ResMut<SessionStatus>,
        Commands,
        ResMut<NextState<AppState>>,
    )>::new(app.world_mut());
    let (mut session, mut status, mut commands, mut next) = state.get_mut(app.world_mut()).unwrap();
    let result = receive_message(
        message,
        &mut session,
        &mut status,
        &mut commands,
        &mut next,
        0.0,
    );
    state.apply(app.world_mut());
    result
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn setup_acknowledges_server_content_and_start_requires_it() {
    let mut app = headless();
    assert!(
        deliver(
            &mut app,
            ServerMessage::Started {
                players: 2,
                input_delay: INPUT_DELAY_TICKS
            }
        )
        .is_err()
    );
    let server_setup = MatchSetup {
        rules: setup().rules.replace("max_health: 40", "max_health: 900"),
        ..setup()
    };
    assert_eq!(
        deliver(&mut app, ServerMessage::Setup(server_setup.clone())).unwrap(),
        Some(ClientMessage::Ready {
            checksum: server_setup.checksum()
        })
    );
    deliver(
        &mut app,
        ServerMessage::Lobby {
            code: "AAAAAA".into(),
            player: PlayerId(0),
            players: 2,
            capacity: 2,
            ready: 1,
        },
    )
    .unwrap();
    assert!(!app.world().resource::<SessionStatus>().can_start);
    deliver(
        &mut app,
        ServerMessage::Lobby {
            code: "AAAAAA".into(),
            player: PlayerId(0),
            players: 2,
            capacity: 2,
            ready: 2,
        },
    )
    .unwrap();
    assert!(app.world().resource::<SessionStatus>().can_start);
    deliver(
        &mut app,
        ServerMessage::Started {
            players: 2,
            input_delay: INPUT_DELAY_TICKS,
        },
    )
    .unwrap();
    assert_eq!(
        app.world()
            .resource::<Match>()
            .sim
            .unit(UnitId(0))
            .unwrap()
            .health,
        900
    );
    assert_eq!(
        app.world().resource::<Session>().replay().unwrap().setup,
        server_setup
    );
}

#[test]
fn divergent_server_turn_is_rejected_before_recording() {
    let mut session = Session::default();
    let mut game = session
        .begin(setup(), Mode::Online, PlayerId(0), 0.0)
        .unwrap();
    let turn = Turn {
        tick: 1,
        inputs: (0..2)
            .map(|player| PlayerInput {
                player: PlayerId(player),
                commands: vec![],
            })
            .collect(),
    };
    let mut authority = setup().simulation().unwrap();
    turn.apply(&mut authority).unwrap();
    session.turns.insert(1, (turn, authority.state_hash() ^ 1));
    session.received_tick = 1;
    session.submitted_tick = 3;
    let error = advance(
        &mut game,
        &mut PreviousPositions::default(),
        &mut session,
        &mut SessionStatus::default(),
        &mut Network::default(),
        0.05,
    )
    .unwrap_err();
    assert!(error.contains("authoritative server"));
    assert_eq!(session.replay().unwrap().turns, []);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rejected_request_does_not_terminate_an_authoritative_match() {
    let mut app = headless();
    app.world_mut().resource_mut::<Session>().mode = Mode::Online;
    app.world_mut().resource_mut::<SessionStatus>().running = true;
    deliver(
        &mut app,
        ServerMessage::Error {
            message: "input arrived too late".into(),
        },
    )
    .unwrap();
    assert!(app.world().resource::<Session>().mode == Mode::Online);
    assert!(app.world().resource::<SessionStatus>().running);
}

#[test]
fn online_backlog_catches_up_with_bounded_work_and_verified_replay() {
    let mut session = Session::default();
    let mut game = session
        .begin(setup(), Mode::Online, PlayerId(0), 0.0)
        .unwrap();
    let mut authority = setup().simulation().unwrap();
    for tick in 1..=20 {
        let turn = Turn {
            tick,
            inputs: (0..2)
                .map(|player| PlayerInput {
                    player: PlayerId(player),
                    commands: vec![],
                })
                .collect(),
        };
        turn.apply(&mut authority).unwrap();
        session.turns.insert(tick, (turn, authority.state_hash()));
    }
    session.received_tick = 20;
    session.submitted_tick = 22;
    let mut previous = PreviousPositions::default();
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    advance_buffered(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        1.0,
    )
    .unwrap();
    assert_eq!(game.sim.tick(), 8, "one update has a fixed catch-up budget");
    for _ in 0..2 {
        advance_buffered(
            &mut game,
            &mut previous,
            &mut session,
            &mut status,
            &mut network,
            1.0,
        )
        .unwrap();
    }
    assert!(session.turns.is_empty());
    assert_eq!(game.sim.state_hash(), authority.state_hash());
    assert_eq!(
        session.replay().unwrap().playback().unwrap().state_hash(),
        authority.state_hash()
    );
}

#[test]
fn websocket_message_decoder_accepts_binary_setup_and_text_turns() {
    use rts_net::{ServerFrame, encode_server};

    let ServerFrame::Binary(bytes) = encode_server(&ServerMessage::Setup(setup())).unwrap() else {
        panic!("setup must be binary")
    };
    assert_eq!(
        decode_message(WsMessage::Binary(bytes)).unwrap(),
        Some(ServerMessage::Setup(setup().minified()))
    );
    let message = ServerMessage::Started {
        players: 2,
        input_delay: INPUT_DELAY_TICKS,
    };
    let ServerFrame::Text(text) = encode_server(&message).unwrap() else {
        panic!("gameplay messages must be text")
    };
    assert_eq!(
        decode_message(WsMessage::Text(text.clone())).unwrap(),
        Some(message)
    );
    assert!(decode_message(WsMessage::Binary(text.into_bytes())).is_err());
    assert!(decode_message(WsMessage::Text("Setup(())".into())).is_err());
    assert_eq!(decode_message(WsMessage::Ping(vec![])).unwrap(), None);
}
