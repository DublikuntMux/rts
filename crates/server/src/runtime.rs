use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use rts_net::{
    ClientMessage, MAX_MESSAGE_BYTES, NetError, ServerMessage, decode_client, encode_server,
    quic::{
        PRIORITY_GAME, STREAM_GAME,
        quinn::{Connection, Endpoint, Incoming},
        read_frame, write_frame,
    },
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
    time::{MissedTickBehavior, interval, timeout},
};

use crate::{ClientId, Config, Coordinator, Delivery};

enum Event {
    Message(ClientId, ClientMessage),
    Invalid(ClientId, &'static str),
    Activity(ClientId),
}

/// Runs the QUIC server with default limits until cancelled.
///
/// # Errors
/// Returns an error if the QUIC endpoint closes unexpectedly. All connection tasks are aborted
/// when this future is dropped, including connections still in their handshake.
pub async fn serve(endpoint: Endpoint) -> io::Result<()> {
    serve_with_config(endpoint, Config::default()).await
}

/// Runs the QUIC server with explicit resource limits and timeouts.
///
/// # Errors
/// Returns an error for invalid limits or an endpoint accept error.
pub async fn serve_with_config(endpoint: Endpoint, config: Config) -> io::Result<()> {
    if config.max_connections == 0
        || config.max_rooms == 0
        || config.event_queue_capacity == 0
        || config.event_queue_capacity > Semaphore::MAX_PERMITS
        || config.outbound_queue_capacity == 0
        || config.outbound_queue_capacity > Semaphore::MAX_PERMITS
        || config.peer_timeout.is_zero()
        || config.lobby_timeout.is_zero()
        || config.handshake_timeout.is_zero()
        || config.poll_interval.is_zero()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "server limits and timeouts must be positive, and queue capacities must fit Tokio's semaphore limit",
        ));
    }
    // Setup is immutable for the lifetime of the server. Validate and compress it
    // once, rather than rebuilding a simulation on the coordinator task per join.
    let mut coordinator = Coordinator::new(config.clone(), rand::make_rng());
    if !coordinator.can_host() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the server rules and map cannot start a match",
        ));
    }
    let setup_frame: Arc<[u8]> =
        encode_server(&ServerMessage::Setup(coordinator.lobby_setup().clone()))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
            .into();
    let config = Arc::new(config);
    let (events_tx, mut events_rx) = mpsc::channel(config.event_queue_capacity);
    let mut outbound: BTreeMap<ClientId, mpsc::Sender<Arc<[u8]>>> = BTreeMap::new();
    let mut tasks = JoinSet::new();
    let mut timer = interval(config.poll_interval.min(Duration::from_millis(10)));
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let started = Instant::now();
    let mut next_id: ClientId = 0;
    loop {
        tokio::select! {
            accepted = endpoint.accept() => {
                let Some(incoming) = accepted else {
                    return Ok(());
                };
                if tasks.len() >= config.max_connections {
                    incoming.refuse();
                    continue;
                }
                let Some(id) = next_id.checked_add(1) else {
                    return Err(io::Error::other("connection identifiers exhausted"));
                };
                next_id = id;
                if !coordinator.connect(id, started.elapsed()) {
                    incoming.refuse();
                    continue;
                }
                let (tx, rx) = mpsc::channel(config.outbound_queue_capacity);
                outbound.insert(id, tx);
                let events = events_tx.clone();
                let config = config.clone();
                tasks.spawn(async move {
                    connection(incoming, id, events, rx, &config).await;
                    id
                });
            }
            event = events_rx.recv() => {
                let deliveries = match event {
                    Some(Event::Message(id, message)) => coordinator.receive(id, message, started.elapsed()),
                    Some(Event::Invalid(id, message)) => vec![Delivery { client: id, message: ServerMessage::Error {
                        message: message.into(),
                    }}],
                    Some(Event::Activity(id)) => {
                        coordinator.touch(id, started.elapsed());
                        Vec::new()
                    }
                    None => return Err(io::Error::other("server event channel closed")),
                };
                dispatch(deliveries, &mut coordinator, &mut outbound, &setup_frame);
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok(id)) = completed {
                    outbound.remove(&id);
                    let deliveries = coordinator.disconnect(id);
                    dispatch(deliveries, &mut coordinator, &mut outbound, &setup_frame);
                }
            }
            _ = timer.tick() => {
                let now = started.elapsed();
                let mut deliveries = coordinator.expire(now);
                deliveries.extend(coordinator.advance(now));
                dispatch(deliveries, &mut coordinator, &mut outbound, &setup_frame);
                outbound.retain(|id, _| coordinator.is_connected(*id));
            }
        }
    }
}

fn dispatch(
    deliveries: Vec<Delivery>,
    coordinator: &mut Coordinator,
    outbound: &mut BTreeMap<ClientId, mpsc::Sender<Arc<[u8]>>>,
    setup_frame: &Arc<[u8]>,
) {
    let mut pending = VecDeque::from(deliveries);
    while let Some(Delivery { client, message }) = pending.pop_front() {
        let Some(sender) = outbound.get(&client) else {
            continue;
        };
        let encoded: Result<Arc<[u8]>, NetError> = if matches!(message, ServerMessage::Setup(_)) {
            Ok(Arc::clone(setup_frame))
        } else {
            encode_server(&message).map(Into::into)
        };
        if encoded.is_err() || sender.try_send(encoded.expect("checked encoding")).is_err() {
            outbound.remove(&client);
            pending.extend(coordinator.disconnect(client));
        }
    }
}

async fn connection(
    incoming: Incoming,
    id: ClientId,
    events: mpsc::Sender<Event>,
    outbound: mpsc::Receiver<Arc<[u8]>>,
    config: &Config,
) {
    let Ok(Ok(conn)) = timeout(config.handshake_timeout, incoming).await else {
        return;
    };
    let Ok(Ok((send, mut recv))) = timeout(config.handshake_timeout, conn.accept_bi()).await else {
        conn.close(1_u32.into(), b"handshake timeout");
        return;
    };
    let mut kind = [0_u8; 1];
    if !matches!(
        timeout(config.handshake_timeout, recv.read_exact(&mut kind)).await,
        Ok(Ok(()))
    ) || kind[0] != STREAM_GAME
    {
        conn.close(1_u32.into(), b"expected game stream");
        return;
    }
    let _ = send.set_priority(PRIORITY_GAME);
    run_game_stream(conn, send, recv, id, events, outbound, config).await;
}

async fn run_game_stream(
    conn: Connection,
    mut send: rts_net::quic::quinn::SendStream,
    mut recv: rts_net::quic::quinn::RecvStream,
    id: ClientId,
    events: mpsc::Sender<Event>,
    mut outbound: mpsc::Receiver<Arc<[u8]>>,
    config: &Config,
) {
    // QUIC keep-alive packets keep the transport open; periodically notify the coordinator while
    // the connection stays alive so silent players never hit the peer timeout during gameplay.
    let mut liveness = interval((config.peer_timeout / 3).max(Duration::from_millis(1)));
    liveness.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        // Removing a sender disconnects this peer. Do not drain queued frames
        // over a slow connection after the coordinator has already removed it.
        if outbound.is_closed() {
            conn.close(0_u32.into(), b"disconnected");
            return;
        }
        tokio::select! {
            _ = liveness.tick() => {
                if conn.close_reason().is_some() {
                    return;
                }
                if events.try_send(Event::Activity(id)).is_err() {
                    conn.close(1_u32.into(), b"overloaded");
                    return;
                }
            }
            incoming = read_frame(&mut recv, MAX_MESSAGE_BYTES) => {
                let Ok(Some(bytes)) = incoming else {
                    conn.close(0_u32.into(), b"closed");
                    return;
                };
                let event = match decode_client(&bytes) {
                    Ok(message) => Event::Message(id, message),
                    Err(NetError::Invalid(message)) if message == "protocol version does not match" =>
                        Event::Invalid(id, "protocol version does not match"),
                    Err(_) => Event::Invalid(id, "invalid or oversized protocol message"),
                };
                if events.try_send(event).is_err() {
                    conn.close(1_u32.into(), b"overloaded");
                    return;
                }
            }
            frame = outbound.recv() => {
                let Some(frame) = frame else {
                    conn.close(0_u32.into(), b"disconnected");
                    return;
                };
                if !matches!(timeout(config.peer_timeout, write_frame(&mut send, &frame)).await, Ok(Ok(()))) {
                    conn.close(1_u32.into(), b"write timeout");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;

    use rand::SeedableRng;
    use rts_net::quic::{ServerCertificate, client_endpoint, server_endpoint};

    use super::*;

    #[test]
    fn setup_deliveries_share_the_cached_frame() {
        let config = Config::default();
        let mut coordinator = Coordinator::new(config, rand::rngs::StdRng::seed_from_u64(0));
        let lobby = coordinator.lobby_setup().clone();
        let setup_frame: Arc<[u8]> = encode_server(&ServerMessage::Setup(lobby.clone()))
            .unwrap()
            .into();
        assert!(coordinator.connect(1, Duration::ZERO));
        let (tx, mut rx) = mpsc::channel(2);
        let mut outbound = BTreeMap::from([(1, tx)]);
        dispatch(
            (0..2)
                .map(|_| Delivery {
                    client: 1,
                    message: ServerMessage::Setup(lobby.clone()),
                })
                .collect(),
            &mut coordinator,
            &mut outbound,
            &setup_frame,
        );
        for _ in 0..2 {
            let delivered = rx.try_recv().unwrap();
            assert_eq!(setup_frame, delivered);
            assert_eq!(setup_frame.as_ptr(), delivered.as_ptr());
        }
    }

    #[test]
    fn outbound_overflow_disconnects_host_and_notifies_guest_in_the_same_pass() {
        let mut coordinator =
            Coordinator::new(Config::default(), rand::rngs::StdRng::seed_from_u64(0));
        let setup_frame: Arc<[u8]> =
            encode_server(&ServerMessage::Setup(coordinator.lobby_setup().clone()))
                .unwrap()
                .into();
        for id in 1..=2 {
            assert!(coordinator.connect(id, Duration::ZERO));
        }
        let created = coordinator.receive(
            1,
            ClientMessage::Create {
                version: rts_net::PROTOCOL_VERSION,
                name: "Host".into(),
                public: true,
            },
            Duration::ZERO,
        );
        let code = created
            .into_iter()
            .find_map(|delivery| match delivery.message {
                ServerMessage::Lobby { code, .. } => Some(code),
                _ => None,
            })
            .unwrap();
        coordinator.receive(
            2,
            ClientMessage::Join {
                version: rts_net::PROTOCOL_VERSION,
                code,
                name: "Guest".into(),
            },
            Duration::ZERO,
        );
        let (host_tx, host_rx) = mpsc::channel(1);
        host_tx.try_send(Arc::from(&b"queued"[..])).unwrap();
        let (guest_tx, mut guest_rx) = mpsc::channel(1);
        let mut outbound = BTreeMap::from([(1, host_tx), (2, guest_tx)]);
        dispatch(
            vec![Delivery {
                client: 1,
                message: ServerMessage::Error {
                    message: "overflow".into(),
                },
            }],
            &mut coordinator,
            &mut outbound,
            &setup_frame,
        );
        assert!(!coordinator.is_connected(1));
        assert!(coordinator.is_connected(2));
        assert!(host_rx.is_closed());
        assert!(!outbound.contains_key(&1));
        let bytes = guest_rx.try_recv().unwrap();
        assert!(matches!(
            rts_net::decode_server(&bytes).unwrap(),
            ServerMessage::Ended { .. }
        ));
    }

    #[tokio::test]
    async fn disconnected_connection_discards_queued_frames() {
        let cert = ServerCertificate::generate_self_signed().unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let server = server_endpoint(socket, &cert, Duration::from_secs(5)).unwrap();
        let (tx, rx) = mpsc::channel(1);
        tx.try_send(Arc::from(&b"stale application message"[..]))
            .unwrap();
        drop(tx);
        let (events, _events_rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let incoming = server.accept().await.unwrap();
            connection(incoming, 1, events, rx, &Config::default()).await;
        });
        let client = client_endpoint(false, None, Duration::from_secs(5)).unwrap();
        let conn = client.connect(address, "rts.local").unwrap().await.unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(&[STREAM_GAME]).await.unwrap();
        let result = timeout(Duration::from_secs(2), read_frame(&mut recv, 1024))
            .await
            .unwrap();
        assert!(!matches!(result, Ok(Some(_))));
        task.await.unwrap();
    }
}
