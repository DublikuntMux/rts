//! Match sessions: local delayed input, server-authoritative turns, and replay playback.
//!
//! The session owns the server connection. Online play connects first, lists rooms, and then
//! creates or joins a room on the same connection. [`SessionStatus`] is the read-only view the
//! interface draws from; the interface changes it only by sending [`SessionAction`]s.

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    ops::ControlFlow,
    sync::mpsc::{self, Receiver, TryRecvError},
};

use bevy::prelude::*;
use ewebsock::{WsEvent, WsMessage, WsSender};
use rts_base::{Command, PlayerId, Sim};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, LobbySlot, MAX_SERVER_MESSAGE_BYTES, MatchSetup,
    PROTOCOL_VERSION, PlayerInput, Replay, RoomInfo, ServerMessage, Turn, decode_server,
    decode_server_binary, encode_client,
};

use crate::{
    game::{AppState, MapCatalog, Match},
    input::Selection,
    library::{Library, Shelf},
    replay_io::ReplayIo,
};

#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub enum SessionAction {
    /// Starts a solo match on a catalog map.
    NewGame {
        map: usize,
    },
    /// Continues a saved solo match.
    LoadGame {
        id: String,
    },
    /// Saves the running solo match.
    SaveGame,
    Connect {
        url: String,
    },
    Disconnect,
    RefreshRooms,
    CreateRoom {
        name: String,
        public: bool,
    },
    JoinRoom {
        name: String,
        code: String,
    },
    SetReady(bool),
    Start,
    LeaveRoom,
    /// Leaves the running or stopped match for the menu.
    Leave,
    WatchReplay {
        id: String,
    },
    /// Watches the most recent match, saved or not.
    WatchLast,
    /// Adds the most recent match to the replay library.
    SaveReplay,
    Delete {
        shelf: Shelf,
        id: String,
    },
    /// Adds a replay file chosen by the player (browser only).
    ImportReplay,
    /// Downloads a stored replay (browser only).
    ExportReplay {
        id: String,
    },
}

/// The server connection as the interface sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Server {
    #[default]
    Offline,
    Connecting {
        url: String,
    },
    Connected {
        url: String,
    },
    Failed {
        url: String,
        reason: String,
    },
}

/// The room this client is in, before its match starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lobby {
    pub code: String,
    /// This client's slot.
    pub player: u8,
    pub public: bool,
    pub slots: Vec<Option<LobbySlot>>,
}

impl Lobby {
    pub const fn is_host(&self) -> bool {
        self.player == 0
    }

    pub fn ready(&self) -> bool {
        self.slots
            .get(usize::from(self.player))
            .and_then(Option::as_ref)
            .is_some_and(|slot| slot.ready)
    }

    pub fn host(&self) -> &str {
        self.slots
            .first()
            .and_then(Option::as_ref)
            .map_or("", |slot| slot.name.as_str())
    }

    pub fn joined(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn all_ready(&self) -> bool {
        self.slots
            .iter()
            .all(|slot| slot.as_ref().is_some_and(|slot| slot.ready))
    }

    /// The host may start once every slot is filled and ready.
    pub fn can_start(&self) -> bool {
        self.is_host() && self.all_ready()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchKind {
    #[default]
    Solo,
    Online,
    Playback,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "interface flags project the session state; Mode is the actual state machine"
)]
pub struct SessionStatus {
    /// The latest result or failure, shown to the player.
    pub message: String,
    pub error: bool,
    pub server: Server,
    /// Public rooms from the last listing.
    pub rooms: Vec<RoomInfo>,
    pub lobby: Option<Lobby>,
    /// A create or join request awaits the server.
    pub joining: bool,
    /// A match is on screen.
    pub running: bool,
    /// The match on screen no longer advances or accepts orders.
    pub stopped: bool,
    pub kind: MatchKind,
    /// A recorded match exists: the current one or the last one played.
    pub has_replay: bool,
    /// A replay file is being imported.
    pub importing: bool,
}

impl SessionStatus {
    pub fn info(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.error = false;
    }

    pub fn fail(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.error = true;
    }

    pub const fn connecting(&self) -> bool {
        matches!(self.server, Server::Connecting { .. })
    }

    pub const fn connected(&self) -> bool {
        matches!(self.server, Server::Connected { .. })
    }

    /// The solo match on screen can be saved.
    pub fn can_save_game(&self) -> bool {
        self.running && !self.stopped && self.kind == MatchKind::Solo
    }

    /// Clears the match view after leaving it.
    const fn leave_match(&mut self) {
        self.running = false;
        self.stopped = false;
        self.kind = MatchKind::Solo;
    }

    fn start_match(&mut self, kind: MatchKind, message: &str) {
        self.running = true;
        self.stopped = false;
        self.kind = kind;
        self.has_replay = true;
        self.info(message);
    }

    fn disconnect(&mut self, server: Server) {
        self.server = server;
        self.rooms.clear();
        self.lobby = None;
        self.joining = false;
    }
}

#[derive(Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Idle,
    Offline,
    Online,
    Playback,
    Stopped,
}

#[derive(Resource, Default)]
pub struct Session {
    mode: Mode,
    local_frames: BTreeMap<u32, Vec<Command>>,
    turns: BTreeMap<u32, (Turn, u64)>,
    received_tick: u32,
    online_setup: Option<MatchSetup>,
    online_players: u8,
    submitted_tick: u32,
    recording: Option<Replay>,
    last_replay: Option<Replay>,
    playback: Option<Replay>,
    last_step: f64,
}

impl Session {
    fn replay(&self) -> Option<&Replay> {
        self.recording.as_ref().or(self.last_replay.as_ref())
    }

    fn archive(&mut self) {
        if let Some(replay) = self.recording.take() {
            self.last_replay = Some(replay);
        }
        self.local_frames.clear();
        self.turns.clear();
        self.online_setup = None;
        self.submitted_tick = 0;
        self.received_tick = 0;
        self.playback = None;
    }

    fn begin(
        &mut self,
        setup: MatchSetup,
        mode: Mode,
        player: PlayerId,
        now: f64,
    ) -> Result<Match, String> {
        let sim = setup
            .simulation()
            .map_err(|error| format!("Cannot start this map: {error}"))?;
        let recording =
            Replay::new(setup).map_err(|error| format!("Cannot record match: {error}"))?;
        self.archive();
        self.recording = Some(recording);
        self.mode = mode;
        self.last_step = now;
        Ok(Match::new(sim, player, true))
    }

    /// Continues a saved solo match from `sim`, its verified state after the recorded turns.
    fn resume(&mut self, saved: Replay, sim: Sim, now: f64) -> Match {
        self.archive();
        self.recording = Some(saved);
        self.mode = Mode::Offline;
        self.last_step = now;
        Match::new(sim, PlayerId(0), true)
    }

    /// Watches a replay that is known to be valid: recorded here or verified by the library.
    fn begin_replay(&mut self, replay: Replay, now: f64) -> Result<Match, String> {
        let sim = replay
            .setup
            .simulation()
            .map_err(|error| format!("Cannot load replay map: {error}"))?;
        self.archive();
        self.last_replay = Some(replay.clone());
        self.playback = Some(replay);
        self.mode = Mode::Playback;
        self.last_step = now;
        Ok(Match::new(sim, PlayerId(0), false))
    }
}

struct Connection {
    sender: WsSender,
    receiver: Receiver<WsEvent>,
    hello: Option<ClientMessage>,
    opened: bool,
    created: f64,
}

impl Connection {
    fn connect(url: &str, hello: ClientMessage, now: f64) -> Result<Self, String> {
        let url = url.trim();
        if url.len() > 2048 || !(url.starts_with("ws://") || url.starts_with("wss://")) {
            return Err("Enter a server address beginning with ws:// or wss://.".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if url.starts_with("wss://") {
            return Err(
                "This native build uses ws:// servers. Secure browser connections use wss://."
                    .into(),
            );
        }
        // Bound incoming events even when rendering is paused or the peer floods its socket.
        let (tx, receiver) = mpsc::sync_channel(128);
        let sender = ewebsock::ws_connect(
            url.into(),
            ewebsock::Options {
                max_incoming_frame_size: MAX_SERVER_MESSAGE_BYTES,
                ..default()
            },
            Box::new(move |event| {
                if tx.try_send(event).is_ok() {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            }),
        )
        .map_err(|error| format!("Cannot open connection: {error}"))?;
        Ok(Self {
            sender,
            receiver,
            hello: Some(hello),
            opened: false,
            created: now,
        })
    }

    fn send(&mut self, message: &ClientMessage) -> Result<(), String> {
        let text = encode_client(message)
            .map_err(|error| format!("Cannot send to the server: {error}"))?;
        self.sender.send(WsMessage::Text(text));
        Ok(())
    }
}

#[derive(Default)]
struct Network(Option<Connection>);

impl Network {
    /// Sends on the open connection.
    fn send(&mut self, message: &ClientMessage) -> Result<(), String> {
        self.0
            .as_mut()
            .filter(|connection| connection.opened)
            .ok_or("Not connected to a server.")?
            .send(message)
    }

    fn leave(&mut self) {
        if let Some(mut connection) = self.0.take() {
            if connection.opened {
                let _ = connection.send(&ClientMessage::Leave);
            }
            connection.sender.close();
        }
    }
}

/// Applies [`SessionAction`]s. Interfaces that send actions run before it.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionActions;

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>()
            .init_resource::<SessionStatus>()
            .init_resource::<ReplayIo>()
            .insert_non_send(Network::default())
            .add_message::<SessionAction>()
            .add_systems(PreUpdate, poll_network)
            .add_systems(
                Update,
                (handle_actions, finish_replay_import)
                    .chain()
                    .in_set(SessionActions),
            )
            .add_systems(
                FixedUpdate,
                advance_session.run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnExit(AppState::InGame), cleanup_match);
    }
}

impl SessionAction {
    /// Actions that work while a match is on screen. Everything else needs the menu.
    const fn allowed_in_match(&self) -> bool {
        matches!(self, Self::Leave | Self::SaveReplay | Self::SaveGame)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one match over every session action keeps the transitions in one place"
)]
fn handle_actions(
    mut actions: MessageReader<SessionAction>,
    mut commands: Commands,
    catalog: Option<Res<MapCatalog>>,
    mut library: ResMut<Library>,
    mut session: ResMut<Session>,
    mut status: ResMut<SessionStatus>,
    mut network: NonSendMut<Network>,
    mut next: ResMut<NextState<AppState>>,
    io: Res<ReplayIo>,
    time: Res<Time>,
    state: Res<State<AppState>>,
) {
    for action in actions.read() {
        if (*state.get() != AppState::Menu || status.running) && !action.allowed_in_match() {
            status.fail("Leave the current match first.");
            continue;
        }
        let now = time.elapsed_secs_f64();
        let mut play = |game: Result<Match, String>,
                        kind: MatchKind,
                        message: &str,
                        status: &mut SessionStatus,
                        network: &mut Network|
         -> Result<(), String> {
            let game = game?;
            // Local matches do not keep a server connection.
            network.leave();
            status.disconnect(Server::Offline);
            commands.insert_resource(game);
            next.set(AppState::InGame);
            status.start_match(kind, message);
            Ok(())
        };
        let result = match action {
            SessionAction::NewGame { map } => catalog
                .as_ref()
                .and_then(|catalog| catalog.0.get(*map))
                .ok_or_else(|| "This map is not available.".to_owned())
                .and_then(|map| {
                    let game = session.begin(map.setup.clone(), Mode::Offline, PlayerId(0), now);
                    play(
                        game,
                        MatchKind::Solo,
                        "Select units, then give orders.",
                        &mut status,
                        &mut network,
                    )
                }),
            SessionAction::LoadGame { id } => {
                library.load(Shelf::Saves, id).and_then(|(saved, sim)| {
                    let game = Ok(session.resume(saved, sim, now));
                    play(
                        game,
                        MatchKind::Solo,
                        "Saved game loaded.",
                        &mut status,
                        &mut network,
                    )
                })
            }
            SessionAction::SaveGame => {
                if status.can_save_game() {
                    session
                        .recording
                        .as_ref()
                        .ok_or_else(|| "There is nothing to save.".to_owned())
                        .and_then(|replay| library.add(Shelf::Saves, replay))
                        .map(|record| status.info(format!("Saved as {}.", record.meta.name)))
                } else {
                    Err("Only a running solo match can be saved.".into())
                }
            }
            SessionAction::Connect { url } => {
                network.leave();
                Connection::connect(
                    url,
                    ClientMessage::Rooms {
                        version: PROTOCOL_VERSION,
                    },
                    now,
                )
                .map(|connection| {
                    network.0 = Some(connection);
                    status.disconnect(Server::Connecting {
                        url: url.trim().into(),
                    });
                    status.info("");
                })
                .inspect_err(|reason| {
                    status.disconnect(Server::Failed {
                        url: url.trim().into(),
                        reason: reason.clone(),
                    });
                })
            }
            SessionAction::Disconnect => {
                network.leave();
                session.online_setup = None;
                status.disconnect(Server::Offline);
                status.info("");
                Ok(())
            }
            SessionAction::RefreshRooms => network.send(&ClientMessage::Rooms {
                version: PROTOCOL_VERSION,
            }),
            SessionAction::CreateRoom { name, public } => {
                session.online_setup = None;
                network
                    .send(&ClientMessage::Create {
                        version: PROTOCOL_VERSION,
                        name: name.clone(),
                        public: *public,
                    })
                    .map(|()| status.joining = true)
            }
            SessionAction::JoinRoom { name, code } => {
                session.online_setup = None;
                network
                    .send(&ClientMessage::Join {
                        version: PROTOCOL_VERSION,
                        code: code.trim().to_ascii_uppercase(),
                        name: name.clone(),
                    })
                    .map(|()| status.joining = true)
            }
            SessionAction::SetReady(ready) => network.send(&ClientMessage::Ready { ready: *ready }),
            SessionAction::Start => {
                if status.lobby.as_ref().is_some_and(Lobby::can_start) {
                    network.send(&ClientMessage::Start)
                } else {
                    Ok(())
                }
            }
            SessionAction::LeaveRoom => {
                if status.lobby.take().is_some() {
                    session.online_setup = None;
                    network.send(&ClientMessage::Leave).and_then(|()| {
                        network.send(&ClientMessage::Rooms {
                            version: PROTOCOL_VERSION,
                        })
                    })
                } else {
                    Ok(())
                }
            }
            SessionAction::Leave => {
                if session.mode == Mode::Online {
                    // Ends the room on the server; the connection stays for the room list.
                    let _ = network.send(&ClientMessage::Leave);
                }
                if status.kind == MatchKind::Online && status.connected() {
                    let _ = network.send(&ClientMessage::Rooms {
                        version: PROTOCOL_VERSION,
                    });
                }
                session.archive();
                session.mode = Mode::Idle;
                next.set(AppState::Menu);
                status.leave_match();
                status.lobby = None;
                status.has_replay = session.replay().is_some();
                Ok(())
            }
            SessionAction::WatchReplay { id } => {
                library.load(Shelf::Replays, id).and_then(|(replay, _)| {
                    let game = session.begin_replay(replay, now);
                    play(
                        game,
                        MatchKind::Playback,
                        "Orders are disabled while watching.",
                        &mut status,
                        &mut network,
                    )
                })
            }
            SessionAction::WatchLast => session
                .replay()
                .cloned()
                .ok_or_else(|| "There is no recorded match yet.".to_owned())
                .and_then(|replay| {
                    let game = session.begin_replay(replay, now);
                    play(
                        game,
                        MatchKind::Playback,
                        "Orders are disabled while watching.",
                        &mut status,
                        &mut network,
                    )
                }),
            SessionAction::SaveReplay => session
                .replay()
                .ok_or_else(|| "There is no recorded match yet.".to_owned())
                .and_then(|replay| library.add(Shelf::Replays, replay))
                .map(|record| status.info(format!("Replay saved as {}.", record.meta.name))),
            SessionAction::Delete { shelf, id } => {
                library.delete(*shelf, id).map(|()| status.info("Deleted."))
            }
            SessionAction::ImportReplay => io.load().map(|message| {
                status.importing = true;
                status.info(message);
            }),
            SessionAction::ExportReplay { id } => library
                .load_text(Shelf::Replays, id)
                .and_then(|text| ReplayIo::save(&text))
                .map(|message| status.info(message)),
        };
        if let Err(error) = result {
            status.fail(error);
        }
    }
}

fn poll_network(
    mut commands: Commands,
    mut network: NonSendMut<Network>,
    mut session: ResMut<Session>,
    mut status: ResMut<SessionStatus>,
    mut game: Option<ResMut<Match>>,
    mut next: ResMut<NextState<AppState>>,
    time: Res<Time>,
) {
    let Some(connection) = network.0.as_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let mut failure = None;
    for _ in 0..128 {
        let event = match connection.receiver.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                failure = Some("Connection closed.".into());
                break;
            }
        };
        let result = match event {
            WsEvent::Opened => {
                connection.opened = true;
                connection
                    .hello
                    .take()
                    .map_or(Ok(()), |hello| connection.send(&hello))
            }
            WsEvent::Message(message) => decode_message(message).and_then(|message| {
                let Some(message) = message else {
                    return Ok(());
                };
                receive_message(
                    message,
                    &mut session,
                    &mut status,
                    game.as_deref_mut(),
                    &mut commands,
                    &mut next,
                    now,
                )
                .and_then(|reply| reply.map_or(Ok(()), |reply| connection.send(&reply)))
            }),
            WsEvent::Error(error) => Err(format!("Connection failed: {error}")),
            WsEvent::Closed => Err("Connection closed.".into()),
        };
        if let Err(error) = result {
            failure = Some(error);
            break;
        }
    }
    if !connection.opened && now - connection.created > 10.0 {
        failure = Some("Server connection timed out.".into());
    }
    if let Some(reason) = failure {
        network.leave();
        session.online_setup = None;
        if session.mode == Mode::Online {
            stop_match(&mut session, &mut status, game.as_deref_mut());
        }
        let url = match &status.server {
            Server::Connecting { url } | Server::Connected { url } | Server::Failed { url, .. } => {
                url.clone()
            }
            Server::Offline => String::new(),
        };
        status.disconnect(Server::Failed {
            url,
            reason: reason.clone(),
        });
        status.fail(reason);
    }
}

/// Freezes an online match that can no longer continue. The player can still save its replay.
fn stop_match(session: &mut Session, status: &mut SessionStatus, game: Option<&mut Match>) {
    session.mode = Mode::Stopped;
    status.stopped = true;
    if let Some(game) = game {
        game.accepts_orders = false;
        game.pending.clear();
    }
}

fn decode_message(message: WsMessage) -> Result<Option<ServerMessage>, String> {
    match message {
        WsMessage::Text(text) => decode_server(&text),
        WsMessage::Binary(bytes) => decode_server_binary(&bytes),
        WsMessage::Ping(_) | WsMessage::Pong(_) => return Ok(None),
        WsMessage::Unknown(_) => return Err("Server sent an unsupported message.".into()),
    }
    .map(Some)
    .map_err(|error| format!("Invalid server response: {error}"))
}

fn receive_message(
    message: ServerMessage,
    session: &mut Session,
    status: &mut SessionStatus,
    game: Option<&mut Match>,
    commands: &mut Commands,
    next: &mut NextState<AppState>,
    now: f64,
) -> Result<Option<ClientMessage>, String> {
    match message {
        ServerMessage::Setup(setup) => {
            if status.running || session.online_setup.is_some() {
                return Err("Unexpected match configuration.".into());
            }
            setup
                .simulation()
                .map_err(|error| format!("Invalid server configuration: {error}"))?;
            let checksum = setup.checksum();
            session.online_setup = Some(setup);
            return Ok(Some(ClientMessage::Loaded { checksum }));
        }
        ServerMessage::Rooms { rooms } => {
            if let Server::Connecting { url } = &status.server {
                status.server = Server::Connected { url: url.clone() };
            }
            status.rooms = rooms;
        }
        ServerMessage::Lobby {
            code,
            player,
            public,
            slots,
        } => {
            if status.running {
                return Err("Unexpected lobby update during a match.".into());
            }
            if status.lobby.is_none() && !status.joining {
                // An update for a room this client already left.
                return Ok(None);
            }
            status.joining = false;
            status.lobby = Some(Lobby {
                code,
                player: player.0,
                public,
                slots,
            });
        }
        ServerMessage::Started {
            players,
            input_delay,
        } => {
            let setup = session
                .online_setup
                .take()
                .ok_or("Server configuration is not loaded.")?;
            let lobby = status
                .lobby
                .take()
                .ok_or("Server started a match outside a room.")?;
            if status.running || players != setup.players || input_delay != INPUT_DELAY_TICKS {
                return Err("Server started an incompatible match.".into());
            }
            let game = session.begin(setup, Mode::Online, PlayerId(lobby.player), now)?;
            session.online_players = players;
            commands.insert_resource(game);
            next.set(AppState::InGame);
            status.start_match(
                MatchKind::Online,
                &format!("You are player {}.", lobby.player + 1),
            );
        }
        ServerMessage::Turn { turn, hash } => {
            if session.mode != Mode::Online || session.turns.len() >= 128 {
                return Err("Unexpected or excessive turn data.".into());
            }
            turn.validate(session.received_tick + 1, session.online_players)
                .map_err(|error| format!("Invalid turn: {error}"))?;
            session.received_tick = turn.tick;
            session.turns.insert(turn.tick, (turn, hash));
        }
        ServerMessage::Ended { reason } => {
            if session.mode == Mode::Online {
                stop_match(session, status, game);
                status.fail(format!("The match ended: {reason}."));
            } else if status.lobby.take().is_some() {
                session.online_setup = None;
                status.fail(format!("The room closed: {reason}."));
                return Ok(Some(ClientMessage::Rooms {
                    version: PROTOCOL_VERSION,
                }));
            }
        }
        ServerMessage::Error { message } => {
            if status.connecting() {
                // The server refused this client before listing rooms, e.g. a version mismatch.
                return Err(format!("The server refused the connection: {message}."));
            }
            // Request rejection (for example, late input) does not end the server match.
            status.joining = false;
            status.fail(capitalize(&message));
        }
    }
    Ok(None)
}

fn capitalize(message: &str) -> String {
    let mut chars = message.chars();
    chars.next().map_or_else(String::new, |first| {
        let mut text: String = first.to_uppercase().chain(chars).collect();
        if !text.ends_with('.') {
            text.push('.');
        }
        text
    })
}

fn advance_session(
    mut game: ResMut<Match>,
    mut session: ResMut<Session>,
    mut status: ResMut<SessionStatus>,
    mut network: NonSendMut<Network>,
    time: Res<Time>,
) {
    if let Err(reason) = advance_buffered(
        &mut game,
        &mut session,
        &mut status,
        &mut network,
        time.elapsed_secs_f64(),
    ) {
        if session.mode == Mode::Online {
            network.leave();
            status.disconnect(Server::Offline);
        }
        stop_match(&mut session, &mut status, Some(&mut game));
        status.fail(reason);
    }
}

/// A slow render frame may skip fixed ticks; consume the server backlog with bounded work.
fn advance_buffered(
    game: &mut Match,
    session: &mut Session,
    status: &mut SessionStatus,
    network: &mut Network,
    now: f64,
) -> Result<(), String> {
    for _ in 0..8 {
        advance(game, session, status, network, now)?;
        if session.mode != Mode::Online || session.turns.is_empty() {
            break;
        }
    }
    Ok(())
}

fn local_turn(
    game: &mut Match,
    frames: &mut BTreeMap<u32, Vec<Command>>,
    next_tick: u32,
) -> Result<Turn, String> {
    let target = game
        .sim
        .tick()
        .checked_add(INPUT_DELAY_TICKS)
        .ok_or("Match tick limit reached.")?;
    frames.insert(target, std::mem::take(&mut game.pending));
    let commands = frames.remove(&next_tick).unwrap_or_default();
    let players = u8::try_from(game.sim.players().len()).map_err(|_| "Invalid player count.")?;
    Ok(Turn {
        tick: next_tick,
        inputs: (0..players)
            .map(|player| PlayerInput {
                player: PlayerId(player),
                commands: if PlayerId(player) == game.local_player {
                    commands.clone()
                } else {
                    Vec::new()
                },
            })
            .collect(),
    })
}

fn advance(
    game: &mut Match,
    session: &mut Session,
    status: &mut SessionStatus,
    network: &mut Network,
    now: f64,
) -> Result<(), String> {
    let next_tick = game
        .sim
        .tick()
        .checked_add(1)
        .ok_or("Match tick limit reached.")?;
    let mut authoritative_hash = None;
    let turn =
        match session.mode {
            Mode::Offline => local_turn(game, &mut session.local_frames, next_tick)?,
            Mode::Online => {
                let Some((turn, hash)) = session.turns.remove(&next_tick) else {
                    if now - session.last_step > 15.0 {
                        return Err("The server stopped responding. Play has stopped.".into());
                    }
                    if now - session.last_step > 0.25 {
                        status.info("Waiting for the server…");
                    }
                    return Ok(());
                };
                authoritative_hash = Some(hash);
                let target = session
                    .received_tick
                    .checked_add(INPUT_DELAY_TICKS)
                    .ok_or("Match tick limit reached.")?;
                if target > session.submitted_tick {
                    network.0.as_mut().ok_or("Connection closed.")?.send(
                        &ClientMessage::Input {
                            tick: target,
                            commands: std::mem::take(&mut game.pending),
                        },
                    )?;
                    session.submitted_tick = target;
                }
                turn
            }
            Mode::Playback => {
                let replay = session.playback.as_ref().ok_or("Replay is not loaded.")?;
                let Some(turn) = replay.turns.get(game.sim.tick() as usize) else {
                    if game.sim.state_hash() != replay.final_hash {
                        return Err("Replay ended with a different match state.".into());
                    }
                    session.mode = Mode::Stopped;
                    status.stopped = true;
                    status.info("Replay finished and verified.");
                    return Ok(());
                };
                turn.clone()
            }
            Mode::Idle | Mode::Stopped => return Ok(()),
        };
    turn.apply(&mut game.sim)
        .map_err(|error| format!("Cannot advance match: {error}"))?;
    let hash = game.sim.state_hash();
    if session.mode == Mode::Online {
        if authoritative_hash != Some(hash) {
            return Err(
                "Match state differs from the authoritative server. Play has stopped.".into(),
            );
        }
        if status.message.starts_with("Waiting for") {
            status.info(format!("You are player {}.", game.local_player.0 + 1));
        }
    }
    if let Some(recording) = &mut session.recording {
        recording
            .push(turn, hash)
            .map_err(|error| format!("Cannot record match: {error}"))?;
    }
    session.last_step = now;
    Ok(())
}

fn finish_replay_import(
    mut io: ResMut<ReplayIo>,
    mut library: ResMut<Library>,
    mut status: ResMut<SessionStatus>,
) {
    let Some(result) = io.take_loaded() else {
        return;
    };
    status.importing = false;
    let result = result
        .and_then(|text| {
            Replay::from_ron(&text).map_err(|error| format!("Cannot import replay: {error}"))
        })
        .and_then(|replay| library.add(Shelf::Replays, &replay));
    match result {
        Ok(record) => status.info(format!("Imported as {}.", record.meta.name)),
        Err(error) => status.fail(error),
    }
}

fn cleanup_match(mut commands: Commands, mut selection: ResMut<Selection>) {
    selection.units.clear();
    commands.remove_resource::<Match>();
}
