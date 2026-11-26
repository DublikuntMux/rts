use std::time::Duration;

use rand::{SeedableRng, rngs::StdRng};
use rts_base::{Command, IVec2, PlayerId, PlayerView, TILE, UnitId, ViewFrame};
use rts_net::{
    ClientMessage, MAX_COMMANDS_PER_TURN, MAX_UNITS_PER_COMMAND, MatchSetup, PROTOCOL_VERSION,
    ServerMessage, encode_server,
};
use rts_server::{Config, Coordinator, Delivery, MAX_CATCH_UP_TURNS, MAX_FUTURE_TICKS};

const NOW: Duration = Duration::ZERO;

/// A coordinator with reproducible room codes.
fn coordinator(config: Config) -> Coordinator {
    Coordinator::new(config, StdRng::seed_from_u64(0))
}

/// A server that plays one public map.
fn fixed(setup: MatchSetup) -> Config {
    Config {
        setup,
        random_layout: false,
        ..Config::default()
    }
}

fn frame(delivery: &Delivery) -> &ViewFrame {
    let ServerMessage::View { frame } = &delivery.message else {
        panic!("expected a view frame, got {:?}", delivery.message)
    };
    frame
}

fn setup() -> MatchSetup {
    MatchSetup {
        rules: Config::default().setup.rules,
        map: r#"(name: "server", players: [(start: (0.5, 0.5)), (start: (1.5, 0.5))],
            tiles: ["....", "...."], units: [
                (kind: "worker", owner: 0, pos: (0.5, 0.5)),
                (kind: "soldier", owner: 1, pos: (1.5, 0.5)),
            ])"#
        .to_owned(),
        seed: 42,
        players: 2,
    }
}

fn receive(server: &mut Coordinator, client: u64, message: ClientMessage) -> Vec<Delivery> {
    server.receive(client, message, NOW)
}

fn create(server: &mut Coordinator, client: u64, setup: &MatchSetup) -> String {
    let messages = receive(
        server,
        client,
        ClientMessage::Create {
            version: PROTOCOL_VERSION,
            name: "Host".to_owned(),
            public: true,
        },
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].client, client);
    let ServerMessage::Setup(lobby) = &messages[0].message else {
        panic!("expected setup")
    };
    assert_eq!(lobby, server.lobby_setup());
    assert_eq!(lobby.rules, setup.rules);
    assert_eq!(lobby.players, setup.players);
    let ServerMessage::Lobby {
        code,
        player,
        slots,
        public,
        ..
    } = &messages[1].message
    else {
        panic!("expected lobby after setup")
    };
    assert_eq!(*player, PlayerId(0));
    assert!(*public);
    assert_eq!(slots.len(), usize::from(setup.players));
    assert_eq!(
        slots[0],
        Some(rts_net::LobbySlot {
            name: "Host".into(),
            ready: false
        })
    );
    code.clone()
}

fn join(server: &mut Coordinator, client: u64, code: &str) -> Vec<Delivery> {
    receive(
        server,
        client,
        ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code: code.to_owned(),
            name: "Guest".to_owned(),
        },
    )
}

fn lobby() -> (Coordinator, String) {
    let content = setup();
    let mut server = coordinator(fixed(content.clone()));
    for id in 1..=3 {
        assert!(server.connect(id, NOW));
    }
    let code = create(&mut server, 1, &content);
    (server, code)
}

fn ready(server: &mut Coordinator, client: u64) -> Vec<Delivery> {
    let checksum = server.lobby_setup().checksum();
    let loaded = receive(server, client, ClientMessage::Loaded { checksum });
    if loaded
        .iter()
        .any(|d| matches!(d.message, ServerMessage::Error { .. }))
    {
        return loaded;
    }
    receive(server, client, ClientMessage::Ready { ready: true })
}

fn running() -> Coordinator {
    let (mut server, code) = lobby();
    let messages = join(&mut server, 2, &code);
    assert_eq!(
        messages[0].message,
        ServerMessage::Setup(server.lobby_setup().clone())
    );
    assert_eq!(messages[0].client, 2);
    ready(&mut server, 1);
    ready(&mut server, 2);
    let messages = receive(&mut server, 1, ClientMessage::Start);
    assert_eq!(messages.len(), 4);
    let sim = setup().simulation().unwrap();
    for (slot, pair) in messages.chunks(2).enumerate() {
        let player = PlayerId(u8::try_from(slot).unwrap());
        assert_eq!(pair[0].client, u64::from(player.0) + 1);
        assert_eq!(pair[1].client, pair[0].client);
        let ServerMessage::Started {
            players: 2,
            input_delay: 2,
            view,
        } = pair[0].message
        else {
            panic!("expected start")
        };
        assert_eq!(view.player, player);
        assert_eq!(view.start, sim.players()[slot].start);
        assert_eq!(frame(&pair[1]).tick, 0);
    }
    server
}

/// Follows one player's frames like a client.
fn view_of(started: &[Delivery], client: u64) -> PlayerView {
    let mut view = None;
    for delivery in started.iter().filter(|d| d.client == client) {
        match &delivery.message {
            ServerMessage::Started { view: intro, .. } => {
                view = Some(
                    PlayerView::new(rts_base::Rules::from_ron(&setup().rules).unwrap(), *intro)
                        .unwrap(),
                );
            }
            ServerMessage::View { frame } => view.as_mut().unwrap().apply(frame.clone()).unwrap(),
            _ => {}
        }
    }
    view.unwrap()
}

#[test]
fn server_can_disable_fog_for_every_player() {
    let mut setup = setup();
    setup.map = r#"(name: "server", players: [(start: (0.5, 0.5)), (start: (11.5, 0.5))],
            tiles: ["............"], units: [
                (kind: "soldier", owner: 0, pos: (0.5, 0.5)),
                (kind: "worker", owner: 1, pos: (11.5, 0.5)),
            ])"#
    .to_owned();

    for fog_of_war in [true, false] {
        let mut config = fixed(setup.clone());
        config.fog_of_war = fog_of_war;
        let mut server = coordinator(config);
        assert!(server.connect(1, NOW));
        assert!(server.connect(2, NOW));
        let code = create(&mut server, 1, &setup);
        join(&mut server, 2, &code);
        ready(&mut server, 1);
        ready(&mut server, 2);
        let started = receive(&mut server, 1, ClientMessage::Start);

        for (client, far_tile) in [(1, 11), (2, 0)] {
            let view = view_of(&started, client);
            assert_eq!(view.intro().fog_disabled, !fog_of_war);
            assert_eq!(view.units().count(), if fog_of_war { 1 } else { 2 });
            assert_eq!(
                view.visibility(IVec2::new(far_tile, 0)),
                if fog_of_war {
                    rts_base::TileVisibility::Unexplored
                } else {
                    rts_base::TileVisibility::Visible
                }
            );
            assert_eq!(view.terrain(IVec2::new(far_tile, 0)).is_none(), fog_of_war);
            let own_count = view
                .units()
                .filter(|(id, _)| view.own(*id).is_some())
                .count();
            assert_eq!(own_count, 1);
        }
        let mut host = view_of(&started, 1);
        let soldier = host
            .units()
            .find(|(id, _)| host.own(*id).is_some())
            .unwrap()
            .0;
        let target = if fog_of_war {
            UnitId(1)
        } else {
            host.units()
                .find(|(id, _)| host.own(*id).is_none())
                .unwrap()
                .0
        };
        receive(
            &mut server,
            1,
            ClientMessage::Input {
                tick: 1,
                commands: vec![Command::Attack {
                    units: vec![soldier],
                    target,
                }],
            },
        );
        let updates = server.advance(Duration::from_millis(50));
        let update = updates
            .iter()
            .find(|delivery| delivery.client == 1)
            .unwrap();
        host.apply(frame(update).clone()).unwrap();
        assert_eq!(host.attack_target(soldier), (!fog_of_war).then_some(target));
    }
}

fn assert_error(messages: &[Delivery], client: u64) {
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].client, client);
    assert!(matches!(messages[0].message, ServerMessage::Error { .. }));
}

fn assert_ended(messages: &[Delivery], expected: usize) {
    assert_eq!(messages.len(), expected);
    assert!(
        messages
            .iter()
            .all(|delivery| matches!(delivery.message, ServerMessage::Ended { .. }))
    );
}

#[test]
fn joins_without_local_checksum_then_ready_requires_matching_setup_and_host_full_room() {
    let (mut server, code) = lobby();
    assert_error(&receive(&mut server, 1, ClientMessage::Start), 1);
    assert_error(&join(&mut server, 2, "bad"), 2);
    assert_error(&join(&mut server, 2, "999999"), 2);
    assert_error(
        &receive(
            &mut server,
            2,
            ClientMessage::Join {
                version: PROTOCOL_VERSION - 1,
                code: code.clone(),
                name: "Guest".to_owned(),
            },
        ),
        2,
    );
    let joined = join(&mut server, 2, &code);
    assert_eq!(joined.len(), 3);
    assert!(
        matches!(&joined[0].message, ServerMessage::Setup(content) if content == server.lobby_setup())
    );
    assert_error(&join(&mut server, 3, &code), 3);
    assert_error(&receive(&mut server, 2, ClientMessage::Start), 2);
    assert_error(&receive(&mut server, 1, ClientMessage::Start), 1);
    assert_error(
        &receive(&mut server, 2, ClientMessage::Loaded { checksum: 0 }),
        2,
    );
    for delivery in ready(&mut server, 1) {
        assert!(matches!(
            delivery.message,
            ServerMessage::Lobby { ref slots, .. } if slots.iter().flatten().filter(|s| s.ready).count() == 1
        ));
    }
    assert_error(&receive(&mut server, 1, ClientMessage::Start), 1);
    for delivery in ready(&mut server, 1) {
        assert!(matches!(
            delivery.message,
            ServerMessage::Lobby { ref slots, .. } if slots.iter().flatten().filter(|s| s.ready).count() == 1
        ));
    }
    for delivery in ready(&mut server, 2) {
        assert!(matches!(
            delivery.message,
            ServerMessage::Lobby { ref slots, .. } if slots.iter().flatten().filter(|s| s.ready).count() == 2
        ));
    }
    assert_eq!(receive(&mut server, 1, ClientMessage::Start).len(), 4);
    assert_error(&receive(&mut server, 1, ClientMessage::Start), 1);
    assert_error(&ready(&mut server, 2), 2);
}

#[test]
fn readiness_clears_on_departure_and_slots_can_be_reused() {
    for leave in [false, true] {
        let (mut server, code) = lobby();
        join(&mut server, 2, &code);
        ready(&mut server, 1);
        ready(&mut server, 2);
        let messages = if leave {
            receive(&mut server, 2, ClientMessage::Leave)
        } else {
            server.disconnect(2)
        };
        assert!(messages.iter().any(|m| m.client == 1
            && matches!(
                m.message,
                ServerMessage::Lobby {
                    ref slots, ..
                } if slots.iter().flatten().count() == 1 && slots[0].as_ref().unwrap().ready
            )));
        let messages = join(&mut server, 3, &code);
        assert!(messages.iter().any(|m| m.client == 3
            && matches!(
                m.message,
                ServerMessage::Lobby {
                    player: PlayerId(1),
                    ref slots, ..
                } if slots[0].as_ref().unwrap().ready && !slots[1].as_ref().unwrap().ready
            )));
        assert_error(&receive(&mut server, 1, ClientMessage::Start), 1);
        ready(&mut server, 3);
        assert_eq!(receive(&mut server, 1, ClientMessage::Start).len(), 4);
        assert_ended(&server.disconnect(1), 2);
        create(&mut server, 3, &setup());
    }
}

#[test]
fn clock_runs_exactly_twenty_hz_without_inputs_and_never_speeds_up_from_input() {
    let mut server = running();
    assert!(server.advance(NOW).is_empty());
    for tick in [1, 3, 8] {
        assert!(
            receive(
                &mut server,
                1,
                ClientMessage::Input {
                    tick,
                    commands: vec![]
                }
            )
            .is_empty()
        );
    }
    assert!(server.advance(Duration::from_millis(49)).is_empty());
    for tick in 1..=100 {
        let now = Duration::from_millis(u64::from(tick) * 50);
        let messages = server.advance(now);
        assert_eq!(messages.len(), 2);
        assert_eq!((messages[0].client, messages[1].client), (1, 2));
        for delivery in &messages {
            assert_eq!(frame(delivery).tick, tick);
        }
        assert!(server.advance(now).is_empty());
        assert!(server.advance(now + Duration::from_millis(49)).is_empty());
    }
}

#[test]
fn private_views_follow_real_movement_and_ignore_forged_unit_ownership() {
    let (mut server, code) = lobby();
    join(&mut server, 2, &code);
    ready(&mut server, 1);
    ready(&mut server, 2);
    let started = receive(&mut server, 1, ClientMessage::Start);
    let mut host = view_of(&started, 1);
    let mut guest = view_of(&started, 2);
    let worker = host
        .units()
        .find(|(_, unit)| unit.owner == PlayerId(0))
        .unwrap()
        .0;
    let initial = host.unit(worker).unwrap().pos;
    let target = IVec2::new(TILE * 7 / 2, TILE / 2);
    let move_command = Command::Move {
        units: vec![worker, UnitId(1), UnitId(u32::MAX)],
        target,
    };
    assert!(
        receive(
            &mut server,
            1,
            ClientMessage::Input {
                tick: 1,
                commands: vec![move_command]
            }
        )
        .is_empty()
    );
    // The guest names the host's worker by its own view ID. The server must drop it.
    let forged = guest
        .units()
        .find(|(_, unit)| unit.owner == PlayerId(0))
        .unwrap()
        .0;
    receive(
        &mut server,
        2,
        ClientMessage::Input {
            tick: 2,
            commands: vec![Command::Stop {
                units: vec![forged],
            }],
        },
    );
    let mut reference = setup().simulation().unwrap();
    reference.step_with_commands([(
        PlayerId(0),
        Command::Move {
            units: vec![UnitId(0)],
            target,
        },
    )]);
    for tick in 1_u32..=12 {
        if tick > 1 {
            reference.step();
        }
        let messages = server.advance(Duration::from_millis(u64::from(tick) * 50));
        assert_eq!(messages.len(), 2);
        host.apply(frame(&messages[0]).clone()).unwrap();
        guest.apply(frame(&messages[1]).clone()).unwrap();
        assert_eq!(
            host.unit(worker).unwrap().pos,
            reference.unit(UnitId(0)).unwrap().pos
        );
        // The guest's soldier never moved.
        let soldier = guest
            .units()
            .find(|(_, unit)| unit.owner == PlayerId(1))
            .unwrap()
            .1;
        assert_eq!(soldier.pos, reference.unit(UnitId(1)).unwrap().pos);
    }
    assert_ne!(host.unit(worker).unwrap().pos, initial);
    assert_eq!(host.move_target(worker), reference.move_target(UnitId(0)));
    // Each player sees its own orders only.
    let guest_worker = guest.unit(forged).unwrap();
    assert_eq!(guest_worker.pos, host.unit(worker).unwrap().pos);
    assert!(guest.own(forged).is_none());
}

#[test]
fn inputs_authenticate_slots_allow_gaps_and_reject_past_duplicate_out_of_order_and_excess_lead() {
    let mut server = running();
    for tick in [0, MAX_FUTURE_TICKS + 1, u32::MAX] {
        assert_error(
            &receive(
                &mut server,
                1,
                ClientMessage::Input {
                    tick,
                    commands: vec![],
                },
            ),
            1,
        );
    }
    assert_error(
        &receive(
            &mut server,
            3,
            ClientMessage::Input {
                tick: 1,
                commands: vec![],
            },
        ),
        3,
    );
    for tick in [2, MAX_FUTURE_TICKS] {
        assert!(
            receive(
                &mut server,
                1,
                ClientMessage::Input {
                    tick,
                    commands: vec![]
                }
            )
            .is_empty()
        );
    }
    for tick in [2, 7, 8] {
        assert_error(
            &receive(
                &mut server,
                1,
                ClientMessage::Input {
                    tick,
                    commands: vec![],
                },
            ),
            1,
        );
    }
    for tick in 1..=8 {
        server.advance(Duration::from_millis(tick * 50));
    }
    assert_error(
        &receive(
            &mut server,
            2,
            ClientMessage::Input {
                tick: 8,
                commands: vec![],
            },
        ),
        2,
    );
    assert!(
        receive(
            &mut server,
            2,
            ClientMessage::Input {
                tick: 10,
                commands: vec![]
            }
        )
        .is_empty()
    );
    assert!(
        receive(
            &mut server,
            1,
            ClientMessage::Input {
                tick: 8 + MAX_FUTURE_TICKS,
                commands: vec![]
            }
        )
        .is_empty()
    );
    for tick in 9..=10 {
        let messages = server.advance(Duration::from_millis(tick * 50));
        assert_eq!(frame(&messages[0]).tick, u32::try_from(tick).unwrap());
        assert_eq!((messages[0].client, messages[1].client), (1, 2));
    }
}

#[test]
fn invalid_oversized_batches_do_not_consume_a_submission() {
    let mut server = running();
    for commands in [
        vec![Command::Stop {
            units: vec![UnitId(0), UnitId(0)],
        }],
        vec![
            Command::Stop {
                units: vec![UnitId(0)]
            };
            5
        ],
        vec![Command::Stop { units: vec![] }; MAX_COMMANDS_PER_TURN + 1],
        vec![Command::Stop {
            units: vec![UnitId(0); MAX_UNITS_PER_COMMAND + 1],
        }],
        vec![
            Command::Stop {
                units: vec![UnitId(u32::MAX); MAX_UNITS_PER_COMMAND]
            };
            MAX_COMMANDS_PER_TURN
        ],
    ] {
        assert_error(
            &receive(&mut server, 1, ClientMessage::Input { tick: 1, commands }),
            1,
        );
    }
    assert!(
        receive(
            &mut server,
            1,
            ClientMessage::Input {
                tick: 1,
                commands: vec![]
            }
        )
        .is_empty()
    );
}

#[test]
fn full_room_inputs_strip_nonexistent_and_forged_unit_selections() {
    let mut content = setup();
    content.map = content.map.replace(
        "(start: (1.5, 0.5))]",
        &("(start: (1.5, 0.5)),".repeat(7) + "]"),
    );
    content.players = 8;
    let mut server = coordinator(fixed(content.clone()));
    for id in 1..=8 {
        assert!(server.connect(id, NOW));
    }
    let code = create(&mut server, 1, &content);
    for id in 2..=8 {
        join(&mut server, id, &code);
    }
    let checksum = server.lobby_setup().checksum();
    for id in 1..=8 {
        receive(&mut server, id, ClientMessage::Loaded { checksum });
        receive(&mut server, id, ClientMessage::Ready { ready: true });
    }
    assert_eq!(receive(&mut server, 1, ClientMessage::Start).len(), 16);
    for id in 1..=8 {
        // Every player orders every view ID to move. Only player 0's worker may move, and only
        // when player 0 orders it.
        let commands = vec![Command::Move {
            units: (0..u32::try_from(MAX_UNITS_PER_COMMAND).unwrap())
                .map(UnitId)
                .collect(),
            target: IVec2::new(TILE * 7 / 2, TILE * 3 / 2),
        }];
        assert!(receive(&mut server, id, ClientMessage::Input { tick: 1, commands }).is_empty());
    }
    let messages = server.advance(Duration::from_millis(50));
    assert_eq!(messages.len(), 8);
    for delivery in &messages {
        let encoded = encode_server(&delivery.message).unwrap();
        assert_eq!(rts_net::decode_server(&encoded).unwrap(), delivery.message);
    }
    let mut reference = content.simulation().unwrap();
    reference.step_with_commands([(
        PlayerId(0),
        Command::Move {
            units: vec![UnitId(0)],
            target: IVec2::new(TILE * 7 / 2, TILE * 3 / 2),
        },
    )]);
    for _ in 2..=40 {
        reference.step();
        server.advance(Duration::from_millis(u64::from(reference.tick()) * 50));
    }
    // The soldier never moved; the worker reached its target.
    assert_eq!(
        reference.unit(UnitId(1)).unwrap().pos,
        content.simulation().unwrap().unit(UnitId(1)).unwrap().pos
    );
}

#[test]
fn long_stalls_bound_catch_up_then_resume_normal_cadence() {
    let mut server = running();
    assert_eq!(
        server.advance(Duration::from_secs(100)).len(),
        MAX_CATCH_UP_TURNS * 2
    );
    assert!(server.advance(Duration::from_secs(100)).is_empty());
    assert!(server.advance(Duration::from_millis(100_049)).is_empty());
    let messages = server.advance(Duration::from_millis(100_050));
    assert_eq!(messages.len(), 2);
    assert_eq!(
        frame(&messages[0]).tick,
        u32::try_from(MAX_CATCH_UP_TURNS + 1).unwrap()
    );
}

#[test]
fn invalid_server_setup_refuses_connections_without_panicking() {
    for content in [
        MatchSetup {
            players: 0,
            ..setup()
        },
        MatchSetup {
            rules: "bad".into(),
            ..setup()
        },
        MatchSetup {
            map: "bad".into(),
            ..setup()
        },
    ] {
        let mut server = coordinator(fixed(content));
        assert!(!server.connect(1, NOW));
        assert!(
            receive(
                &mut server,
                1,
                ClientMessage::Create {
                    version: PROTOCOL_VERSION,
                    name: "Host".to_owned(),
                    public: true,
                }
            )
            .is_empty()
        );
        assert!(server.advance(Duration::from_secs(1)).is_empty());
    }
}

#[test]
fn leave_disconnect_and_socket_expiry_end_games_but_no_input_progress_timeout_exists() {
    assert_ended(&running().disconnect(2), 2);
    assert_ended(&receive(&mut running(), 2, ClientMessage::Leave), 2);
    let mut server = running();
    for id in 1..=2 {
        server.touch(id, Duration::from_secs(20));
    }
    assert!(server.expire(Duration::from_secs(20)).is_empty());
    assert_eq!(
        server.advance(Duration::from_secs(20)).len(),
        MAX_CATCH_UP_TURNS * 2
    );
    assert_ended(&server.expire(Duration::from_secs(35)), 2);
}

#[test]
fn room_connection_limits_and_unique_codes_remain_enforced() {
    let mut server = coordinator(Config {
        max_connections: 3,
        max_rooms: 2,
        ..fixed(setup())
    });
    assert!(server.connect(1, NOW));
    assert!(!server.connect(1, NOW));
    assert!(server.connect(2, NOW));
    assert!(server.connect(3, NOW));
    assert!(!server.connect(4, NOW));
    assert_error(
        &receive(
            &mut server,
            1,
            ClientMessage::Create {
                version: PROTOCOL_VERSION - 1,
                name: "Host".to_owned(),
                public: true,
            },
        ),
        1,
    );
    let first = create(&mut server, 1, &setup());
    let second = create(&mut server, 2, &setup());
    assert_ne!(first, second);
    for code in [&first, &second] {
        assert_eq!(code.len(), 6);
        assert!(
            code.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()),
            "{code}"
        );
    }
    assert_error(
        &receive(
            &mut server,
            3,
            ClientMessage::Create {
                version: PROTOCOL_VERSION,
                name: "Host".to_owned(),
                public: true,
            },
        ),
        3,
    );
    server.disconnect(1);
    let third = create(&mut server, 3, &setup());
    assert_ne!(third, second);
}

fn rooms(server: &mut Coordinator, client: u64) -> Vec<rts_net::RoomInfo> {
    let messages = receive(
        server,
        client,
        ClientMessage::Rooms {
            version: PROTOCOL_VERSION,
        },
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].client, client);
    let ServerMessage::Rooms { rooms } = &messages[0].message else {
        panic!("expected rooms")
    };
    encode_server(&messages[0].message).unwrap();
    rooms.clone()
}

fn assert_error_text(messages: &[Delivery], client: u64, expected: &str) {
    assert_error(messages, client);
    assert_eq!(
        messages[0].message,
        ServerMessage::Error {
            message: expected.into()
        }
    );
}

#[test]
fn room_listing_filters_public_waiting_nonfull_rooms_and_keeps_code_order_and_limit() {
    let content = setup();
    let mut server = coordinator(fixed(content.clone()));
    for id in 1..=80 {
        assert!(server.connect(id, NOW));
    }
    assert_eq!(rooms(&mut server, 80), []);
    let private = receive(
        &mut server,
        1,
        ClientMessage::Create {
            version: PROTOCOL_VERSION,
            name: "Private Host".into(),
            public: false,
        },
    );
    let ServerMessage::Lobby {
        code: private_code,
        public: false,
        ..
    } = &private[1].message
    else {
        panic!("expected private lobby")
    };
    assert_eq!(rooms(&mut server, 80), []);
    // Private rooms remain joinable by code.
    assert_eq!(join(&mut server, 2, private_code).len(), 3);
    let full_code = create(&mut server, 3, &content);
    join(&mut server, 4, &full_code);
    let started_code = create(&mut server, 5, &content);
    join(&mut server, 6, &started_code);
    ready(&mut server, 5);
    ready(&mut server, 6);
    receive(&mut server, 5, ClientMessage::Start);
    assert_eq!(rooms(&mut server, 80), []);
    let mut codes: Vec<_> = (7..=76)
        .map(|id| create(&mut server, id, &content))
        .collect();
    codes.sort();
    let listed = rooms(&mut server, 80);
    assert_eq!(listed.len(), rts_net::MAX_LISTED_ROOMS);
    assert_eq!(
        listed.iter().map(|r| r.code.clone()).collect::<Vec<_>>(),
        codes[..rts_net::MAX_LISTED_ROOMS]
    );
    for room in listed {
        assert_eq!(room.host, "Host");
        assert_eq!(room.map, "server");
        assert_eq!((room.players, room.capacity), (1, 2));
    }
    assert_error_text(
        &receive(
            &mut server,
            7,
            ClientMessage::Rooms {
                version: PROTOCOL_VERSION,
            },
        ),
        7,
        "connection is already in a room",
    );
    assert_error_text(
        &receive(
            &mut server,
            80,
            ClientMessage::Rooms {
                version: PROTOCOL_VERSION - 1,
            },
        ),
        80,
        "protocol version does not match",
    );
}

#[test]
fn explicit_readiness_requires_loaded_and_all_ready_and_can_be_revoked() {
    let (mut server, code) = lobby();
    join(&mut server, 2, &code);
    assert_error_text(
        &receive(&mut server, 1, ClientMessage::Ready { ready: true }),
        1,
        "load the match setup first",
    );
    assert_error_text(
        &receive(&mut server, 1, ClientMessage::Loaded { checksum: 0 }),
        1,
        "setup checksum does not match",
    );
    assert_error_text(
        &receive(&mut server, 1, ClientMessage::Ready { ready: true }),
        1,
        "load the match setup first",
    );
    let checksum = server.lobby_setup().checksum();
    for id in 1..=2 {
        let loaded = receive(&mut server, id, ClientMessage::Loaded { checksum });
        for message in loaded {
            assert!(
                matches!(message.message, ServerMessage::Lobby { slots, .. } if slots.iter().flatten().all(|s| !s.ready))
            );
        }
    }
    assert_error_text(
        &receive(&mut server, 1, ClientMessage::Start),
        1,
        "all players must be ready",
    );
    receive(&mut server, 1, ClientMessage::Ready { ready: true });
    assert_error_text(
        &receive(&mut server, 1, ClientMessage::Start),
        1,
        "all players must be ready",
    );
    receive(&mut server, 2, ClientMessage::Ready { ready: true });
    let changed = receive(&mut server, 2, ClientMessage::Ready { ready: false });
    assert_eq!(changed.len(), 2);
    for message in changed {
        assert!(
            matches!(message.message, ServerMessage::Lobby { slots, .. } if slots[0].as_ref().unwrap().ready && !slots[1].as_ref().unwrap().ready)
        );
    }
    assert_error_text(
        &receive(&mut server, 1, ClientMessage::Start),
        1,
        "all players must be ready",
    );
    receive(&mut server, 2, ClientMessage::Ready { ready: true });
    assert_eq!(receive(&mut server, 1, ClientMessage::Start).len(), 4);
    assert_error_text(
        &receive(&mut server, 2, ClientMessage::Ready { ready: false }),
        2,
        "match has already started",
    );
    assert_error_text(
        &receive(&mut server, 2, ClientMessage::Loaded { checksum }),
        2,
        "match has already started",
    );
}

#[test]
fn guest_leave_can_list_and_rejoin_with_new_name_but_must_load_and_ready_again() {
    let (mut server, code) = lobby();
    join(&mut server, 2, &code);
    ready(&mut server, 1);
    ready(&mut server, 2);
    let left = receive(&mut server, 2, ClientMessage::Leave);
    assert_eq!(left.len(), 2);
    assert_eq!(left[0].client, 2);
    assert_eq!(
        left[0].message,
        ServerMessage::Ended {
            reason: "left the room".into()
        }
    );
    assert!(matches!(&left[1].message, ServerMessage::Lobby { slots, .. } if slots[1].is_none()));
    assert!(server.is_connected(2));
    assert_eq!(rooms(&mut server, 2)[0].code, code);
    let joined = receive(
        &mut server,
        2,
        ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code,
            name: "New Guest".into(),
        },
    );
    for message in joined.iter().skip(1) {
        assert!(
            matches!(&message.message, ServerMessage::Lobby { slots, .. }
            if slots[1] == Some(rts_net::LobbySlot { name: "New Guest".into(), ready: false }))
        );
    }
    assert_error_text(
        &receive(&mut server, 2, ClientMessage::Ready { ready: true }),
        2,
        "load the match setup first",
    );
    assert_error(&receive(&mut server, 1, ClientMessage::Start), 1);
    ready(&mut server, 2);
    assert_eq!(receive(&mut server, 1, ClientMessage::Start).len(), 4);
}

#[test]
fn host_leave_or_any_match_leave_ends_room_and_all_connections_can_browse_and_create_again() {
    for started in [false, true] {
        let (mut server, code) = lobby();
        join(&mut server, 2, &code);
        if started {
            ready(&mut server, 1);
            ready(&mut server, 2);
            receive(&mut server, 1, ClientMessage::Start);
        }
        let leaving = if started { 2 } else { 1 };
        assert_ended(&receive(&mut server, leaving, ClientMessage::Leave), 2);
        for id in 1..=2 {
            assert!(server.is_connected(id));
            assert_eq!(rooms(&mut server, id), []);
        }
        let new_code = create(&mut server, 2, &setup());
        assert_eq!(join(&mut server, 1, &new_code).len(), 3);
    }
}

#[test]
fn listed_map_name_is_sanitized_and_truncated_by_character() {
    let mut content = setup();
    let name = format!("Map\n{}", "界".repeat(70));
    content.map = content
        .map
        .replace("\"server\"", &format!("\"Map\\n{}\"", "界".repeat(70)));
    let mut server = coordinator(fixed(content.clone()));
    assert!(server.connect(1, NOW));
    assert!(server.connect(2, NOW));
    create(&mut server, 1, &content);
    let listed = rooms(&mut server, 2);
    assert_eq!(
        listed[0].map,
        name.replace('\n', " ")
            .chars()
            .take(rts_net::MAX_MAP_NAME_CHARS)
            .collect::<String>()
    );
}
