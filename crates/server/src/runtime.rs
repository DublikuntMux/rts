use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use rts_net::{
    ClientMessage, MAX_MESSAGE_BYTES, MAX_SERVER_MESSAGE_BYTES, ServerFrame, ServerMessage,
    decode_client, encode_server,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
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
    Invalid(ClientId),
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
        || config.outbound_queue_capacity == 0
        || config.peer_timeout.is_zero()
        || config.lobby_timeout.is_zero()
        || config.handshake_timeout.is_zero()
        || config.poll_interval.is_zero()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "server limits and timeouts must be positive",
        ));
    }
    config
        .setup
        .simulation()
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
                    Some(Event::Invalid(id)) => vec![Delivery { client: id, message: ServerMessage::Error {
                        message: "invalid or oversized protocol message".into(),
                    }}],
                    Some(Event::Activity(id)) => {
                        coordinator.touch(id, started.elapsed());
                        Vec::new()
                    }
                    None => return Err(io::Error::other("server event channel closed")),
                };
                dispatch(deliveries, &mut coordinator, &mut outbound);
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok(id)) = completed {
                    outbound.remove(&id);
                    let deliveries = coordinator.disconnect(id);
                    dispatch(deliveries, &mut coordinator, &mut outbound);
                }
            }
            _ = timer.tick() => {
                let now = started.elapsed();
                let mut deliveries = coordinator.expire(now);
                deliveries.extend(coordinator.advance(now));
                dispatch(deliveries, &mut coordinator, &mut outbound);
                outbound.retain(|id, _| coordinator.is_connected(*id));
            }
        }
    }
}

fn dispatch(
    deliveries: Vec<Delivery>,
    coordinator: &mut Coordinator,
    outbound: &mut BTreeMap<ClientId, mpsc::Sender<Message>>,
) {
    let mut pending = VecDeque::from(deliveries);
    while let Some(Delivery { client, message }) = pending.pop_front() {
        let Some(sender) = outbound.get(&client) else {
            continue;
        };
        let encoded = encode_server(&message).map(|frame| match frame {
            ServerFrame::Text(text) => Message::Text(text.into()),
            ServerFrame::Binary(bytes) => Message::Binary(bytes.into()),
        });
        if encoded.is_err() || sender.try_send(encoded.expect("checked encoding")).is_err() {
            outbound.remove(&client);
            pending.extend(coordinator.disconnect(client));
        }
    }
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
        tokio::select! {
            _ = heartbeat.tick() => {
                if !matches!(timeout(config.peer_timeout, socket.send(Message::Ping(Vec::new().into()))).await, Ok(Ok(()))) { return; }
            }
            incoming = socket.next() => {
                let event = match incoming {
                    Some(Ok(Message::Text(text))) => decode_client(&text)
                        .map_or(Event::Invalid(id), |message| Event::Message(id, message)),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                        // Flush tungstenite's automatic pong. A blocked writer cannot retain
                        // a task indefinitely.
                        if !matches!(timeout(config.peer_timeout, socket.flush()).await, Ok(Ok(()))) { return; }
                        Event::Activity(id)
                    }
                    Some(Ok(Message::Binary(_))) => Event::Invalid(id),
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
