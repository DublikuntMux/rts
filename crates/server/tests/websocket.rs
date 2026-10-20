#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use rts_base::{Command, IVec2, PlayerId, TILE, UnitId};
use rts_net::{
    ClientMessage, MAX_MESSAGE_BYTES, MatchSetup, PROTOCOL_VERSION, Replay, ServerMessage,
    decode_server, decode_server_binary, encode_client,
};
use rts_server::{Config, serve_with_config};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Server {
    address: std::net::SocketAddr,
    task: JoinHandle<std::io::Result<()>>,
}

impl Server {
    async fn new(config: Config) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        Self {
            address,
            task: tokio::spawn(serve_with_config(listener, config)),
        }
    }

    async fn connect(&self) -> Client {
        timeout(
            Duration::from_secs(2),
            connect_async(format!("ws://{}", self.address)),
        )
        .await
        .unwrap()
        .unwrap()
        .0
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn send(client: &mut Client, message: &ClientMessage) {
    client
        .send(Message::Text(encode_client(message).unwrap().into()))
        .await
        .unwrap();
}

async fn receive(client: &mut Client) -> ServerMessage {
    timeout(Duration::from_secs(3), async {
        loop {
            match client.next().await.unwrap().unwrap() {
                Message::Text(text) => return decode_server(&text).unwrap(),
                Message::Binary(bytes) => return decode_server_binary(&bytes).unwrap(),
                Message::Ping(_) => client.flush().await.unwrap(),
                message => panic!("expected server frame, got {message:?}"),
            }
        }
    })
    .await
    .unwrap()
}

async fn expect_error(client: &mut Client) {
    assert!(matches!(receive(client).await, ServerMessage::Error { .. }));
}

async fn create(host: &mut Client) -> (String, MatchSetup) {
    send(
        host,
        &ClientMessage::Create {
            version: PROTOCOL_VERSION,
        },
    )
    .await;
    let ServerMessage::Setup(setup) = receive(host).await else {
        panic!("expected setup first")
    };
    let ServerMessage::Lobby {
        code,
        player: PlayerId(0),
        players: 1,
        ready: 0,
        ..
    } = receive(host).await
    else {
        panic!("expected host lobby")
    };
    (code, setup)
}

async fn join(host: &mut Client, guest: &mut Client, code: &str) -> MatchSetup {
    send(
        guest,
        &ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code: code.to_owned(),
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
            players: 2,
            ..
        }
    ));
    assert!(matches!(
        receive(guest).await,
        ServerMessage::Lobby {
            player: PlayerId(1),
            players: 2,
            ..
        }
    ));
    setup
}

async fn ready(host: &mut Client, guest: &mut Client, checksum: u64) {
    send(host, &ClientMessage::Ready { checksum }).await;
    for client in [&mut *host, &mut *guest] {
        assert!(matches!(
            receive(client).await,
            ServerMessage::Lobby { ready: 1, .. }
        ));
    }
    send(guest, &ClientMessage::Ready { checksum }).await;
    for client in [&mut *host, &mut *guest] {
        assert!(matches!(
            receive(client).await,
            ServerMessage::Lobby { ready: 2, .. }
        ));
    }
}

async fn start(host: &mut Client, guest: &mut Client) {
    send(host, &ClientMessage::Start).await;
    for client in [&mut *host, &mut *guest] {
        assert_eq!(
            receive(client).await,
            ServerMessage::Started {
                players: 2,
                input_delay: 2
            }
        );
    }
}

#[tokio::test]
async fn websocket_server_simulates_moves_rejects_forged_ownership_and_produces_replay_hashes() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    assert_eq!(join(&mut host, &mut guest, &code).await, setup);
    ready(&mut host, &mut guest, setup.checksum()).await;
    start(&mut host, &mut guest).await;
    let mut sim = setup.simulation().unwrap();
    let initial = sim.unit(UnitId(0)).unwrap().pos;
    let mut replay = Replay::new(setup).unwrap();
    send(
        &mut host,
        &ClientMessage::Input {
            tick: 2,
            commands: vec![Command::Move {
                units: vec![UnitId(0)],
                target: IVec2::new(10 * TILE, 8 * TILE),
            }],
        },
    )
    .await;
    send(
        &mut host,
        &ClientMessage::Input {
            tick: 8,
            commands: vec![Command::Stop {
                units: vec![UnitId(0)],
            }],
        },
    )
    .await;
    send(
        &mut guest,
        &ClientMessage::Input {
            tick: 4,
            commands: vec![Command::Stop {
                units: vec![UnitId(0)],
            }],
        },
    )
    .await;
    for tick in 1..=12 {
        let message = receive(&mut host).await;
        assert_eq!(message, receive(&mut guest).await);
        let ServerMessage::Turn { turn, hash } = message else {
            panic!("expected authoritative turn")
        };
        assert_eq!(turn.tick, tick);
        turn.apply(&mut sim).unwrap();
        assert_eq!(sim.state_hash(), hash);
        replay.push(turn, hash).unwrap();
        if tick == 5 {
            assert!(sim.move_target(UnitId(0)).is_some());
        }
    }
    assert_ne!(sim.unit(UnitId(0)).unwrap().pos, initial);
    assert!(sim.move_target(UnitId(0)).is_none());
    assert_eq!(
        Replay::from_ron(&replay.to_ron().unwrap())
            .unwrap()
            .playback()
            .unwrap()
            .state_hash(),
        sim.state_hash()
    );
}

#[tokio::test]
async fn readiness_is_required_and_invalid_joins_or_starts_remain_retryable() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest, mut outsider) = (
        server.connect().await,
        server.connect().await,
        server.connect().await,
    );
    let (code, setup) = create(&mut host).await;
    send(&mut host, &ClientMessage::Start).await;
    expect_error(&mut host).await;
    guest
        .send(Message::Text(
            format!("Join(version:1,code:\"{code}\")").into(),
        ))
        .await
        .unwrap();
    expect_error(&mut guest).await;
    join(&mut host, &mut guest, &code).await;
    send(
        &mut outsider,
        &ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code,
        },
    )
    .await;
    expect_error(&mut outsider).await;
    send(&mut host, &ClientMessage::Start).await;
    expect_error(&mut host).await;
    send(
        &mut guest,
        &ClientMessage::Ready {
            checksum: setup.checksum() ^ 1,
        },
    )
    .await;
    expect_error(&mut guest).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    send(&mut guest, &ClientMessage::Start).await;
    expect_error(&mut guest).await;
    start(&mut host, &mut guest).await;
}

#[tokio::test]
async fn runtime_clock_has_no_initial_turn_and_runs_without_input_even_with_slow_config_poll() {
    let server = Server::new(Config {
        poll_interval: Duration::from_millis(250),
        ..Config::default()
    })
    .await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    let started = Instant::now();
    start(&mut host, &mut guest).await;
    assert!(
        timeout(Duration::from_millis(20), receive(&mut host))
            .await
            .is_err()
    );
    for tick in 1..=6 {
        let message = receive(&mut host).await;
        assert_eq!(message, receive(&mut guest).await);
        let ServerMessage::Turn { turn, .. } = message else {
            panic!("expected turn")
        };
        assert_eq!(turn.tick, tick);
        assert!(turn.inputs.iter().all(|input| input.commands.is_empty()));
        assert!(started.elapsed() >= Duration::from_millis(u64::from(tick) * 50));
    }
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn inputs_cannot_vote_for_speed_change_and_invalid_ticks_are_retryable() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest, mut outsider) = (
        server.connect().await,
        server.connect().await,
        server.connect().await,
    );
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    start(&mut host, &mut guest).await;
    send(
        &mut outsider,
        &ClientMessage::Input {
            tick: 2,
            commands: vec![],
        },
    )
    .await;
    expect_error(&mut outsider).await;
    // The owner and state are never accepted from the wire; Hash is removed too.
    for raw in ["Input(tick:2,player:0,commands:[])", "Hash(tick:1,hash:0)"] {
        guest.send(Message::Text(raw.into())).await.unwrap();
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
    // Invalid submissions are checked by the deterministic coordinator tests; avoid
    // racing the 50 ms clock while checking socket errors here.
    let first = receive(&mut host).await;
    assert_eq!(first, receive(&mut guest).await);
    assert!(matches!(first, ServerMessage::Turn { turn, .. } if turn.tick == 1));
}

#[tokio::test]
async fn socket_heartbeats_keep_game_running_with_no_client_input_or_hashes() {
    let server = Server::new(Config {
        peer_timeout: Duration::from_millis(180),
        ..Config::default()
    })
    .await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    start(&mut host, &mut guest).await;
    for tick in 1..=16 {
        let message = receive(&mut host).await;
        assert_eq!(message, receive(&mut guest).await);
        assert!(matches!(message, ServerMessage::Turn { turn, .. } if turn.tick == tick));
    }
}

#[tokio::test]
async fn large_setup_is_minified_and_compressed_before_lobby() {
    let mut config = Config::default();
    config
        .setup
        .rules
        .push_str(&" ".repeat(MAX_MESSAGE_BYTES * 3));
    config
        .setup
        .map
        .push_str(&" ".repeat(MAX_MESSAGE_BYTES * 3));
    config.setup.seed = 123;
    let expected = config.setup.minified();
    let server = Server::new(config).await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    assert_eq!(setup, expected);
    assert_eq!(join(&mut host, &mut guest, &code).await, expected);
    ready(&mut host, &mut guest, setup.checksum()).await;
    start(&mut host, &mut guest).await;
}

#[tokio::test]
async fn guest_disconnect_updates_lobby_and_oversized_client_input_ends_match() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let (code, setup) = create(&mut host).await;
    join(&mut host, &mut guest, &code).await;
    guest.close(None).await.unwrap();
    assert!(matches!(
        receive(&mut host).await,
        ServerMessage::Lobby {
            players: 1,
            ready: 0,
            ..
        }
    ));
    let mut guest = server.connect().await;
    join(&mut host, &mut guest, &code).await;
    ready(&mut host, &mut guest, setup.checksum()).await;
    start(&mut host, &mut guest).await;
    guest
        .send(Message::Text(" ".repeat(MAX_MESSAGE_BYTES + 1).into()))
        .await
        .unwrap();
    // A close may cross the first tick on slow machines.
    loop {
        match receive(&mut host).await {
            ServerMessage::Ended { .. } => break,
            ServerMessage::Turn { .. } => {}
            message => panic!("unexpected {message:?}"),
        }
    }
}

#[tokio::test]
async fn stalled_handshake_is_bounded_and_releases_connection_capacity() {
    let server = Server::new(Config {
        max_connections: 1,
        handshake_timeout: Duration::from_millis(50),
        ..Config::default()
    })
    .await;
    let _stalled = TcpStream::connect(server.address).await.unwrap();
    sleep(Duration::from_millis(120)).await;
    create(&mut server.connect().await).await;
}

#[tokio::test]
async fn invalid_setup_and_resource_limits_fail_before_accepting_connections() {
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
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let error = serve_with_config(listener, config).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
