use rts_base::{IVec2, PlayerKnowledge, TILE, UnitId};

use super::*;

/// The public setup a server shares for `setup()`, and an authority to produce its frames.
fn online(session: &mut Session, player: PlayerId) -> (Match, Sim, PlayerKnowledge) {
    let lobby = LobbySetup {
        rules: setup().rules,
        map: "Proving Grounds".into(),
        players: 2,
    };
    let mut authority = setup().simulation().unwrap();
    authority.enforce_fog();
    let knowledge = PlayerKnowledge::new(&authority, player);
    let game = session
        .begin_online(&lobby, knowledge.intro(&authority), 0.0)
        .unwrap();
    (game, authority, knowledge)
}

/// Buffers authoritative frames as the network would.
fn receive_frames(
    session: &mut Session,
    authority: &mut Sim,
    knowledge: &mut PlayerKnowledge,
    ticks: u32,
) {
    for _ in 0..ticks {
        if session.received_tick.is_some() {
            authority.step();
        }
        let frame = knowledge.observe(authority);
        session.received_tick = Some(frame.tick);
        session.frames.insert(frame.tick, frame);
    }
}

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
    let mut game = session.begin(setup(), 0.0).unwrap();
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    let target = IVec2::new(5 * TILE, TILE * 10);
    game.queue_command(Command::Move {
        units: vec![UnitId(0)],
        target,
    });
    advance(&mut game, &mut session, &mut status, &mut network, 0.05).unwrap();
    assert!(game.view.move_target(UnitId(0)).is_none());
    advance(&mut game, &mut session, &mut status, &mut network, 0.10).unwrap();
    assert_eq!(game.view.move_target(UnitId(0)), Some(target));
    game.queue_command(Command::Stop {
        units: vec![UnitId(0)],
    });
    advance(&mut game, &mut session, &mut status, &mut network, 0.15).unwrap();
    assert!(game.view.move_target(UnitId(0)).is_some());
    advance(&mut game, &mut session, &mut status, &mut network, 0.20).unwrap();
    assert!(game.view.move_target(UnitId(0)).is_none());
    let replay = session.local_replay().unwrap();
    assert_eq!(replay.turns.len(), 4);
    assert_eq!(
        replay.playback().unwrap().state_hash(),
        game.local_sim().unwrap().state_hash()
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one scenario follows a skirmish through save, resume, result, restart, and playback"
)]
fn skirmish_save_resume_result_restart_and_playback_use_the_same_setup() {
    let mut app = headless();
    let mut setup = crate::skirmish::test_setup();
    setup.rules = setup.rules.replace("worker_ticks: 4", "worker_ticks: 120");
    app.world_mut().resource_mut::<MapCatalog>().0[0].setup = setup.clone();
    app.world_mut().resource_mut::<MapCatalog>().0[0].skirmish = true;

    let send = |app: &mut App, action| {
        app.world_mut().write_message(action);
        app.update();
        app.update();
    };
    send(
        &mut app,
        SessionAction::NewGame {
            map: 0,
            difficulty: rts_ai::Difficulty::Hard,
        },
    );
    assert!(app.world().resource::<Match>().view.is_skirmish());
    assert_eq!(
        app.world().resource::<Session>().difficulty,
        rts_ai::Difficulty::Hard
    );
    assert!(
        app.world_mut()
            .resource_mut::<Match>()
            .queue_command(Command::Gather {
                units: vec![UnitId(3)],
                deposit: rts_base::DepositId(0),
            })
    );
    for _ in 0..6 {
        app.world_mut().run_schedule(FixedUpdate);
    }
    assert!(
        app.world()
            .resource::<Session>()
            .local_replay()
            .unwrap()
            .turns
            .iter()
            .flat_map(|turn| &turn.inputs)
            .any(|input| input.player == PlayerId(1)
                && input.commands.iter().any(|command| matches!(
                    command,
                    Command::TrainWorker { units } if units == &[UnitId(1)]
                )))
    );
    let saved_hash = app
        .world()
        .resource::<Match>()
        .local_sim()
        .unwrap()
        .state_hash();
    send(&mut app, SessionAction::SaveGame);
    let saved = app.world().resource::<Library>().records(Shelf::Saves)[0]
        .id
        .clone();
    send(&mut app, SessionAction::Leave);
    app.world_mut().resource_mut::<Session>().difficulty = rts_ai::Difficulty::Easy;
    send(&mut app, SessionAction::LoadGame { id: saved });
    assert_eq!(
        app.world().resource::<Session>().difficulty,
        rts_ai::Difficulty::Hard,
        "a saved game resumes at its own difficulty"
    );
    assert_eq!(
        app.world()
            .resource::<Match>()
            .local_sim()
            .unwrap()
            .state_hash(),
        saved_hash
    );

    assert!(
        app.world_mut()
            .resource_mut::<Match>()
            .queue_command(Command::Attack {
                units: vec![UnitId(2)],
                target: UnitId(1),
            })
    );
    for _ in 0..100 {
        app.world_mut().run_schedule(FixedUpdate);
    }
    assert_eq!(status(&app).result, Some(MatchResult::Victory));
    assert!(app.world().resource::<PauseMenu>().open);
    let game = app.world().resource::<Match>();
    assert!(!game.accepts_orders);
    let victory_hash = game.local_sim().unwrap().state_hash();
    send(&mut app, SessionAction::SaveReplay);
    let replay = app.world().resource::<Library>().records(Shelf::Replays)[0]
        .id
        .clone();
    send(&mut app, SessionAction::Restart);
    app.update();
    assert_eq!(
        *app.world().resource::<State<AppState>>().get(),
        AppState::InGame
    );
    assert_eq!(status(&app).result, None);
    assert_eq!(
        app.world().resource::<Session>().difficulty,
        rts_ai::Difficulty::Hard
    );
    let game = app.world().resource::<Match>();
    assert!(game.accepts_orders);
    assert_eq!(
        game.local_sim().unwrap().state_hash(),
        Replay::new(setup)
            .unwrap()
            .simulation()
            .unwrap()
            .state_hash()
    );

    // PauseMenuPlugin owns clearing the overlay when state changes in the actual app.
    app.world_mut().resource_mut::<PauseMenu>().open = false;
    send(&mut app, SessionAction::Leave);
    send(&mut app, SessionAction::WatchReplay { id: replay });
    for _ in 0..100 {
        app.world_mut().run_schedule(FixedUpdate);
    }
    assert_eq!(status(&app).result, Some(MatchResult::Victory));
    assert!(status(&app).stopped);
    assert_eq!(
        app.world()
            .resource::<Match>()
            .local_sim()
            .unwrap()
            .state_hash(),
        victory_hash
    );
}

#[test]
fn playback_keeps_the_original_recording_available_after_finishing_and_leaving() {
    let replay = Recording::Match(Replay::new(setup()).unwrap());
    let mut session = Session::default();
    let mut game = session.begin_replay(replay.clone(), 0.0).unwrap();
    let mut status = SessionStatus::default();
    advance(
        &mut game,
        &mut session,
        &mut status,
        &mut Network::default(),
        0.05,
    )
    .unwrap();
    assert!(status.stopped);
    assert_eq!(session.replay(), Some(&replay));
    session.archive();
    assert_eq!(session.replay(), Some(&replay));
    let _ = session.begin(setup(), 0.1).unwrap();
    assert_eq!(session.last_replay.as_ref(), Some(&replay));
}

#[test]
fn absent_network_frame_stalls_without_consuming_input_or_changing_state() {
    let mut session = Session::default();
    let (mut game, mut authority, mut knowledge) = online(&mut session, PlayerId(0));
    receive_frames(&mut session, &mut authority, &mut knowledge, 1);
    session.submitted_tick = INPUT_DELAY_TICKS;
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    advance(&mut game, &mut session, &mut status, &mut network, 0.05).unwrap();
    assert_eq!(game.view.units().count(), 7);
    game.queue_command(Command::Stop {
        units: vec![UnitId(0)],
    });
    let pending = game.pending.clone();
    let units: Vec<_> = game
        .view
        .units()
        .map(|(id, unit)| (id, unit.clone()))
        .collect();
    for now in [0.1, 0.3, 0.4] {
        advance(&mut game, &mut session, &mut status, &mut network, now).unwrap();
        assert_eq!(game.view.tick(), 0);
        assert_eq!(game.pending, pending);
        let Some(Recording::View(replay)) = session.replay() else {
            panic!("online matches record the player's view")
        };
        assert_eq!(replay.frames.len(), 1);
    }
    assert_eq!(status.message, "Waiting for the server…");
    assert!(advance(&mut game, &mut session, &mut status, &mut network, 16.0).is_err());
    assert_eq!(
        game.view
            .units()
            .map(|(id, unit)| (id, unit.clone()))
            .collect::<Vec<_>>(),
        units
    );
    assert_eq!(game.pending, pending);
}

fn headless() -> App {
    use crate::{game::CatalogMap, storage::tests::temporary_store};

    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .insert_state(AppState::Menu)
        .insert_resource(Time::<()>::default())
        .insert_resource(MapCatalog(vec![CatalogMap {
            name: "Proving Grounds".into(),
            setup: setup(),
            preview: Handle::default(),
            skirmish: false,
        }]))
        .insert_resource(Library::new(temporary_store("session")))
        .init_resource::<Selection>()
        .add_plugins(SessionPlugin);
    app
}

fn status(app: &App) -> &SessionStatus {
    app.world().resource::<SessionStatus>()
}

#[test]
fn saved_solo_game_continues_from_its_recorded_state() {
    let mut app = headless();
    app.world_mut().write_message(SessionAction::NewGame {
        map: 0,
        difficulty: rts_ai::Difficulty::Normal,
    });
    app.update();
    app.update();
    let target = IVec2::new(5 * TILE, 10 * TILE);
    app.world_mut()
        .resource_mut::<Match>()
        .queue_command(Command::Move {
            units: vec![UnitId(0)],
            target,
        });
    for _ in 0..6 {
        app.world_mut().run_schedule(FixedUpdate);
    }
    let saved_hash = app
        .world()
        .resource::<Match>()
        .local_sim()
        .unwrap()
        .state_hash();
    let saved_tick = app.world().resource::<Match>().view.tick();
    assert_eq!(
        app.world().resource::<Match>().view.move_target(UnitId(0)),
        Some(target)
    );
    app.world_mut().write_message(SessionAction::SaveGame);
    app.update();
    assert_eq!(status(&app).message, "Saved as Save 01.");
    let record = app.world().resource::<Library>().records(Shelf::Saves)[0].clone();
    assert_eq!(record.meta.ticks, saved_tick);
    assert_eq!(record.meta.map, "Proving Grounds");

    // Saving is refused for playback and in the menu.
    app.world_mut().write_message(SessionAction::Leave);
    app.update();
    app.update();
    assert!(!status(&app).can_save_game());
    app.world_mut().write_message(SessionAction::SaveGame);
    app.update();
    assert!(status(&app).error);

    app.world_mut()
        .write_message(SessionAction::LoadGame { id: record.id });
    app.update();
    let game = app.world().resource::<Match>();
    assert_eq!(game.view.tick(), saved_tick);
    assert_eq!(game.local_sim().unwrap().state_hash(), saved_hash);
    assert!(game.accepts_orders);
    app.update();
    app.world_mut().run_schedule(FixedUpdate);
    let session = app.world().resource::<Session>();
    let replay = session.local_replay().unwrap();
    assert_eq!(replay.turns.len(), saved_tick as usize + 1);
    assert_eq!(
        replay.playback().unwrap().state_hash(),
        app.world()
            .resource::<Match>()
            .local_sim()
            .unwrap()
            .state_hash()
    );
}

#[tokio::test(flavor = "current_thread")]
#[expect(
    clippy::too_many_lines,
    reason = "one scripted real-socket match from the room list through private replays"
)]
async fn real_client_sessions_play_private_views_of_a_server_mod() {
    use std::time::Duration;

    let cert = rts_net::quic::ServerCertificate::generate_self_signed().unwrap();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "quic://{}#{}",
        socket.local_addr().unwrap(),
        cert.fingerprint
    );
    let mod_setup = MatchSetup {
        rules: setup()
            .rules
            .replace("\"worker\"", "\"mod_worker\"")
            .replace("speed: 2.5", "speed: 3.5")
            .replace(
                "radius: 0.3,",
                "radius: 0.3, visual_bounds: Some((offset: (0.125, -0.25), size: (0.75, 0.5))),",
            ),
        map: setup()
            .map
            .replace("\"worker\"", "\"mod_worker\"")
            .replace("Proving Grounds", "Server mod"),
        ..setup()
    };
    let config = rts_server::Config {
        setup: mod_setup.clone(),
        random_layout: false,
        ..default()
    };
    let endpoint = rts_net::quic::server_endpoint(socket, &cert, config.lobby_timeout).unwrap();
    let server = tokio::spawn(rts_server::serve_with_config(endpoint, config));
    let mut host = headless();
    let mut guest = headless();
    for app in [&mut host, &mut guest] {
        app.world_mut()
            .write_message(SessionAction::Connect { url: url.clone() });
    }
    let mut created = false;
    let mut joined = false;
    let mut host_ready = false;
    let mut guest_ready = false;
    let mut started = false;
    let mut moved = false;
    let mut stopped = false;
    for _ in 0..1000 {
        host.update();
        guest.update();
        if !created && status(&host).connected() {
            host.world_mut().write_message(SessionAction::CreateRoom {
                name: "Host".into(),
                public: true,
            });
            created = true;
        }
        if created && !joined && status(&guest).connected() {
            if let Some(room) = status(&guest).rooms.first().cloned() {
                assert_eq!(room.host, "Host");
                assert_eq!(room.map, "Server mod");
                assert_eq!((room.players, room.capacity), (1, 2));
                guest.world_mut().write_message(SessionAction::JoinRoom {
                    name: "Guest".into(),
                    code: room.code,
                });
                joined = true;
            } else {
                guest.world_mut().write_message(SessionAction::RefreshRooms);
            }
        }
        for (app, readied) in [(&mut host, &mut host_ready), (&mut guest, &mut guest_ready)] {
            if !*readied
                && status(app)
                    .lobby
                    .as_ref()
                    .is_some_and(|lobby| lobby.joined() == 2)
            {
                app.world_mut().write_message(SessionAction::SetReady(true));
                *readied = true;
            }
        }
        if !started && status(&host).lobby.as_ref().is_some_and(Lobby::can_start) {
            let lobby = status(&host).lobby.clone().unwrap();
            assert_eq!(lobby.host(), "Host");
            assert_eq!(lobby.slots[1].as_ref().unwrap().name, "Guest");
            host.world_mut().write_message(SessionAction::Start);
            started = true;
        }
        for app in [&mut host, &mut guest] {
            let tick = app
                .world()
                .get_resource::<Match>()
                .map_or(0, |game| game.view.tick());
            if tick < 40 {
                app.world_mut().run_schedule(FixedUpdate);
            }
        }
        if let Some(game) = host.world().get_resource::<Match>() {
            let tick = game.view.tick();
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
        let done = |app: &App| {
            app.world()
                .get_resource::<Match>()
                .is_some_and(|game| game.view.tick() >= 40)
        };
        if done(&host) && done(&guest) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(moved && stopped, "{:?}", status(&host));
    let game = host.world().resource::<Match>();
    let other = guest.world().resource::<Match>();
    assert!(game.view.tick() >= 40, "{}", status(&host).message);
    assert!(
        game.local_sim().is_none(),
        "online clients never hold the match"
    );
    assert_ne!(
        game.view.unit(UnitId(0)).unwrap().pos,
        IVec2::new(TILE * 9 / 2, TILE * 8)
    );
    assert!(
        game.view.move_target(UnitId(0)).is_none(),
        "Stop must be applied by the server"
    );
    // Opposite corners: each player sees only its own units and its own part of the map.
    for (view, player) in [(&game.view, PlayerId(0)), (&other.view, PlayerId(1))] {
        assert_eq!(view.player(), player);
        assert!(view.units().all(|(_, unit)| unit.owner == player));
        assert!(
            view.terrain_tiles()
                .iter()
                .filter(|tile| tile.is_none())
                .count()
                > view.terrain_tiles().len() / 2
        );
    }
    assert_ne!(game.view.start(), other.view.start());
    assert_eq!(
        game.view
            .rules()
            .unit(game.view.unit(UnitId(0)).unwrap().kind)
            .id,
        "mod_worker",
        "the server's rules mod reaches the client"
    );
    assert_eq!(status(&host).kind, MatchKind::Online);
    assert!(
        !status(&host).can_save_game(),
        "online matches are not saved games"
    );
    for app in [&host, &guest] {
        let Some(Recording::View(replay)) = app.world().resource::<Session>().replay() else {
            panic!("online matches record the player's own view")
        };
        assert_eq!(replay.map, "Server mod");
        let text = Recording::View(replay.clone()).to_ron().unwrap();
        assert!(!text.contains("seed") && !text.contains("tiles"));
        let view = replay.playback().unwrap();
        let live = &app.world().resource::<Match>().view;
        assert_eq!(view.tick(), live.tick());
        assert_eq!(view.terrain_tiles(), live.terrain_tiles());
        assert_eq!(
            view.units().collect::<Vec<_>>(),
            live.units().collect::<Vec<_>>()
        );
    }
    assert_eq!(
        host.world().resource::<MapCatalog>().0[0].setup,
        setup(),
        "solo configuration stays local"
    );
    guest.world_mut().write_message(SessionAction::Leave);
    guest.update();
    for _ in 0..200 {
        host.update();
        guest.update();
        if host.world().resource::<Session>().mode == Mode::Stopped {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(host.world().resource::<Session>().mode == Mode::Stopped);
    assert!(!host.world().resource::<Match>().accepts_orders);
    assert!(status(&host).stopped);
    // Both clients keep their connection and can browse rooms again.
    assert!(status(&host).connected());
    assert!(status(&guest).connected());
    assert!(!status(&guest).running);
    host.world_mut().write_message(SessionAction::SaveReplay);
    host.update();
    assert_eq!(status(&host).message, "Replay saved as Replay 01.");
    // The private replay plays back from the library.
    let id = host.world().resource::<Library>().records(Shelf::Replays)[0]
        .id
        .clone();
    host.world_mut().write_message(SessionAction::Leave);
    host.update();
    host.update();
    host.world_mut()
        .write_message(SessionAction::WatchReplay { id });
    host.update();
    host.update();
    for _ in 0..60 {
        host.world_mut().run_schedule(FixedUpdate);
    }
    assert!(status(&host).stopped);
    assert_eq!(status(&host).message, "Replay finished.");
    assert!(host.world().resource::<Match>().view.tick() >= 40);
    server.abort();
}

fn deliver(app: &mut App, message: ServerMessage) -> Result<Option<ClientMessage>, String> {
    use bevy::ecs::system::SystemState;

    let mut state = SystemState::<(
        ResMut<Session>,
        ResMut<SessionStatus>,
        Option<ResMut<Match>>,
        Commands,
        ResMut<NextState<AppState>>,
    )>::new(app.world_mut());
    let (mut session, mut status, mut game, mut commands, mut next) =
        state.get_mut(app.world_mut()).unwrap();
    let result = receive_message(
        message,
        &mut session,
        &mut status,
        game.as_deref_mut(),
        &mut commands,
        &mut next,
        0.0,
    );
    state.apply(app.world_mut());
    result
}

fn lobby_message(ready: [bool; 2]) -> ServerMessage {
    ServerMessage::Lobby {
        code: "AAAAAA".into(),
        player: PlayerId(0),
        public: true,
        slots: ["Host", "Guest"]
            .into_iter()
            .zip(ready)
            .map(|(name, ready)| {
                Some(LobbySlot {
                    name: name.into(),
                    ready,
                })
            })
            .collect(),
    }
}

#[test]
fn setup_acknowledges_server_content_and_start_requires_readiness() {
    let mut app = headless();
    app.world_mut().resource_mut::<SessionStatus>().joining = true;
    let mut authority = setup().simulation().unwrap();
    let mut knowledge = PlayerKnowledge::new(&authority, PlayerId(0));
    let started = ServerMessage::Started {
        players: 2,
        input_delay: INPUT_DELAY_TICKS,
        view: knowledge.intro(&authority),
    };
    assert!(deliver(&mut app, started.clone()).is_err());
    let server_setup = LobbySetup {
        rules: setup().rules.replace("max_health: 40", "max_health: 900"),
        map: "Proving Grounds".into(),
        players: 2,
    };
    assert_eq!(
        deliver(&mut app, ServerMessage::Setup(server_setup.clone())).unwrap(),
        Some(ClientMessage::Loaded {
            checksum: server_setup.checksum()
        })
    );
    deliver(&mut app, lobby_message([true, false])).unwrap();
    let lobby = status(&app).lobby.clone().unwrap();
    assert!(!status(&app).joining);
    assert!(lobby.is_host() && lobby.ready() && !lobby.can_start());
    assert_eq!(lobby.host(), "Host");
    deliver(&mut app, lobby_message([true, true])).unwrap();
    assert!(status(&app).lobby.as_ref().unwrap().can_start());
    deliver(&mut app, started).unwrap();
    assert!(status(&app).lobby.is_none());
    // Frames must arrive in order, starting at tick 0.
    authority.step();
    let early = knowledge.observe(&authority);
    let mut late = early;
    late.tick = 1;
    assert!(deliver(&mut app, ServerMessage::View { frame: late }).is_err());
    let mut app = headless();
    app.world_mut().resource_mut::<SessionStatus>().joining = true;
    deliver(&mut app, ServerMessage::Setup(server_setup.clone())).unwrap();
    deliver(&mut app, lobby_message([true, true])).unwrap();
    let authority = MatchSetup {
        rules: server_setup.rules.clone(),
        ..setup()
    }
    .simulation()
    .unwrap();
    let mut knowledge = PlayerKnowledge::new(&authority, PlayerId(0));
    deliver(
        &mut app,
        ServerMessage::Started {
            players: 2,
            input_delay: INPUT_DELAY_TICKS,
            view: knowledge.intro(&authority),
        },
    )
    .unwrap();
    deliver(
        &mut app,
        ServerMessage::View {
            frame: knowledge.observe(&authority),
        },
    )
    .unwrap();
    app.world_mut().resource_mut::<Session>().submitted_tick = INPUT_DELAY_TICKS;
    app.update();
    app.world_mut().run_schedule(FixedUpdate);
    let game = app.world().resource::<Match>();
    assert_eq!(game.view.unit(UnitId(0)).unwrap().health(), 900);
    let Some(Recording::View(replay)) = app.world().resource::<Session>().replay() else {
        panic!("expected a view replay")
    };
    assert_eq!(replay.rules, server_setup.rules);
    assert_eq!(replay.map, "Proving Grounds");
}

#[test]
fn closed_room_returns_to_the_room_list_and_stale_lobbies_are_ignored() {
    let mut app = headless();
    {
        let mut status = app.world_mut().resource_mut::<SessionStatus>();
        status.server = Server::Connected {
            url: "quic://host:9001".into(),
        };
    }
    // An update for a room this client never asked to join is ignored.
    deliver(&mut app, lobby_message([false, false])).unwrap();
    assert!(status(&app).lobby.is_none());
    app.world_mut().resource_mut::<SessionStatus>().joining = true;
    deliver(&mut app, lobby_message([false, false])).unwrap();
    assert!(status(&app).lobby.is_some());
    assert_eq!(
        deliver(
            &mut app,
            ServerMessage::Ended {
                reason: "a player left the room".into()
            }
        )
        .unwrap(),
        Some(ClientMessage::Rooms {
            version: PROTOCOL_VERSION
        })
    );
    assert!(status(&app).lobby.is_none());
    assert!(status(&app).connected());
    assert_eq!(
        status(&app).message,
        "The room closed: a player left the room."
    );
    assert!(status(&app).error);
}

#[test]
fn refusal_while_connecting_fails_the_connection_and_later_errors_do_not() {
    let mut app = headless();
    app.world_mut().resource_mut::<SessionStatus>().server = Server::Connecting {
        url: "quic://host:9001".into(),
    };
    assert!(
        deliver(
            &mut app,
            ServerMessage::Error {
                message: "protocol version does not match".into()
            }
        )
        .is_err()
    );
    deliver(&mut app, ServerMessage::Rooms { rooms: Vec::new() }).unwrap();
    assert!(status(&app).connected());
    app.world_mut().resource_mut::<SessionStatus>().joining = true;
    deliver(
        &mut app,
        ServerMessage::Error {
            message: "room is full".into(),
        },
    )
    .unwrap();
    assert!(!status(&app).joining);
    assert_eq!(status(&app).message, "Room is full.");
}

#[test]
fn invalid_server_frame_is_rejected_before_recording() {
    let mut session = Session::default();
    let (mut game, mut authority, mut knowledge) = online(&mut session, PlayerId(0));
    receive_frames(&mut session, &mut authority, &mut knowledge, 1);
    let mut bad = session.frames[&0].clone();
    bad.removed.push(UnitId(500));
    session.frames.insert(0, bad);
    session.submitted_tick = INPUT_DELAY_TICKS;
    let error = advance(
        &mut game,
        &mut session,
        &mut SessionStatus::default(),
        &mut Network::default(),
        0.05,
    )
    .unwrap_err();
    assert!(error.contains("Invalid server view"), "{error}");
    let Some(Recording::View(replay)) = session.replay() else {
        panic!("expected a view replay")
    };
    assert_eq!(replay.frames, []);
}

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
fn online_backlog_catches_up_with_bounded_work_and_a_private_replay() {
    let mut session = Session::default();
    let (mut game, mut authority, mut knowledge) = online(&mut session, PlayerId(1));
    receive_frames(&mut session, &mut authority, &mut knowledge, 21);
    session.submitted_tick = 22;
    let mut status = SessionStatus::default();
    let mut network = Network::default();
    advance_buffered(&mut game, &mut session, &mut status, &mut network, 1.0).unwrap();
    assert_eq!(
        game.view.tick(),
        7,
        "one update has a fixed catch-up budget"
    );
    for _ in 0..8 {
        advance_buffered(&mut game, &mut session, &mut status, &mut network, 1.0).unwrap();
    }
    assert!(session.frames.is_empty());
    assert_eq!(game.view.tick(), 20);
    assert!(game.view.units().all(|(_, unit)| unit.owner == PlayerId(1)));
    let Some(Recording::View(replay)) = session.replay() else {
        panic!("expected a view replay")
    };
    assert_eq!(replay.ticks(), 20);
    assert_eq!(
        replay.playback().unwrap().units().collect::<Vec<_>>(),
        game.view.units().collect::<Vec<_>>()
    );
}

#[test]
fn message_decoder_accepts_compact_binary_server_frames() {
    use rts_net::encode_server;

    let lobby = LobbySetup {
        rules: setup().rules,
        map: "Proving Grounds".into(),
        players: 2,
    };
    let bytes = encode_server(&ServerMessage::Setup(lobby.clone())).unwrap();
    assert_eq!(
        decode_message(&bytes).unwrap(),
        ServerMessage::Setup(lobby.minified())
    );
    let sim = setup().simulation().unwrap();
    let mut knowledge = PlayerKnowledge::new(&sim, PlayerId(0));
    for message in [
        ServerMessage::Started {
            players: 2,
            input_delay: INPUT_DELAY_TICKS,
            view: knowledge.intro(&sim),
        },
        ServerMessage::View {
            frame: knowledge.observe(&sim),
        },
    ] {
        let bytes = encode_server(&message).unwrap();
        assert_eq!(decode_message(&bytes).unwrap(), message);
    }
    assert!(decode_message(b"Setup(())").is_err());
}

#[test]
fn pause_freezes_solo_and_replay_ticks_but_online_still_applies_authoritative_frames() {
    for mode in [Mode::Offline, Mode::Playback, Mode::Online] {
        let mut session = Session::default();
        let mut game = session.begin(setup(), 0.0).unwrap();
        if mode == Mode::Playback {
            let mut reference = setup().simulation().unwrap();
            reference.enforce_fog();
            let turn = Turn {
                tick: 1,
                inputs: (0..2)
                    .map(|player| PlayerInput {
                        player: PlayerId(player),
                        commands: vec![],
                    })
                    .collect(),
            };
            turn.apply(&mut reference).unwrap();
            let mut replay = Replay::new(setup()).unwrap();
            replay.push(turn, reference.state_hash()).unwrap();
            game = session.begin_replay(Recording::Match(replay), 0.0).unwrap();
        } else if mode == Mode::Online {
            let (online_game, mut authority, mut knowledge) = online(&mut session, PlayerId(0));
            game = online_game;
            // No socket is needed for already-buffered frames with no new input submission.
            receive_frames(&mut session, &mut authority, &mut knowledge, 2);
            session.submitted_tick = 3;
        }
        game.pending.push(Command::Stop {
            units: vec![UnitId(0)],
        });
        let mut app = App::new();
        app.insert_resource(game)
            .insert_resource(session)
            .init_resource::<SessionStatus>()
            .insert_resource(PauseMenu { open: true })
            .insert_non_send(Network::default())
            .insert_resource(Time::<()>::default())
            .add_systems(FixedUpdate, advance_session);
        app.world_mut().run_schedule(FixedUpdate);
        if mode == Mode::Online {
            assert_eq!(app.world().resource::<Session>().applied_frames, 2);
        } else {
            assert_eq!(app.world().resource::<Match>().view.tick(), 0);
            assert_eq!(app.world().resource::<Match>().pending.len(), 1);
            app.world_mut().resource_mut::<PauseMenu>().open = false;
            app.world_mut().run_schedule(FixedUpdate);
        }
        assert_eq!(app.world().resource::<Match>().view.tick(), 1);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn client_syncs_server_textures_and_translations_on_connect_and_clears_on_disconnect() {
    let cert = rts_net::quic::ServerCertificate::generate_self_signed().unwrap();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "quic://{}#{}",
        socket.local_addr().unwrap(),
        cert.fingerprint
    );
    let png = include_bytes!("../../../../assets/textures/terrain/wood.png").to_vec();
    let en_overlay =
        br#"(code: "en", name: "English", strings: {"unit.soldier": "Laser Trooper"})"#.to_vec();
    let fr_locale = r#"(code: "fr", name: "Français", strings: {"nav.quit": "Quitter"})"#
        .as_bytes()
        .to_vec();
    let mut assets = BTreeMap::new();
    assets.insert(
        "textures/units/laser.png".to_owned(),
        rts_server::ServerAsset::from_bytes("textures/units/laser.png", png).unwrap(),
    );
    assets.insert(
        "locales/en.lang.ron".to_owned(),
        rts_server::ServerAsset::from_bytes("locales/en.lang.ron", en_overlay).unwrap(),
    );
    assets.insert(
        "locales/fr.lang.ron".to_owned(),
        rts_server::ServerAsset::from_bytes("locales/fr.lang.ron", fr_locale).unwrap(),
    );
    let config = rts_server::Config {
        assets,
        ..default()
    };
    let endpoint = rts_net::quic::server_endpoint(socket, &cert, config.lobby_timeout).unwrap();
    let server = tokio::spawn(rts_server::serve_with_config(endpoint, config));

    let mut client = headless();
    client.insert_resource(Assets::<Image>::default());
    client
        .world_mut()
        .write_message(SessionAction::Connect { url });
    for _ in 0..400 {
        client.update();
        if status(&client).connected() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(status(&client).connected());
    assert!(
        client
            .world()
            .resource::<ServerAssets>()
            .image("textures/units/laser.png")
            .is_some()
    );
    assert_eq!(
        client
            .world()
            .resource::<Localization>()
            .unit_name("soldier"),
        "Laser Trooper"
    );
    assert!(
        client
            .world()
            .resource::<Localization>()
            .languages()
            .iter()
            .any(|(code, name)| code == "fr" && name == "Français")
    );

    client.world_mut().write_message(SessionAction::Disconnect);
    client.update();
    assert!(
        client
            .world()
            .resource::<ServerAssets>()
            .image("textures/units/laser.png")
            .is_none()
    );
    assert_eq!(
        client
            .world()
            .resource::<Localization>()
            .unit_name("soldier"),
        "Rifle squad"
    );
    assert!(
        !client
            .world()
            .resource::<Localization>()
            .languages()
            .iter()
            .any(|(code, _)| code == "fr")
    );
    server.abort();
}
