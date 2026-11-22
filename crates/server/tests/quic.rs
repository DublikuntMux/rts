use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

use rts_base::{Command, IVec2, PlayerId, PlayerView, Rules, TILE, UnitId};
use rts_net::{
    ClientMessage, LobbySetup, MAX_MESSAGE_BYTES, MAX_SERVER_MESSAGE_BYTES, MatchSetup,
    PROTOCOL_VERSION, Recording, ServerMessage, ViewReplay, decode_server, encode_client,
    quic::{
        PRIORITY_GAME, STREAM_GAME, ServerCertificate, client_endpoint,
        quinn::{Connection, Endpoint, RecvStream, SendStream},
        read_frame, server_endpoint, write_frame,
    },
};
use rts_server::{Config, RANDOM_MAP_NAME, serve_with_config};
use tokio::{
    task::JoinHandle,
    time::{sleep, timeout},
};

struct Client {
    _endpoint: Endpoint,
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

impl Client {
    fn close(self) {
        self.conn.close(0_u32.into(), b"bye");
    }
}

struct Server {
    address: std::net::SocketAddr,
    task: JoinHandle<std::io::Result<()>>,
}

impl Server {
    fn new(config: Config) -> Self {
        let cert = ServerCertificate::generate_self_signed().unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let endpoint = server_endpoint(socket, &cert, config.lobby_timeout).unwrap();
        Self {
            address,
            task: tokio::spawn(serve_with_config(endpoint, config)),
        }
    }

    async fn connect(&self) -> Client {
        let endpoint = client_endpoint(false, None, Duration::from_secs(15)).unwrap();
        let conn = timeout(
            Duration::from_secs(3),
            endpoint.connect(self.address, "rts.local").unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        let (mut send, recv) = conn.open_bi().await.unwrap();
        let _ = send.set_priority(PRIORITY_GAME);
        send.write_all(&[STREAM_GAME]).await.unwrap();
        Client {
            _endpoint: endpoint,
            conn,
            send,
            recv,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn send(client: &mut Client, message: &ClientMessage) {
    write_frame(&mut client.send, &encode_client(message).unwrap())
        .await
        .unwrap();
}

async fn send_raw(client: &mut Client, bytes: &[u8]) {
    write_frame(&mut client.send, bytes).await.unwrap();
}

async fn receive(client: &mut Client) -> ServerMessage {
    let bytes = timeout(
        Duration::from_secs(3),
        read_frame(&mut client.recv, MAX_SERVER_MESSAGE_BYTES),
    )
    .await
    .unwrap()
    .unwrap()
    .expect("expected server frame");
    decode_server(&bytes).unwrap()
}

/// Expects an error, skipping view frames that a running match streams meanwhile.
async fn expect_error(client: &mut Client) {
    loop {
        match receive(client).await {
            ServerMessage::Error { .. } => return,
            ServerMessage::View { .. } => {}
            message => panic!("expected an error, got {message:?}"),
        }
    }
}

async fn create(host: &mut Client) -> (String, LobbySetup) {
    send(
        host,
        &ClientMessage::Create {
            version: PROTOCOL_VERSION,
            name: "Host".to_owned(),
            public: true,
        },
    )
    .await;
    let ServerMessage::Setup(setup) = receive(host).await else {
        panic!("expected setup first")
    };
    let ServerMessage::Lobby {
        code,
        player: PlayerId(0),
        slots,
        ..
    } = receive(host).await
    else {
        panic!("expected host lobby")
    };
    assert_eq!(slots.iter().flatten().count(), 1);
    assert!(!slots[0].as_ref().unwrap().ready);
    (code, setup)
}

async fn join(host: &mut Client, guest: &mut Client, code: &str) -> LobbySetup {
    send(
        guest,
        &ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code: code.to_owned(),
            name: "Guest".to_owned(),
        },
    )
    .await;
    let ServerMessage::Setup(setup) = receive(guest).await else {
        panic!("expected setup first")
    };
    assert!(matches!(
        receive(host).await,
        ServerMessage::Lobby {
            player: PlayerId(0),
            ref slots, ..
        } if slots.iter().flatten().count() == 2
    ));
    assert!(matches!(
        receive(guest).await,
        ServerMessage::Lobby {
            player: PlayerId(1),
            ref slots, ..
        } if slots.iter().flatten().count() == 2
    ));
    setup
}

async fn ready(host: &mut Client, guest: &mut Client, checksum: u64) {
    for host_turn in [true, false] {
        let sender = if host_turn { &mut *host } else { &mut *guest };
        send(sender, &ClientMessage::Loaded { checksum }).await;
        for client in [&mut *host, &mut *guest] {
            assert!(matches!(receive(client).await, ServerMessage::Lobby { .. }));
        }
        let sender = if host_turn { &mut *host } else { &mut *guest };
        send(sender, &ClientMessage::Ready { ready: true }).await;
        for client in [&mut *host, &mut *guest] {
            let ServerMessage::Lobby { slots, .. } = receive(client).await else {
                panic!("expected lobby")
            };
            assert_eq!(
                slots.iter().flatten().filter(|s| s.ready).count(),
                if host_turn { 1 } else { 2 }
            );
        }
    }
}

/// A client's record of its match: the view it draws and the replay it keeps.
struct Perspective {
    view: PlayerView,
    replay: ViewReplay,
}

impl Perspective {
    async fn next(&mut self, client: &mut Client) {
        let ServerMessage::View { frame } = receive(client).await else {
            panic!("expected a view frame")
        };
        self.replay.push(frame.clone()).unwrap();
        self.view.apply(frame).unwrap();
    }
}

async fn begin(client: &mut Client, setup: &LobbySetup) -> Perspective {
    let ServerMessage::Started {
        players: 2,
        input_delay: 2,
        view,
    } = receive(client).await
    else {
        panic!("expected start")
    };
    let mut perspective = Perspective {
        view: PlayerView::new(Rules::from_ron(&setup.rules).unwrap(), view).unwrap(),
        replay: ViewReplay::new(setup.map.clone(), setup.rules.clone(), view).unwrap(),
    };
    perspective.next(client).await;
    assert_eq!(perspective.view.tick(), 0);
    perspective
}

async fn start(
    host: &mut Client,
    guest: &mut Client,
    setup: &LobbySetup,
) -> (Perspective, Perspective) {
    send(host, &ClientMessage::Start).await;
    let host_view = begin(host, setup).await;
    let guest_view = begin(guest, setup).await;
    assert_eq!(host_view.view.player(), PlayerId(0));
    assert_eq!(guest_view.view.player(), PlayerId(1));
    (host_view, guest_view)
}

/// A public corridor: the host's soldier in the west, the guest's worker in the east.
fn corridor() -> Config {
    let row = ".".repeat(14);
    Config {
        setup: MatchSetup {
            rules: Config::default().setup.rules,
            map: format!(
                r#"(name: "Corridor", players: [(start: (1.5, 1.5)), (start: (12.5, 1.5))],
                tiles: ["{row}", "{row}", "{row}"],
                units: [
                    (kind: "soldier", owner: 0, pos: (1.5, 1.5)),
                    (kind: "worker", owner: 1, pos: (12.5, 1.5)),
                ])"#
            ),
            seed: 7,
            players: 2,
        },
        random_layout: false,
        ..Config::default()
    }
}

async fn lobby(server: &Server) -> (Client, Client, LobbySetup) {
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    assert_eq!(join(&mut host, &mut guest, &code).await, setup);
    ready(&mut host, &mut guest, setup.checksum()).await;
    (host, guest, setup)
}

#[tokio::test]
async fn quic_players_receive_distinct_private_views_of_a_random_layout() {
    let server = Server::new(Config::default());
    let (mut host, mut guest, setup) = lobby(&server).await;
    assert_eq!(setup.map, RANDOM_MAP_NAME);
    assert!(!setup.rules.contains("//"), "rules arrive minified");
    let (mut host_view, mut guest_view) = start(&mut host, &mut guest, &setup).await;
    assert_ne!(host_view.view.start(), guest_view.view.start());
    let worker = host_view
        .view
        .units()
        .find(|(_, unit)| unit.kind == host_view.view.rules().unit_kind("worker").unwrap())
        .unwrap()
        .0;
    let initial = host_view.view.unit(worker).unwrap().pos;
    let target = host_view.view.start() + IVec2::new(0, 2 * TILE);
    send(
        &mut host,
        &ClientMessage::Input {
            tick: 2,
            commands: vec![Command::Move {
                units: vec![worker],
                target,
            }],
        },
    )
    .await;
    // The guest cannot command the host's units with any ID.
    send(
        &mut guest,
        &ClientMessage::Input {
            tick: 3,
            commands: vec![Command::Stop {
                units: (0..64).map(UnitId).collect(),
            }],
        },
    )
    .await;
    for tick in 1..=30 {
        host_view.next(&mut host).await;
        guest_view.next(&mut guest).await;
        assert_eq!(host_view.view.tick(), tick);
        assert_eq!(guest_view.view.tick(), tick);
        for perspective in [&host_view, &guest_view] {
            let view = &perspective.view;
            assert!(view.units().all(|(_, unit)| unit.owner == view.player()));
            assert!(view.resources().is_some());
        }
    }
    assert_ne!(host_view.view.unit(worker).unwrap().pos, initial);
    for perspective in [&host_view, &guest_view] {
        let text = Recording::View(perspective.replay.clone())
            .to_ron()
            .unwrap();
        assert!(!text.contains("seed") && !text.contains("tiles:"));
        let (Recording::View(replay), None) = Recording::from_ron(&text).unwrap() else {
            panic!("expected a view replay")
        };
        let replayed = replay.playback().unwrap();
        assert_eq!(replayed.tick(), perspective.view.tick());
        assert_eq!(
            replayed.units().collect::<Vec<_>>(),
            perspective.view.units().collect::<Vec<_>>()
        );
        assert_eq!(replayed.terrain_tiles(), perspective.view.terrain_tiles());
    }
    assert_ne!(
        host_view.view.terrain_tiles(),
        guest_view.view.terrain_tiles(),
        "each player explored a different part of the map"
    );
}

#[tokio::test]
async fn quic_units_are_discovered_then_hidden_and_quiet_frames_leak_nothing() {
    let server = Server::new(corridor());
    let (mut host, mut guest, setup) = lobby(&server).await;
    assert_eq!(setup.map, "Corridor");
    let (mut host_view, mut guest_view) = start(&mut host, &mut guest, &setup).await;
    let soldier = host_view.view.units().next().unwrap().0;
    assert_eq!(host_view.view.units().count(), 1);
    assert_eq!(host_view.view.terrain(IVec2::new(12, 1)), None);
    send(
        &mut host,
        &ClientMessage::Input {
            tick: 2,
            commands: vec![Command::Move {
                units: vec![soldier],
                target: IVec2::new(8 * TILE + TILE / 2, TILE + TILE / 2),
            }],
        },
    )
    .await;
    let mut seen = None;
    for _ in 0..200 {
        host_view.next(&mut host).await;
        guest_view.next(&mut guest).await;
        if let Some((id, unit)) = host_view
            .view
            .units()
            .find(|(_, unit)| unit.owner == PlayerId(1))
        {
            assert!(host_view.view.own(id).is_none());
            assert_eq!(unit.pos, IVec2::new(12 * TILE + TILE / 2, TILE + TILE / 2));
            seen = Some(id);
            break;
        }
    }
    let enemy = seen.expect("the guest's worker comes into sight");
    assert!(
        guest_view
            .view
            .units()
            .any(|(_, unit)| unit.owner == PlayerId(0))
    );
    let tick = host_view.view.tick();
    send(
        &mut host,
        &ClientMessage::Input {
            tick: tick + 2,
            commands: vec![Command::Move {
                units: vec![soldier],
                target: IVec2::new(TILE + TILE / 2, TILE + TILE / 2),
            }],
        },
    )
    .await;
    for _ in 0..140 {
        host_view.next(&mut host).await;
        guest_view.next(&mut guest).await;
    }
    assert!(host_view.view.unit(enemy).is_none(), "the worker vanished");
    assert_eq!(
        host_view.view.visibility(IVec2::new(9, 1)),
        rts_base::TileVisibility::Remembered
    );
    assert!(host_view.view.terrain(IVec2::new(9, 1)).is_some());
    let tick = guest_view.view.tick();
    let worker = guest_view
        .view
        .units()
        .find(|(_, u)| u.owner == PlayerId(1))
        .unwrap()
        .0;
    send(
        &mut guest,
        &ClientMessage::Input {
            tick: tick + 2,
            commands: vec![Command::Move {
                units: vec![worker],
                target: IVec2::new(13 * TILE + TILE / 2, 2 * TILE + TILE / 2),
            }],
        },
    )
    .await;
    for _ in 0..20 {
        let ServerMessage::View { frame } = receive(&mut host).await else {
            panic!("expected a frame")
        };
        assert!(frame.is_quiet(), "hidden activity leaked: {frame:?}");
        host_view.view.apply(frame).unwrap();
        guest_view.next(&mut guest).await;
    }
    assert_ne!(
        guest_view.view.unit(worker).unwrap().pos,
        IVec2::new(12 * TILE + TILE / 2, TILE + TILE / 2)
    );
}

#[tokio::test]
async fn readiness_is_required_and_invalid_joins_or_starts_remain_retryable() {
    let server = Server::new(Config::default());
    let (mut host, mut guest, mut outsider) = (
        server.connect().await,
        server.connect().await,
        server.connect().await,
    );
    let (code, setup) = create(&mut host).await;
    send(&mut host, &ClientMessage::Start).await;
    expect_error(&mut host).await;
    send_raw(&mut guest, &[2, 1]).await; // invalid Join
    expect_error(&mut guest).await;
    join(&mut host, &mut guest, &code).await;
    send(
        &mut outsider,
        &ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code,
            name: "Guest".to_owned(),
        },
    )
    .await;
    expect_error(&mut outsider).await;
    send(&mut host, &ClientMessage::Start).await;
    expect_error(&mut host).await;
    send(
        &mut guest,
        &ClientMessage::Loaded {
            checksum: setup.checksum() ^ 1,
        },
    )
    .await;
    expect_error(&mut guest).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    send(&mut guest, &ClientMessage::Start).await;
    expect_error(&mut guest).await;
    start(&mut host, &mut guest, &setup).await;
}

#[tokio::test]
async fn runtime_clock_has_no_initial_turn_and_runs_without_input_even_with_slow_config_poll() {
    let server = Server::new(Config {
        poll_interval: Duration::from_millis(250),
        ..Config::default()
    });
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    let started = Instant::now();
    let (mut host_view, mut guest_view) = start(&mut host, &mut guest, &setup).await;
    for tick in 1..=6 {
        host_view.next(&mut host).await;
        guest_view.next(&mut guest).await;
        assert_eq!(host_view.view.tick(), tick);
        assert_eq!(guest_view.view.tick(), tick);
        assert!(started.elapsed() >= Duration::from_millis(u64::from(tick) * 50));
    }
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn inputs_cannot_vote_for_speed_change_and_invalid_frames_are_retryable() {
    let server = Server::new(Config::default());
    let (mut host, mut guest, mut outsider) = (
        server.connect().await,
        server.connect().await,
        server.connect().await,
    );
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    let (mut host_view, _) = start(&mut host, &mut guest, &setup).await;
    send(
        &mut outsider,
        &ClientMessage::Input {
            tick: 2,
            commands: vec![],
        },
    )
    .await;
    expect_error(&mut outsider).await;
    for raw in [&[99_u8][..], &[6, 0, 0]] {
        send_raw(&mut guest, raw).await;
        expect_error(&mut guest).await;
    }
    for tick in [2, 8] {
        send(
            &mut guest,
            &ClientMessage::Input {
                tick,
                commands: vec![],
            },
        )
        .await;
    }
    host_view.next(&mut host).await;
    assert_eq!(host_view.view.tick(), 1);
}

#[tokio::test]
async fn quic_keepalive_keeps_game_running_with_no_client_input() {
    let server = Server::new(Config {
        peer_timeout: Duration::from_millis(180),
        ..Config::default()
    });
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    let (mut host_view, mut guest_view) = start(&mut host, &mut guest, &setup).await;
    for tick in 1..=16 {
        host_view.next(&mut host).await;
        guest_view.next(&mut guest).await;
        assert_eq!(
            (host_view.view.tick(), guest_view.view.tick()),
            (tick, tick)
        );
    }
}

#[tokio::test]
async fn large_setup_is_minified_and_compressed_before_lobby() {
    let mut config = Config::default();
    config
        .setup
        .rules
        .push_str(&" ".repeat(MAX_MESSAGE_BYTES * 3));
    config.setup.seed = 123;
    let expected = LobbySetup {
        rules: config.setup.minified().rules,
        map: RANDOM_MAP_NAME.to_owned(),
        players: 2,
    };
    let server = Server::new(config);
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    assert_eq!(setup, expected);
    assert_eq!(join(&mut host, &mut guest, &code).await, expected);
    ready(&mut host, &mut guest, setup.checksum()).await;
    start(&mut host, &mut guest, &setup).await;
}

#[tokio::test]
async fn guest_disconnect_updates_lobby_and_oversized_client_input_ends_match() {
    let server = Server::new(Config::default());
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    guest.close();
    assert!(matches!(
        receive(&mut host).await,
        ServerMessage::Lobby {
            ref slots, ..
        } if slots.iter().flatten().count() == 1 && !slots[0].as_ref().unwrap().ready
    ));
    let mut guest = server.connect().await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    start(&mut host, &mut guest, &setup).await;
    let _ = write_frame(&mut guest.send, &vec![0xff; MAX_MESSAGE_BYTES + 1]).await;
    loop {
        match receive(&mut host).await {
            ServerMessage::Ended { .. } => break,
            ServerMessage::View { .. } => {}
            message => panic!("unexpected {message:?}"),
        }
    }
}

#[tokio::test]
async fn stalled_stream_handshake_is_bounded_and_releases_connection_capacity() {
    let server = Server::new(Config {
        max_connections: 1,
        handshake_timeout: Duration::from_millis(80),
        ..Config::default()
    });
    // Connect QUIC without opening the required game stream so the handshake times out.
    let endpoint = client_endpoint(false, None, Duration::from_secs(5)).unwrap();
    let _stalled = endpoint
        .connect(server.address, "rts.local")
        .unwrap()
        .await
        .unwrap();
    sleep(Duration::from_millis(160)).await;
    create(&mut server.connect().await).await;
}

#[tokio::test]
async fn invalid_setup_and_resource_limits_fail_before_accepting_connections() {
    let cert = ServerCertificate::generate_self_signed().unwrap();
    for config in [
        Config {
            setup: MatchSetup {
                rules: "invalid".to_owned(),
                ..Config::default().setup
            },
            ..Config::default()
        },
        Config {
            max_connections: 0,
            ..Config::default()
        },
        Config {
            poll_interval: Duration::ZERO,
            ..Config::default()
        },
        Config {
            event_queue_capacity: usize::MAX,
            ..Config::default()
        },
        Config {
            outbound_queue_capacity: usize::MAX,
            ..Config::default()
        },
    ] {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = server_endpoint(socket, &cert, Duration::from_secs(5)).unwrap();
        let error = serve_with_config(endpoint, config).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[tokio::test]
async fn room_browsing_and_leaving_reuse_the_same_quic_connections() {
    let server = Server::new(Config::default());
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    send_raw(&mut guest, &[0, 4]).await; // Rooms with old version 4
    assert_eq!(
        receive(&mut guest).await,
        ServerMessage::Error {
            message: "protocol version does not match".into()
        }
    );
    send(
        &mut guest,
        &ClientMessage::Rooms {
            version: PROTOCOL_VERSION,
        },
    )
    .await;
    assert_eq!(
        receive(&mut guest).await,
        ServerMessage::Rooms { rooms: vec![] }
    );
    let (code, _) = create(&mut host).await;
    send(
        &mut guest,
        &ClientMessage::Rooms {
            version: PROTOCOL_VERSION,
        },
    )
    .await;
    let ServerMessage::Rooms { rooms } = receive(&mut guest).await else {
        panic!("expected rooms")
    };
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].code, code);
    assert_eq!(rooms[0].host, "Host");
    join(&mut host, &mut guest, &code).await;
    send(
        &mut guest,
        &ClientMessage::Rooms {
            version: PROTOCOL_VERSION,
        },
    )
    .await;
    assert_eq!(
        receive(&mut guest).await,
        ServerMessage::Error {
            message: "connection is already in a room".into()
        }
    );
    send(&mut guest, &ClientMessage::Ready { ready: true }).await;
    assert_eq!(
        receive(&mut guest).await,
        ServerMessage::Error {
            message: "load the match setup first".into()
        }
    );
    send(&mut guest, &ClientMessage::Leave).await;
    assert_eq!(
        receive(&mut guest).await,
        ServerMessage::Ended {
            reason: "left the room".into()
        }
    );
    assert!(
        matches!(receive(&mut host).await, ServerMessage::Lobby { slots, .. } if slots[1].is_none())
    );
    send(
        &mut guest,
        &ClientMessage::Rooms {
            version: PROTOCOL_VERSION,
        },
    )
    .await;
    assert!(
        matches!(receive(&mut guest).await, ServerMessage::Rooms { rooms } if rooms.len() == 1)
    );
    join(&mut host, &mut guest, &code).await;
    send(&mut host, &ClientMessage::Leave).await;
    for client in [&mut host, &mut guest] {
        assert!(matches!(receive(client).await, ServerMessage::Ended { .. }));
        send(
            client,
            &ClientMessage::Rooms {
                version: PROTOCOL_VERSION,
            },
        )
        .await;
        assert_eq!(
            receive(client).await,
            ServerMessage::Rooms { rooms: vec![] }
        );
    }
    let (code, _) = create(&mut guest).await;
    join(&mut guest, &mut host, &code).await;
}
