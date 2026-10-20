#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rts_base::{Command, IVec2, PlayerId, TILE, UnitId};
use rts_net::{
    ClientMessage, MatchSetup, PROTOCOL_VERSION, Replay, ServerMessage, Turn, decode_server,
    encode_client,
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
    let message = timeout(Duration::from_secs(3), client.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = message else {
        panic!("expected RON text, got {message:?}")
    };
    decode_server(&text).unwrap()
}

async fn expect_error(client: &mut Client) {
    assert!(matches!(receive(client).await, ServerMessage::Error { .. }));
}

fn setup() -> MatchSetup {
    MatchSetup {
        rules: include_str!("../../../assets/data/core.rules.ron").to_owned(),
        map: include_str!("../../../assets/maps/proving_grounds.map.ron").to_owned(),
        seed: 123,
        players: 2,
    }
}

async fn create(host: &mut Client, checksum: u64) -> String {
    send(
        host,
        &ClientMessage::Create {
            version: PROTOCOL_VERSION,
            checksum,
            players: 2,
        },
    )
    .await;
    let ServerMessage::Lobby {
        code,
        player: PlayerId(0),
        players: 1,
        capacity: 2,
    } = receive(host).await
    else {
        panic!("expected host lobby")
    };
    code
}

async fn join(host: &mut Client, guest: &mut Client, code: &str, checksum: u64) {
    send(
        guest,
        &ClientMessage::Join {
            version: PROTOCOL_VERSION,
            code: code.to_owned(),
            checksum,
        },
    )
    .await;
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
}

async fn start(host: &mut Client, guest: &mut Client) -> Turn {
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
    let first = receive(host).await;
    assert_eq!(first, receive(guest).await);
    let ServerMessage::Turn(turn) = first else {
        panic!("expected bootstrap turn")
    };
    turn
}

async fn hash(host: &mut Client, guest: &mut Client, tick: u32, value: u64) {
    for client in [&mut *host, &mut *guest] {
        send(client, &ClientMessage::Hash { tick, hash: value }).await;
    }
    for client in [&mut *host, &mut *guest] {
        assert_eq!(
            receive(client).await,
            ServerMessage::HashConfirmed { tick, hash: value }
        );
    }
}

#[tokio::test]
async fn two_websocket_clients_exchange_moves_empty_frames_hashes_and_replay() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let setup = setup();
    let code = create(&mut host, setup.checksum()).await;
    join(&mut host, &mut guest, &code, setup.checksum()).await;
    let mut turn = start(&mut host, &mut guest).await;
    let mut host_sim = setup.simulation().unwrap();
    let mut guest_sim = setup.simulation().unwrap();
    let initial_position = host_sim.unit(UnitId(0)).unwrap().pos;
    let mut replay = Replay::new(setup).unwrap();
    for tick in 1..=24 {
        if tick > 1 {
            let host_commands = match tick {
                2 => vec![Command::Move {
                    units: vec![UnitId(0)],
                    target: IVec2::new(10 * TILE, 8 * TILE),
                }],
                12 => vec![Command::Stop {
                    units: vec![UnitId(0)],
                }],
                _ => Vec::new(),
            };
            let guest_commands = if tick == 2 {
                vec![Command::Move {
                    units: vec![UnitId(7)],
                    target: IVec2::new(38 * TILE, 24 * TILE),
                }]
            } else {
                Vec::new()
            };
            // The relay waits for both slots, even if the guest submits first.
            send(
                &mut guest,
                &ClientMessage::Input {
                    tick,
                    commands: guest_commands,
                },
            )
            .await;
            send(
                &mut host,
                &ClientMessage::Input {
                    tick,
                    commands: host_commands,
                },
            )
            .await;
            let message = receive(&mut host).await;
            assert_eq!(message, receive(&mut guest).await);
            let ServerMessage::Turn(next) = message else {
                panic!("expected agreed turn")
            };
            turn = next;
        }
        turn.validate(tick, 2).unwrap();
        turn.apply(&mut host_sim).unwrap();
        turn.apply(&mut guest_sim).unwrap();
        let state_hash = host_sim.state_hash();
        assert_eq!(state_hash, guest_sim.state_hash());
        replay.push(turn.clone(), state_hash).unwrap();
        hash(&mut host, &mut guest, tick, state_hash).await;
    }
    assert_ne!(host_sim.unit(UnitId(0)).unwrap().pos, initial_position);
    assert!(host_sim.move_target(UnitId(0)).is_none());
    let recording = Replay::from_ron(&replay.to_ron().unwrap()).unwrap();
    assert_eq!(
        recording.playback().unwrap().state_hash(),
        host_sim.state_hash()
    );
}

#[tokio::test]
async fn websocket_bad_joins_full_room_and_unauthorized_start_are_retryable() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest, mut outsider) = (
        server.connect().await,
        server.connect().await,
        server.connect().await,
    );
    let code = create(&mut host, 17).await;
    send(&mut host, &ClientMessage::Start).await;
    expect_error(&mut host).await;
    // Use raw RON for a version rejected by the protocol encoder itself.
    guest
        .send(Message::Text(
            format!("Join(version:99,code:\"{code}\",checksum:17)").into(),
        ))
        .await
        .unwrap();
    expect_error(&mut guest).await;
    send(
        &mut guest,
        &ClientMessage::Join {
            version: 1,
            code: code.clone(),
            checksum: 18,
        },
    )
    .await;
    expect_error(&mut guest).await;
    join(&mut host, &mut guest, &code, 17).await;
    send(
        &mut outsider,
        &ClientMessage::Join {
            version: 1,
            code: code.clone(),
            checksum: 17,
        },
    )
    .await;
    expect_error(&mut outsider).await;
    send(&mut guest, &ClientMessage::Start).await;
    expect_error(&mut guest).await;
    start(&mut host, &mut guest).await;
}

#[tokio::test]
async fn websocket_unauthorized_duplicate_and_future_inputs_are_retryable() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest, mut outsider) = (
        server.connect().await,
        server.connect().await,
        server.connect().await,
    );
    let code = create(&mut host, 17).await;
    join(&mut host, &mut guest, &code, 17).await;
    start(&mut host, &mut guest).await;
    send(
        &mut outsider,
        &ClientMessage::Input {
            tick: 2,
            commands: Vec::new(),
        },
    )
    .await;
    expect_error(&mut outsider).await;
    // Slots are never accepted from the wire.
    guest
        .send(Message::Text("Input(tick:2,player:0,commands:[])".into()))
        .await
        .unwrap();
    expect_error(&mut guest).await;
    for tick in [3, u32::MAX] {
        send(
            &mut guest,
            &ClientMessage::Input {
                tick,
                commands: Vec::new(),
            },
        )
        .await;
        expect_error(&mut guest).await;
    }
    send(
        &mut guest,
        &ClientMessage::Input {
            tick: 2,
            commands: Vec::new(),
        },
    )
    .await;
    send(
        &mut guest,
        &ClientMessage::Input {
            tick: 2,
            commands: Vec::new(),
        },
    )
    .await;
    expect_error(&mut guest).await;
    // No turn is emitted while the host is still missing.
    assert!(
        timeout(Duration::from_millis(30), host.next())
            .await
            .is_err()
    );
    send(
        &mut host,
        &ClientMessage::Input {
            tick: 2,
            commands: Vec::new(),
        },
    )
    .await;
    assert!(matches!(
        receive(&mut host).await,
        ServerMessage::Turn(Turn { tick: 2, .. })
    ));
    assert!(matches!(
        receive(&mut guest).await,
        ServerMessage::Turn(Turn { tick: 2, .. })
    ));
}

#[tokio::test]
async fn websocket_hash_mismatch_ends_both_clients_and_connection_can_rejoin() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let code = create(&mut host, 17).await;
    join(&mut host, &mut guest, &code, 17).await;
    start(&mut host, &mut guest).await;
    send(&mut host, &ClientMessage::Hash { tick: 1, hash: 9 }).await;
    send(&mut guest, &ClientMessage::Hash { tick: 1, hash: 10 }).await;
    for client in [&mut host, &mut guest] {
        assert!(
            matches!(receive(client).await, ServerMessage::Ended { reason } if reason.contains("mismatch"))
        );
    }
    let code = create(&mut host, 18).await;
    join(&mut host, &mut guest, &code, 18).await;
}

#[tokio::test]
async fn websocket_disconnect_updates_lobby_but_ends_running_match() {
    let server = Server::new(Config::default()).await;
    let (mut host, mut guest) = (server.connect().await, server.connect().await);
    let code = create(&mut host, 17).await;
    join(&mut host, &mut guest, &code, 17).await;
    guest.close(None).await.unwrap();
    assert!(matches!(
        receive(&mut host).await,
        ServerMessage::Lobby { players: 1, .. }
    ));
    let mut replacement = server.connect().await;
    join(&mut host, &mut replacement, &code, 17).await;
    start(&mut host, &mut replacement).await;
    replacement.close(None).await.unwrap();
    assert!(matches!(
        receive(&mut host).await,
        ServerMessage::Ended { .. }
    ));
}

#[tokio::test]
async fn websocket_stalled_match_times_out_while_waiting_lobby_survives() {
    let server = Server::new(Config {
        peer_timeout: Duration::from_millis(180),
        poll_interval: Duration::from_millis(5),
        ..Config::default()
    })
    .await;
    let mut host = server.connect().await;
    let code = create(&mut host, 17).await;
    sleep(Duration::from_millis(240)).await;
    let mut guest = server.connect().await;
    join(&mut host, &mut guest, &code, 17).await;
    start(&mut host, &mut guest).await;
    // Both sockets remain responsive while their match fails to produce a turn/hash.
    for _ in 0..3 {
        send(&mut host, &ClientMessage::Start).await;
        send(&mut guest, &ClientMessage::Start).await;
        expect_error(&mut host).await;
        expect_error(&mut guest).await;
        sleep(Duration::from_millis(40)).await;
    }
    for client in [&mut host, &mut guest] {
        assert!(matches!(receive(client).await, ServerMessage::Ended { .. }));
    }
}

#[tokio::test]
async fn stalled_handshake_is_bounded_and_releases_connection_capacity() {
    let server = Server::new(Config {
        max_connections: 1,
        handshake_timeout: Duration::from_millis(50),
        poll_interval: Duration::from_millis(5),
        ..Config::default()
    })
    .await;
    let _stalled = TcpStream::connect(server.address).await.unwrap();
    sleep(Duration::from_millis(120)).await;
    let mut host = server.connect().await;
    create(&mut host, 17).await;
}
