use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use rts_net::{
    ClientMessage, MAX_MESSAGE_BYTES, MAX_SERVER_MESSAGE_BYTES, NetError, ServerFrame,
    ServerMessage, decode_client, encode_server,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Semaphore, mpsc},
    task::JoinSet,
    time::{MissedTickBehavior, interval, timeout},
};
use tokio_tungstenite::{
    accept_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use crate::{ClientId, Config, Coordinator, Delivery};

enum Event {
    Message(ClientId, ClientMessage),
    Invalid(ClientId, &'static str),
    Activity(ClientId),
}

/// Runs the plain-WebSocket server with default limits until cancelled.
///
/// # Errors
/// Returns a TCP listener accept error. All connection tasks are aborted when
/// this future is dropped, including connections still in their handshake.
pub async fn serve(listener: TcpListener) -> io::Result<()> {
    serve_with_config(listener, Config::default()).await
}

/// Runs the server with explicit resource limits and timeouts.
///
/// # Errors
/// Returns an error for invalid limits or a TCP listener accept error.
pub async fn serve_with_config(listener: TcpListener, config: Config) -> io::Result<()> {
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
    let setup_frame = encode_message(&ServerMessage::Setup(config.setup.clone()))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let mut coordinator = Coordinator::new(config.clone(), rand::make_rng());
    let config = Arc::new(config);
    let (events_tx, mut events_rx) = mpsc::channel(config.event_queue_capacity);
    let mut outbound = BTreeMap::new();
    let mut tasks = JoinSet::new();
    let mut timer = interval(config.poll_interval.min(Duration::from_millis(10)));
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let started = Instant::now();
    let mut next_id: ClientId = 0;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                if tasks.len() >= config.max_connections { continue; }
                let Some(id) = next_id.checked_add(1) else {
                    return Err(io::Error::other("connection identifiers exhausted"));
                };
                next_id = id;
                if !coordinator.connect(id, started.elapsed()) { continue; }
                let (tx, rx) = mpsc::channel(config.outbound_queue_capacity);
                outbound.insert(id, tx);
                let events = events_tx.clone();
                let config = config.clone();
                tasks.spawn(async move {
                    connection(stream, id, events, rx, &config).await;
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
    outbound: &mut BTreeMap<ClientId, mpsc::Sender<Message>>,
    setup_frame: &Message,
) {
    let mut pending = VecDeque::from(deliveries);
    while let Some(Delivery { client, message }) = pending.pop_front() {
        let Some(sender) = outbound.get(&client) else {
            continue;
        };
        let encoded = if matches!(message, ServerMessage::Setup(_)) {
            Ok(setup_frame.clone())
        } else {
            encode_message(&message)
        };
        if encoded.is_err() || sender.try_send(encoded.expect("checked encoding")).is_err() {
            outbound.remove(&client);
            pending.extend(coordinator.disconnect(client));
        }
    }
}

fn encode_message(message: &ServerMessage) -> Result<Message, NetError> {
    encode_server(message).map(|frame| match frame {
        ServerFrame::Text(text) => Message::Text(text.into()),
        ServerFrame::Binary(bytes) => Message::Binary(bytes.into()),
    })
}

async fn connection(
    stream: TcpStream,
    id: ClientId,
    events: mpsc::Sender<Event>,
    mut outbound: mpsc::Receiver<Message>,
    config: &Config,
) {
    let websocket_config = WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(8192)
        .max_write_buffer_size(MAX_SERVER_MESSAGE_BYTES * 2)
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let Ok(Ok(mut socket)) = timeout(
        config.handshake_timeout,
        accept_async_with_config(stream, Some(websocket_config)),
    )
    .await
    else {
        return;
    };
    // Socket liveness is independent of gameplay input. Browsers automatically pong.
    let mut heartbeat = interval((config.peer_timeout / 3).max(Duration::from_millis(1)));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        // Removing a sender disconnects this peer. Do not drain queued frames
        // over a slow socket after the coordinator has already removed it.
        if outbound.is_closed() {
            return;
        }
        tokio::select! {
            _ = heartbeat.tick() => {
                if !matches!(timeout(config.peer_timeout, socket.send(Message::Ping(Vec::new().into()))).await, Ok(Ok(()))) { return; }
            }
            incoming = socket.next() => {
                let event = match incoming {
                    Some(Ok(Message::Text(text))) => match decode_client(&text) {
                        Ok(message) => Event::Message(id, message),
                        Err(NetError::Invalid(message)) if message == "protocol version does not match" =>
                            Event::Invalid(id, "protocol version does not match"),
                        Err(_) => Event::Invalid(id, "invalid or oversized protocol message"),
                    },
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                        // Flush tungstenite's automatic pong. A blocked writer cannot retain
                        // a task indefinitely.
                        if !matches!(timeout(config.peer_timeout, socket.flush()).await, Ok(Ok(()))) { return; }
                        Event::Activity(id)
                    }
                    Some(Ok(Message::Binary(_))) => Event::Invalid(id, "invalid or oversized protocol message"),
                    Some(Ok(Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                };
                // A saturated coordinator queue disconnects the producer instead of
                // accumulating messages or blocking every other socket.
                if events.try_send(event).is_err() { return; }
            }
            message = outbound.recv() => {
                let Some(message) = message else { return };
                if !matches!(timeout(config.peer_timeout, socket.send(message)).await, Ok(Ok(()))) { return; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;

    use super::*;

    #[test]
    fn setup_deliveries_share_the_cached_frame() {
        let config = Config::default();
        let setup_frame = encode_message(&ServerMessage::Setup(config.setup.clone())).unwrap();
        let mut coordinator =
            Coordinator::new(config.clone(), rand::rngs::StdRng::seed_from_u64(0));
        assert!(coordinator.connect(1, Duration::ZERO));
        let (tx, mut rx) = mpsc::channel(2);
        let mut outbound = BTreeMap::from([(1, tx)]);
        dispatch(
            (0..2)
                .map(|_| Delivery {
                    client: 1,
                    message: ServerMessage::Setup(config.setup.clone()),
                })
                .collect(),
            &mut coordinator,
            &mut outbound,
            &setup_frame,
        );
        let Message::Binary(cached) = setup_frame else {
            panic!("expected binary setup")
        };
        for _ in 0..2 {
            let Message::Binary(delivered) = rx.try_recv().unwrap() else {
                panic!("expected binary setup")
            };
            assert_eq!(cached, delivered);
            assert_eq!(cached.as_ptr(), delivered.as_ptr());
        }
    }

    #[test]
    fn outbound_overflow_disconnects_host_and_notifies_guest_in_the_same_pass() {
        let config = Config::default();
        let setup_frame = encode_message(&ServerMessage::Setup(config.setup.clone())).unwrap();
        let mut coordinator = Coordinator::new(config, rand::rngs::StdRng::seed_from_u64(0));
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
        host_tx.try_send(Message::Text("queued".into())).unwrap();
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
        let Message::Text(text) = guest_rx.try_recv().unwrap() else {
            panic!("expected room end")
        };
        assert!(matches!(
            rts_net::decode_server(&text).unwrap(),
            ServerMessage::Ended { .. }
        ));
    }

    #[tokio::test]
    async fn disconnected_connection_discards_queued_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel(1);
        tx.try_send(Message::Text("stale application message".into()))
            .unwrap();
        drop(tx);
        let (events, _events_rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            connection(stream, 1, events, rx, &Config::default()).await;
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        timeout(Duration::from_secs(2), async {
            while let Some(Ok(message)) = client.next().await {
                assert!(!matches!(message, Message::Text(_) | Message::Binary(_)));
            }
        })
        .await
        .unwrap();
        task.await.unwrap();
    }
}
