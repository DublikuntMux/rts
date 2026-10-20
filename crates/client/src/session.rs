//! Match sessions: local delayed input, server-authoritative turns, and replay playback.

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    ops::ControlFlow,
    sync::mpsc::{self, Receiver, TryRecvError},
};

use bevy::prelude::*;
use ewebsock::{WsEvent, WsMessage, WsSender};
use rts_base::{Command, PlayerId};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, MAX_SERVER_MESSAGE_BYTES, MatchSetup, PROTOCOL_VERSION,
    PlayerInput, Replay, ServerMessage, Turn, decode_server, decode_server_binary, encode_client,
};

use crate::{
    game::{AppState, Match, MatchData, PreviousPositions},
    input::Selection,
    replay_io::ReplayIo,
};

#[derive(Message, Clone)]
pub enum SessionAction {
    Solo,
    Host { url: String },
    Join { url: String, code: String },
    Start,
    Leave,
    ReplayLast,
    SaveReplay,
    LoadReplay,
}

#[derive(Resource)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "UI availability flags project the session state; Mode is the actual state machine"
)]
pub struct SessionStatus {
    pub message: String,
    pub room_code: Option<String>,
    pub player: u8,
    pub joined: u8,
    pub capacity: u8,
    pub can_start: bool,
    pub has_replay: bool,
    pub running: bool,
    pub playback: bool,
    pub busy: bool,
}

impl SessionStatus {
    fn playing(message: &str, playback: bool) -> Self {
        Self {
            message: message.into(),
            running: true,
            playback,
            has_replay: true,
            ..default()
        }
    }
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self {
            message: "Play solo, or create or join a room.".into(),
            room_code: None,
            player: 0,
            joined: 0,
            capacity: 0,
            can_start: false,
            has_replay: false,
            running: false,
            playback: false,
            busy: false,
        }
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
        self.archive();
        let sim = setup
            .simulation()
            .map_err(|error| format!("Cannot start this map: {error}"))?;
        self.recording =
            Some(Replay::new(setup).map_err(|error| format!("Cannot record match: {error}"))?);
        self.mode = mode;
        self.last_step = now;
        Ok(Match::new(sim, player, true))
    }

    fn begin_replay(&mut self, replay: Replay, now: f64) -> Result<Match, String> {
        // Validate imported files before replacing the current session.
        replay
            .playback()
            .map_err(|error| format!("Cannot play replay: {error}"))?;
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
        let text = encode_client(message).map_err(|error| format!("Cannot send input: {error}"))?;
        self.sender.send(WsMessage::Text(text));
        Ok(())
    }
}

#[derive(Default)]
struct Network(Option<Connection>);

impl Network {
    fn leave(&mut self) {
        if let Some(mut connection) = self.0.take() {
            if connection.opened {
                let _ = connection.send(&ClientMessage::Leave);
            }
            connection.sender.close();
        }
    }
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>()
            .init_resource::<SessionStatus>()
            .init_resource::<ReplayIo>()
            .insert_non_send(Network::default())
            .add_message::<SessionAction>()
            .add_systems(PreUpdate, poll_network)
            .add_systems(Update, (handle_actions, finish_replay_load).chain())
            .add_systems(
                FixedUpdate,
                advance_session.run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnExit(AppState::InGame), cleanup_match);
    }
}

fn handle_actions(
    mut actions: MessageReader<SessionAction>,
    mut commands: Commands,
    data: Option<Res<MatchData>>,
    mut session: ResMut<Session>,
    mut status: ResMut<SessionStatus>,
    mut network: NonSendMut<Network>,
    mut next: ResMut<NextState<AppState>>,
    io: Res<ReplayIo>,
    time: Res<Time>,
    state: Res<State<AppState>>,
) {
    for action in actions.read() {
        if (*state.get() != AppState::Menu || status.running)
            && !matches!(action, SessionAction::Leave | SessionAction::SaveReplay)
        {
            status.message = "Leave the current match before starting another.".into();
            continue;
        }
        let Some(data) = &data else {
            continue;
        };
        let now = time.elapsed_secs_f64();
        let result = match action {
            SessionAction::Solo => {
                network.leave();
                session
                    .begin(data.0.clone(), Mode::Offline, PlayerId(0), now)
                    .map(|game| {
                        commands.insert_resource(game);
                        next.set(AppState::InGame);
                        *status = SessionStatus::playing("Solo match", false);
                    })
            }
            SessionAction::Host { url } | SessionAction::Join { url, .. } => {
                let hello = if let SessionAction::Join { code, .. } = action {
                    ClientMessage::Join {
                        version: PROTOCOL_VERSION,
                        code: code.trim().to_ascii_uppercase(),
                    }
                } else {
                    ClientMessage::Create {
                        version: PROTOCOL_VERSION,
                    }
                };
                network.leave();
                session.online_setup = None;
                Connection::connect(url, hello, now).map(|connection| {
                    network.0 = Some(connection);
                    *status = SessionStatus {
                        message: "Connecting to server…".into(),
                        busy: true,
                        has_replay: session.replay().is_some(),
                        ..default()
                    };
                })
            }
            SessionAction::Start => {
                if status.can_start {
                    network
                        .0
                        .as_mut()
                        .ok_or_else(|| "Connection is closed.".to_owned())
                        .and_then(|connection| connection.send(&ClientMessage::Start))
                } else {
                    Ok(())
                }
            }
            SessionAction::Leave => {
                network.leave();
                session.archive();
                session.mode = Mode::Idle;
                next.set(AppState::Menu);
                *status = SessionStatus {
                    has_replay: session.replay().is_some(),
                    ..default()
                };
                Ok(())
            }
            SessionAction::ReplayLast => session.replay().cloned().map_or_else(
                || Err("There is no recorded match yet.".into()),
                |replay| {
                    network.leave();
                    session.begin_replay(replay, now).map(|game| {
                        commands.insert_resource(game);
                        next.set(AppState::InGame);
                        *status = SessionStatus::playing("Replay playback", true);
                    })
                },
            ),
            SessionAction::SaveReplay => session
                .replay()
                .ok_or_else(|| "There is no recorded match yet.".to_owned())
                .and_then(|replay| {
                    replay
                        .to_ron()
                        .map_err(|error| format!("Cannot save replay: {error}"))
                })
                .and_then(|text| ReplayIo::save(&text))
                .map(|message| status.message = message),
            SessionAction::LoadReplay => io.load().map(|message| status.message = message),
        };
        if let Err(error) = result {
            status.message = error;
            status.busy = false;
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
        session.mode = Mode::Stopped;
        if let Some(game) = game.as_mut() {
            game.accepts_orders = false;
            game.pending.clear();
        }
        status.message = reason;
        status.busy = false;
        if !status.running {
            status.room_code = None;
        }
        status.can_start = false;
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
            return Ok(Some(ClientMessage::Ready { checksum }));
        }
        ServerMessage::Lobby {
            code,
            player,
            players,
            capacity,
            ready,
        } => {
            if status.running {
                return Err("Unexpected lobby update during a match.".into());
            }
            status.message = format!("Room {code}: {players}/{capacity} players, {ready} ready");
            status.room_code = Some(code);
            status.player = player.0;
            status.joined = players;
            status.capacity = capacity;
            status.can_start = player.0 == 0 && players == capacity && ready == capacity;
            status.busy = false;
        }
        ServerMessage::Started {
            players,
            input_delay,
        } => {
            let setup = session
                .online_setup
                .take()
                .ok_or("Server configuration is not loaded.")?;
            if status.running
                || status.room_code.is_none()
                || players != setup.players
                || input_delay != INPUT_DELAY_TICKS
            {
                return Err("Server started an incompatible match.".into());
            }
            let game = session.begin(setup, Mode::Online, PlayerId(status.player), now)?;
            commands.insert_resource(game);
            next.set(AppState::InGame);
            status.message = format!("Online match (player {})", status.player + 1);
            status.running = true;
            status.can_start = false;
            status.has_replay = true;
        }
        ServerMessage::Turn { turn, hash } => {
            if session.mode != Mode::Online || session.turns.len() >= 128 {
                return Err("Unexpected or excessive turn data.".into());
            }
            turn.validate(session.received_tick + 1, status.capacity)
                .map_err(|error| format!("Invalid turn: {error}"))?;
            session.received_tick = turn.tick;
            session.turns.insert(turn.tick, (turn, hash));
        }
        ServerMessage::Ended { reason } => return Err(reason),
        ServerMessage::Error { message } => {
            // Request rejection (for example, late input) does not end the server match.
            status.message = message;
            status.busy = false;
        }
    }
    Ok(None)
}

fn advance_session(
    mut game: ResMut<Match>,
    mut previous: ResMut<PreviousPositions>,
    mut session: ResMut<Session>,
    mut status: ResMut<SessionStatus>,
    mut network: NonSendMut<Network>,
    time: Res<Time>,
) {
    if let Err(reason) = advance_buffered(
        &mut game,
        &mut previous,
        &mut session,
        &mut status,
        &mut network,
        time.elapsed_secs_f64(),
    ) {
        network.leave();
        session.mode = Mode::Stopped;
        game.accepts_orders = false;
        game.pending.clear();
        previous.0.clear();
        status.message = reason;
    }
}

/// A slow render frame may skip fixed ticks; consume the server backlog with bounded work.
fn advance_buffered(
    game: &mut Match,
    previous: &mut PreviousPositions,
    session: &mut Session,
    status: &mut SessionStatus,
    network: &mut Network,
    now: f64,
) -> Result<(), String> {
    for _ in 0..8 {
        advance(game, previous, session, status, network, now)?;
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
    previous: &mut PreviousPositions,
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
                    // A stalled fixed clock must not repeatedly interpolate the previous tick backwards.
                    previous.0.clear();
                    if now - session.last_step > 15.0 {
                        return Err("The server stopped responding. Play has stopped.".into());
                    }
                    if now - session.last_step > 0.25 {
                        status.message = "Waiting for the server…".into();
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
                    previous.0.clear();
                    status.message = "Replay finished and verified.".into();
                    return Ok(());
                };
                turn.clone()
            }
            Mode::Idle | Mode::Stopped => {
                previous.0.clear();
                return Ok(());
            }
        };
    previous.0.clear();
    previous
        .0
        .extend(game.sim.units().map(|(id, unit)| (id, unit.pos)));
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
            status.message = format!("Online match (player {})", game.local_player.0 + 1);
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

fn finish_replay_load(
    mut commands: Commands,
    mut io: ResMut<ReplayIo>,
    mut session: ResMut<Session>,
    mut status: ResMut<SessionStatus>,
    mut network: NonSendMut<Network>,
    mut next: ResMut<NextState<AppState>>,
    time: Res<Time>,
    state: Res<State<AppState>>,
) {
    if let Some(result) = io.take_loaded() {
        if *state.get() != AppState::Menu || status.running {
            status.message = "Leave the current match before loading a replay.".into();
            return;
        }
        let result = result
            .and_then(|text| {
                Replay::from_ron(&text).map_err(|error| format!("Cannot load replay: {error}"))
            })
            .and_then(|replay| session.begin_replay(replay, time.elapsed_secs_f64()));
        match result {
            Ok(game) => {
                network.leave();
                commands.insert_resource(game);
                next.set(AppState::InGame);
                *status = SessionStatus::playing("Replay playback", true);
            }
            Err(error) => status.message = error,
        }
    }
}

fn cleanup_match(
    mut commands: Commands,
    mut selection: ResMut<Selection>,
    mut previous: ResMut<PreviousPositions>,
) {
    selection.units.clear();
    previous.0.clear();
    commands.remove_resource::<Match>();
}
