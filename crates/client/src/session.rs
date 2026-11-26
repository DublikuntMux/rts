//! Match sessions: local delayed input, server-authoritative turns, and replay playback.
//!
//! The session owns the server connection. Online play connects first, lists rooms, and then
//! creates or joins a room on the same connection. [`SessionStatus`] is the read-only view the
//! interface draws from; the interface changes it only by sending [`SessionAction`]s.

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

use bevy::prelude::*;
use rts_ai::Difficulty;
use rts_assets::LocaleData;
use rts_base::{Command, MatchOutcome, PlayerId, PlayerView, Sim, ViewFrame, ViewIntro};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, LobbySetup, LobbySlot, MAX_SERVER_MESSAGE_BYTES, MatchSetup,
    PROTOCOL_VERSION, PlayerInput, Recording, Replay, RoomInfo, ServerMessage, Turn, ViewReplay,
    assets::{
        AssetClientMessage, AssetEntry, AssetKind, AssetServerMessage, MAX_ASSET_FILE_BYTES,
        MAX_ASSET_FRAME_BYTES, decode_asset_server, encode_asset_client, sha256,
        validate_asset_path,
    },
    decode_server, encode_client,
    quic::{
        PRIORITY_ASSETS, PRIORITY_GAME, STREAM_ASSETS, STREAM_GAME, client_endpoint,
        parse_server_url, read_frame, write_frame,
    },
};

use crate::{
    game::{AppState, MapCatalog, Match, Seat},
    input::Selection,
    library::{Library, Shelf},
    locale::Localization,
    menu::Menu,
    pause_menu::{PauseMenu, world_input_blocked},
    server_assets::{ServerAssets, cache_dir, read_cached, write_cached},
};

#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub enum SessionAction {
    /// Starts a solo match on a catalog map against an opponent of this difficulty.
    NewGame {
        map: usize,
        difficulty: Difficulty,
    },
    /// Starts the solo match over with its original setup and difficulty.
    Restart,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchResult {
    Victory,
    Defeat,
    Draw,
}

impl MatchResult {
    const fn for_player(outcome: MatchOutcome, player: PlayerId) -> Self {
        match outcome {
            MatchOutcome::Winner(winner) if winner.0 == player.0 => Self::Victory,
            MatchOutcome::Winner(_) => Self::Defeat,
            MatchOutcome::Draw => Self::Draw,
        }
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            Self::Victory => "result.victory",
            Self::Defeat => "result.defeat",
            Self::Draw => "result.draw",
        }
    }

    pub const fn detail_key(self) -> &'static str {
        match self {
            Self::Victory => "result.victory_detail",
            Self::Defeat => "result.defeat_detail",
            Self::Draw => "result.draw_detail",
        }
    }
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
    /// Server-provided asset download progress `(done, total)` while connecting.
    pub asset_progress: Option<(usize, usize)>,
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
    pub result: Option<MatchResult>,
    /// A recorded match exists: the current one or the last one played.
    pub has_replay: bool,
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

    pub const fn can_restart(&self) -> bool {
        self.running && matches!(self.kind, MatchKind::Solo)
    }

    /// Clears the match view after leaving it.
    const fn leave_match(&mut self) {
        self.running = false;
        self.stopped = false;
        self.kind = MatchKind::Solo;
        self.result = None;
    }

    fn start_match(&mut self, kind: MatchKind, message: &str) {
        self.running = true;
        self.stopped = false;
        self.kind = kind;
        self.result = None;
        self.has_replay = true;
        self.info(message);
    }

    fn disconnect(&mut self, server: Server) {
        self.server = server;
        self.asset_progress = None;
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
    /// Local orders in view IDs by the tick they execute.
    local_frames: BTreeMap<u32, Vec<Command>>,
    /// Server frames received but not yet applied, by tick.
    frames: BTreeMap<u32, ViewFrame>,
    /// Tick of the newest server frame received.
    received_tick: Option<u32>,
    /// Frames applied from a server or a player-perspective replay.
    applied_frames: usize,
    online_setup: Option<LobbySetup>,
    submitted_tick: u32,
    recording: Option<Recording>,
    last_replay: Option<Recording>,
    last_step: f64,
    /// The solo opponent's difficulty, kept by restarts and saved games.
    pub difficulty: Difficulty,
}

impl Session {
    fn replay(&self) -> Option<&Recording> {
        self.recording.as_ref().or(self.last_replay.as_ref())
    }

    /// The full replay of the current or last local match.
    fn local_replay(&self) -> Option<&Replay> {
        match self.replay()? {
            Recording::Match(replay) => Some(replay),
            Recording::View(_) => None,
        }
    }

    fn archive(&mut self) {
        if let Some(replay) = self.recording.take() {
            self.last_replay = Some(replay);
        }
        self.local_frames.clear();
        self.frames.clear();
        self.online_setup = None;
        self.submitted_tick = 0;
        self.received_tick = None;
        self.applied_frames = 0;
    }

    /// Starts a solo match on `setup`, played by the local player against computer opponents.
    fn begin(&mut self, setup: MatchSetup, now: f64) -> Result<Match, String> {
        let recording =
            Replay::new(setup).map_err(|error| format!("Cannot start this map: {error}"))?;
        self.begin_recording(recording, now)
    }

    fn begin_recording(&mut self, recording: Replay, now: f64) -> Result<Match, String> {
        let sim = recording
            .simulation()
            .map_err(|error| format!("Cannot start this map: {error}"))?;
        let game = self.local_match(sim);
        self.archive();
        self.recording = Some(Recording::Match(recording));
        self.mode = Mode::Offline;
        self.last_step = now;
        Ok(game)
    }

    fn local_match(&self, sim: Sim) -> Match {
        let players = u8::try_from(sim.players().len()).expect("bounded player count");
        let opponents = if sim.is_skirmish() {
            (1..players)
                .map(|player| Seat::new(&sim, PlayerId(player), self.difficulty))
                .collect()
        } else {
            Vec::new()
        };
        let mut game = Match::new(sim, PlayerId(0), true);
        game.local_mut().expect("local match").opponents = opponents;
        game
    }

    /// Continues a saved solo match. Replays its turns so every player's knowledge and the
    /// opponents' memory match the live match.
    fn resume(&mut self, saved: Replay, now: f64) -> Result<Match, String> {
        let invalid = |error: rts_net::NetError| format!("Cannot load: {error}");
        let mut game = self.local_match(saved.simulation().map_err(invalid)?);
        for turn in &saved.turns {
            let local = game.local_mut().expect("local match");
            for seat in &mut local.opponents {
                seat.commands(&local.sim);
            }
            turn.apply(&mut local.sim).map_err(invalid)?;
            game.observe_local();
        }
        if game.local_sim().map(Sim::state_hash) != Some(saved.final_hash) {
            return Err("Cannot load: the saved match does not verify.".into());
        }
        game.volleys.clear();
        self.archive();
        self.recording = Some(Recording::Match(saved));
        self.mode = Mode::Offline;
        self.last_step = now;
        Ok(game)
    }

    /// Joins a started online match from the local player's perspective.
    fn begin_online(
        &mut self,
        setup: &LobbySetup,
        view: ViewIntro,
        now: f64,
    ) -> Result<Match, String> {
        let invalid =
            |error: rts_net::NetError| format!("Server started an invalid match: {error}");
        let rules = setup.rules().map_err(invalid)?;
        let recording =
            ViewReplay::new(setup.map.clone(), setup.rules.clone(), view).map_err(invalid)?;
        let view = PlayerView::new(rules, view)
            .map_err(|error| format!("Server started an invalid match: {error}"))?;
        self.archive();
        self.recording = Some(Recording::View(recording));
        self.mode = Mode::Online;
        self.last_step = now;
        Ok(Match::online(view, true))
    }

    /// Watches a recording that is known to be valid: recorded here or verified by the library.
    fn begin_replay(&mut self, replay: Recording, now: f64) -> Result<Match, String> {
        let game = match &replay {
            Recording::Match(replay) => Match::full_replay(
                replay
                    .simulation()
                    .map_err(|error| format!("Cannot load replay map: {error}"))?,
                PlayerId(0),
            ),
            Recording::View(replay) => Match::online(
                replay
                    .view()
                    .map_err(|error| format!("Cannot load replay: {error}"))?,
                false,
            ),
        };
        self.archive();
        self.last_replay = Some(replay);
        self.mode = Mode::Playback;
        self.last_step = now;
        Ok(game)
    }
}

/// Maximum RTT-adaptive input lead in ticks (matches the server's future-input window).
const MAX_ADAPTIVE_LEAD_TICKS: u32 = 16;

enum QuicEvent {
    AssetProgress { done: usize, total: usize },
    SyncedAssets(Vec<(String, Vec<u8>)>),
    Opened,
    Frame(Vec<u8>),
    Error(String),
    Closed,
}

enum OutboundCommand {
    Send(Vec<u8>),
    Close,
}

struct Connection {
    sender: tokio::sync::mpsc::Sender<OutboundCommand>,
    receiver: Receiver<QuicEvent>,
    rtt_ms: Arc<AtomicU32>,
    hello: Option<ClientMessage>,
    opened: bool,
    created: f64,
}

impl Connection {
    fn connect(url: &str, hello: ClientMessage, now: f64) -> Result<Self, String> {
        let parsed = parse_server_url(url)?;
        let (event_tx, receiver) = mpsc::sync_channel(128);
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<OutboundCommand>(64);
        let rtt_ms = Arc::new(AtomicU32::new(0));
        let rtt_handle = Arc::clone(&rtt_ms);

        std::thread::Builder::new()
            .name("rts-quic-client".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    let _ = event_tx
                        .try_send(QuicEvent::Error("Cannot start the network runtime.".into()));
                    return;
                };
                runtime.block_on(run_quic_client(parsed, event_tx, out_rx, rtt_handle));
            })
            .map_err(|error| format!("Cannot start network thread: {error}"))?;

        Ok(Self {
            sender: out_tx,
            receiver,
            rtt_ms,
            hello: Some(hello),
            opened: false,
            created: now,
        })
    }

    fn send(&self, message: &ClientMessage) -> Result<(), String> {
        let bytes = encode_client(message)
            .map_err(|error| format!("Cannot send to the server: {error}"))?;
        self.sender
            .try_send(OutboundCommand::Send(bytes))
            .map_err(|_| "Network send queue is full.".to_owned())
    }

    /// Computes an RTT-adaptive input lead in ticks so commands still arrive on time over
    /// high-latency connections.
    fn input_lead_ticks(&self) -> u32 {
        let rtt_ms = self.rtt_ms.load(Ordering::Relaxed);
        let one_way_ms = rtt_ms / 2;
        let tick_ms = 1000 / rts_base::TICK_RATE;
        let adaptive = one_way_ms.div_ceil(tick_ms).saturating_add(1);
        adaptive.clamp(INPUT_DELAY_TICKS, MAX_ADAPTIVE_LEAD_TICKS)
    }

    fn close(&self) {
        let _ = self.sender.try_send(OutboundCommand::Close);
    }
}

async fn run_quic_client(
    parsed: rts_net::quic::ServerUrl,
    event_tx: mpsc::SyncSender<QuicEvent>,
    mut out_rx: tokio::sync::mpsc::Receiver<OutboundCommand>,
    rtt_handle: Arc<AtomicU32>,
) {
    let conn = match open_quic_connection(&parsed).await {
        Ok(conn) => conn,
        Err(error) => {
            let _ = event_tx.try_send(QuicEvent::Error(error));
            return;
        }
    };
    let (mut send, mut recv) = match conn.open_bi().await {
        Ok(streams) => streams,
        Err(error) => {
            let _ = event_tx.try_send(QuicEvent::Error(format!(
                "Cannot open game stream: {error}"
            )));
            return;
        }
    };
    let _ = send.set_priority(PRIORITY_GAME);
    if let Err(error) = send.write_all(&[STREAM_GAME]).await {
        let _ = event_tx.try_send(QuicEvent::Error(format!(
            "Cannot initialize game stream: {error}"
        )));
        return;
    }
    if let Err(error) = sync_server_assets(&conn, &event_tx).await {
        conn.close(1_u32.into(), b"asset sync failed");
        let _ = event_tx.try_send(QuicEvent::Error(error));
        return;
    }
    if event_tx.try_send(QuicEvent::Opened).is_err() {
        conn.close(0_u32.into(), b"client dropped");
        return;
    }
    let mut rtt_tick = tokio::time::interval(Duration::from_millis(250));
    rtt_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = rtt_tick.tick() => {
                let ms = u32::try_from(conn.rtt().as_millis()).unwrap_or(u32::MAX);
                rtt_handle.store(ms, Ordering::Relaxed);
            }
            incoming = read_frame(&mut recv, MAX_SERVER_MESSAGE_BYTES) => {
                match incoming {
                    Ok(Some(bytes)) => {
                        let ms = u32::try_from(conn.rtt().as_millis()).unwrap_or(u32::MAX);
                        rtt_handle.store(ms, Ordering::Relaxed);
                        if event_tx.try_send(QuicEvent::Frame(bytes)).is_err() {
                            conn.close(0_u32.into(), b"receive queue full");
                            return;
                        }
                    }
                    Ok(None) => {
                        let _ = event_tx.try_send(QuicEvent::Closed);
                        return;
                    }
                    Err(error) => {
                        let _ = event_tx.try_send(QuicEvent::Error(error.to_string()));
                        return;
                    }
                }
            }
            outbound = out_rx.recv() => {
                match outbound {
                    Some(OutboundCommand::Send(bytes)) => {
                        if let Err(error) = write_frame(&mut send, &bytes).await {
                            let _ = event_tx.try_send(QuicEvent::Error(error.to_string()));
                            return;
                        }
                    }
                    Some(OutboundCommand::Close) | None => {
                        conn.close(0_u32.into(), b"leave");
                        return;
                    }
                }
            }
        }
    }
}

async fn sync_server_assets(
    conn: &rts_net::quic::quinn::Connection,
    event_tx: &mpsc::SyncSender<QuicEvent>,
) -> Result<(), String> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|error| format!("Cannot open asset sync stream: {error}"))?;
    let _ = send.set_priority(PRIORITY_ASSETS);
    send.write_all(&[STREAM_ASSETS])
        .await
        .map_err(|error| format!("Cannot initialize asset stream: {error}"))?;
    let request = encode_asset_client(&AssetClientMessage::GetManifest {
        version: PROTOCOL_VERSION,
    })
    .map_err(|error| format!("Cannot encode asset manifest request: {error}"))?;
    write_frame(&mut send, &request)
        .await
        .map_err(|error| format!("Cannot request server asset manifest: {error}"))?;
    let response = read_frame(&mut recv, MAX_ASSET_FRAME_BYTES)
        .await
        .map_err(|error| format!("Cannot read server asset manifest: {error}"))?
        .ok_or_else(|| "Server closed the asset sync stream.".to_owned())?;
    let entries = match decode_asset_server(&response)
        .map_err(|error| format!("Invalid server asset manifest: {error}"))?
    {
        AssetServerMessage::Manifest { entries } => entries,
        AssetServerMessage::Error { message } => {
            return Err(format!("Server asset sync failed: {message}"));
        }
        _ => return Err("Unexpected response while reading asset manifest.".into()),
    };
    if entries.is_empty() {
        let _ = send.finish();
        return Ok(());
    }
    let cache = cache_dir();
    let total = entries.len();
    let mut synced = Vec::with_capacity(total);
    let mut missing: BTreeMap<String, AssetEntry> = BTreeMap::new();
    for entry in entries {
        if let Some(bytes) = read_cached(cache.as_deref(), &entry.sha256, MAX_ASSET_FILE_BYTES) {
            synced.push((entry.path, bytes));
        } else {
            missing.insert(entry.path.clone(), entry);
        }
    }
    let _ = event_tx.try_send(QuicEvent::AssetProgress {
        done: synced.len(),
        total,
    });
    if !missing.is_empty() {
        fetch_missing_assets(
            &mut send,
            &mut recv,
            &mut missing,
            cache.as_deref(),
            &mut synced,
            total,
            event_tx,
        )
        .await?;
    }
    let _ = send.finish();
    event_tx
        .send(QuicEvent::SyncedAssets(synced))
        .map_err(|_| "Client stopped before asset sync finished.".to_owned())
}

async fn fetch_missing_assets(
    send: &mut rts_net::quic::quinn::SendStream,
    recv: &mut rts_net::quic::quinn::RecvStream,
    missing: &mut BTreeMap<String, AssetEntry>,
    cache: Option<&std::path::Path>,
    synced: &mut Vec<(String, Vec<u8>)>,
    total: usize,
    event_tx: &mpsc::SyncSender<QuicEvent>,
) -> Result<(), String> {
    let fetch = encode_asset_client(&AssetClientMessage::Fetch {
        paths: missing.keys().cloned().collect(),
    })
    .map_err(|error| format!("Cannot encode asset fetch request: {error}"))?;
    write_frame(send, &fetch)
        .await
        .map_err(|error| format!("Cannot request server assets: {error}"))?;
    loop {
        let frame = read_frame(recv, MAX_ASSET_FRAME_BYTES)
            .await
            .map_err(|error| format!("Cannot read server asset: {error}"))?
            .ok_or_else(|| "Server closed the asset stream mid-transfer.".to_owned())?;
        match decode_asset_server(&frame)
            .map_err(|error| format!("Invalid server asset payload: {error}"))?
        {
            AssetServerMessage::File {
                path,
                sha256: digest,
                bytes,
            } => {
                let expected = missing
                    .remove(&path)
                    .ok_or_else(|| format!("Server sent unexpected asset `{path}`."))?;
                if digest != expected.sha256
                    || bytes.len() != expected.size as usize
                    || sha256(&bytes) != expected.sha256
                {
                    return Err(format!("Asset integrity check failed for `{path}`."));
                }
                write_cached(cache, &digest, &bytes);
                synced.push((path, bytes));
                let _ = event_tx.try_send(QuicEvent::AssetProgress {
                    done: synced.len(),
                    total,
                });
            }
            AssetServerMessage::Done => break,
            AssetServerMessage::Error { message } => {
                return Err(format!("Server asset sync failed: {message}"));
            }
            AssetServerMessage::Manifest { .. } => {
                return Err("Unexpected duplicate asset manifest.".into());
            }
        }
    }
    if !missing.is_empty() {
        return Err("Server omitted requested client assets.".into());
    }
    Ok(())
}

async fn open_quic_connection(
    parsed: &rts_net::quic::ServerUrl,
) -> Result<rts_net::quic::quinn::Connection, String> {
    let host_port = parsed.socket_host_port();
    let addr = tokio::net::lookup_host(&host_port)
        .await
        .map_err(|error| format!("Cannot resolve server: {error}"))?
        .next()
        .ok_or_else(|| "Could not resolve the server address.".to_owned())?;
    let endpoint = client_endpoint(
        addr.is_ipv6(),
        parsed.pinned_sha256,
        Duration::from_secs(30),
    )
    .map_err(|error| format!("Cannot open QUIC socket: {error}"))?;
    let connecting = endpoint
        .connect(addr, parsed.sni_name())
        .map_err(|error| format!("Cannot connect to server: {error}"))?;
    tokio::time::timeout(Duration::from_secs(10), connecting)
        .await
        .map_err(|_| "Server connection timed out.".to_owned())?
        .map_err(|error| format!("QUIC handshake failed: {error}"))
}

#[derive(Default)]
struct Network(Option<Connection>);

impl Network {
    /// Sends on the open connection.
    fn send(&self, message: &ClientMessage) -> Result<(), String> {
        self.0
            .as_ref()
            .filter(|connection| connection.opened)
            .ok_or("Not connected to a server.")?
            .send(message)
    }

    fn input_lead_ticks(&self) -> u32 {
        self.0
            .as_ref()
            .map_or(INPUT_DELAY_TICKS, Connection::input_lead_ticks)
    }

    fn leave(&mut self) {
        if let Some(connection) = self.0.take() {
            if connection.opened {
                let _ = connection.send(&ClientMessage::Leave);
            }
            connection.close();
        }
    }
}

/// Applies [`SessionAction`]s. Interfaces that send actions run before it.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionActions;

pub struct SessionPlugin;

#[derive(Resource)]
struct RestartMatch(Match);

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>()
            .init_resource::<SessionStatus>()
            .init_resource::<PauseMenu>()
            .init_resource::<ServerAssets>()
            .init_resource::<Localization>()
            .insert_non_send(Network::default())
            .add_message::<SessionAction>()
            .add_systems(PreUpdate, poll_network)
            .add_systems(Update, handle_actions.in_set(SessionActions))
            .add_systems(
                FixedUpdate,
                advance_session.run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnEnter(AppState::Restarting), restart_match)
            .add_systems(OnExit(AppState::InGame), cleanup_match);
    }
}

impl SessionAction {
    /// Actions that work while a match is on screen. Everything else needs the menu.
    const fn allowed_in_match(&self) -> bool {
        matches!(
            self,
            Self::Leave | Self::Restart | Self::SaveReplay | Self::SaveGame
        )
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
    mut server_assets: ResMut<ServerAssets>,
    mut loc: ResMut<Localization>,
    mut network: NonSendMut<Network>,
    mut next: ResMut<NextState<AppState>>,
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
                        network: &mut Network,
                        server_assets: &mut ServerAssets,
                        loc: &mut Localization|
         -> Result<(), String> {
            let game = game?;
            // Local matches do not keep a server connection or server-synced assets.
            network.leave();
            server_assets.clear();
            loc.clear_server_locales();
            status.disconnect(Server::Offline);
            commands.insert_resource(game);
            next.set(AppState::InGame);
            status.start_match(kind, message);
            Ok(())
        };
        let result = match action {
            SessionAction::NewGame { map, difficulty } => catalog
                .as_ref()
                .and_then(|catalog| catalog.0.get(*map))
                .ok_or_else(|| "This map is not available.".to_owned())
                .and_then(|map| {
                    session.difficulty = *difficulty;
                    let game = session.begin(map.setup.clone(), now)?;
                    let message = solo_start_message(&game);
                    play(
                        Ok(game),
                        MatchKind::Solo,
                        message,
                        &mut status,
                        &mut network,
                        &mut server_assets,
                        &mut loc,
                    )
                }),
            SessionAction::Restart if status.can_restart() => session
                .local_replay()
                .ok_or_else(|| "The previous match setup is unavailable.".to_owned())
                .and_then(|replay| {
                    replay
                        .restarted()
                        .map_err(|error| format!("Cannot restart this match: {error}"))
                })
                .and_then(|recording| {
                    let game = session.begin_recording(recording, now)?;
                    let message = solo_start_message(&game);
                    commands.insert_resource(RestartMatch(game));
                    next.set(AppState::Restarting);
                    status.start_match(MatchKind::Solo, message);
                    Ok(())
                }),
            SessionAction::Restart => Err("Only a solo match can be restarted.".into()),
            SessionAction::LoadGame { id } => library.load(Shelf::Saves, id).and_then(|saved| {
                let Recording::Match(saved) = saved else {
                    return Err("This save is not a local match.".into());
                };
                session.difficulty = library
                    .get(Shelf::Saves, id)
                    .and_then(|record| record.meta.difficulty)
                    .unwrap_or_default();
                let game = session.resume(saved, now);
                play(
                    game,
                    MatchKind::Solo,
                    "Saved game loaded.",
                    &mut status,
                    &mut network,
                    &mut server_assets,
                    &mut loc,
                )
            }),
            SessionAction::SaveGame => {
                if status.can_save_game() {
                    session
                        .recording
                        .as_ref()
                        .ok_or_else(|| "There is nothing to save.".to_owned())
                        .and_then(|replay| {
                            library.add(Shelf::Saves, replay, Some(session.difficulty))
                        })
                        .map(|record| status.info(format!("Saved as {}.", record.meta.name)))
                } else {
                    Err("Only a running solo match can be saved.".into())
                }
            }
            SessionAction::Connect { url } => {
                network.leave();
                server_assets.clear();
                loc.clear_server_locales();
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
                server_assets.clear();
                loc.clear_server_locales();
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
                library.load(Shelf::Replays, id).and_then(|replay| {
                    let game = session.begin_replay(replay, now);
                    play(
                        game,
                        MatchKind::Playback,
                        "Orders are disabled while watching.",
                        &mut status,
                        &mut network,
                        &mut server_assets,
                        &mut loc,
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
                        &mut server_assets,
                        &mut loc,
                    )
                }),
            SessionAction::SaveReplay => session
                .replay()
                .ok_or_else(|| "There is no recorded match yet.".to_owned())
                .and_then(|replay| library.add(Shelf::Replays, replay, None))
                .map(|record| status.info(format!("Replay saved as {}.", record.meta.name))),
            SessionAction::Delete { shelf, id } => {
                library.delete(*shelf, id).map(|()| status.info("Deleted."))
            }
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
    mut server_assets: ResMut<ServerAssets>,
    mut loc: ResMut<Localization>,
    mut images: Option<ResMut<Assets<Image>>>,
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
            QuicEvent::AssetProgress { done, total } => {
                status.asset_progress = Some((done, total));
                Ok(())
            }
            QuicEvent::SyncedAssets(files) => {
                status.asset_progress = None;
                apply_synced_assets(files, &mut server_assets, &mut loc, images.as_deref_mut())
            }
            QuicEvent::Opened => {
                status.asset_progress = None;
                connection.opened = true;
                connection
                    .hello
                    .take()
                    .map_or(Ok(()), |hello| connection.send(&hello))
            }
            QuicEvent::Frame(bytes) => decode_message(&bytes).and_then(|message| {
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
            QuicEvent::Error(error) => Err(format!("Connection failed: {error}")),
            QuicEvent::Closed => Err("Connection closed.".into()),
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
        server_assets.clear();
        loc.clear_server_locales();
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

fn apply_synced_assets(
    files: Vec<(String, Vec<u8>)>,
    server_assets: &mut ServerAssets,
    loc: &mut Localization,
    mut images: Option<&mut Assets<Image>>,
) -> Result<(), String> {
    server_assets.clear();
    loc.clear_server_locales();
    for (path, bytes) in files {
        let kind = validate_asset_path(&path)
            .map_err(|error| format!("Invalid server asset path `{path}`: {error}"))?;
        match kind {
            AssetKind::Texture => {
                if let Some(images) = images.as_deref_mut() {
                    server_assets.install_texture(path, bytes, images)?;
                }
            }
            AssetKind::Locale => {
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| format!("Server locale `{path}` is not valid UTF-8."))?;
                let locale = LocaleData::parse_str(text)
                    .map_err(|error| format!("Invalid server locale `{path}`: {error}"))?;
                loc.apply_server_locale(locale);
            }
        }
    }
    Ok(())
}

/// Freezes a settled or interrupted match and leaves its replay available.
fn stop_match(session: &mut Session, status: &mut SessionStatus, game: Option<&mut Match>) {
    session.mode = Mode::Stopped;
    status.stopped = true;
    if let Some(game) = game {
        game.accepts_orders = false;
        game.pending.clear();
    }
}

fn decode_message(bytes: &[u8]) -> Result<ServerMessage, String> {
    decode_server(bytes).map_err(|error| format!("Invalid server response: {error}"))
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
                .rules()
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
            view,
        } => {
            let setup = session
                .online_setup
                .take()
                .ok_or("Server configuration is not loaded.")?;
            let lobby = status
                .lobby
                .take()
                .ok_or("Server started a match outside a room.")?;
            if status.running
                || players != setup.players
                || input_delay != INPUT_DELAY_TICKS
                || view.player != PlayerId(lobby.player)
            {
                return Err("Server started an incompatible match.".into());
            }
            let game = session.begin_online(&setup, view, now)?;
            commands.insert_resource(game);
            next.set(AppState::InGame);
            status.start_match(
                MatchKind::Online,
                &format!("You are player {}.", lobby.player + 1),
            );
        }
        ServerMessage::View { frame } => {
            if session.mode != Mode::Online || session.frames.len() >= 128 {
                return Err("Unexpected or excessive view data.".into());
            }
            let expected = session
                .received_tick
                .map_or(Some(0), |tick| tick.checked_add(1));
            if Some(frame.tick) != expected {
                return Err("The server sent a view out of order.".into());
            }
            session.received_tick = Some(frame.tick);
            session.frames.insert(frame.tick, frame);
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
    mut pause: ResMut<PauseMenu>,
    menu: Option<Res<Menu>>,
) {
    if session.mode != Mode::Online && world_input_blocked(Some(&pause), menu.as_deref()) {
        return;
    }
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
    if status.result.is_some() {
        pause.open = true;
    }
}

/// Consumes buffered server frames smoothly on jittery connections while keeping bounded catch-up.
fn advance_buffered(
    game: &mut Match,
    session: &mut Session,
    status: &mut SessionStatus,
    network: &mut Network,
    now: f64,
) -> Result<(), String> {
    let budget = if session.mode == Mode::Online {
        match session.frames.len() {
            len if len <= 1 || (len == 2 && session.applied_frames > 0) => 1,
            2..=6 => 2,
            _ => 8,
        }
    } else {
        1
    };
    for _ in 0..budget {
        advance(game, session, status, network, now)?;
        if session.mode != Mode::Online || session.frames.is_empty() {
            break;
        }
    }
    Ok(())
}

/// Builds the next local turn: the local player's delayed orders and each computer opponent's
/// orders, all bound to the authority on the tick they execute.
fn local_turn(
    game: &mut Match,
    frames: &mut BTreeMap<u32, Vec<Command>>,
    next_tick: u32,
) -> Result<Turn, String> {
    let target = game
        .view
        .tick()
        .checked_add(INPUT_DELAY_TICKS)
        .ok_or("Match tick limit reached.")?;
    frames.insert(target, std::mem::take(&mut game.pending));
    let ordered = frames.remove(&next_tick).unwrap_or_default();
    let local_player = game.local_player;
    let local = game
        .local_mut()
        .ok_or("The match has no local simulation.")?;
    let players = u8::try_from(local.sim.players().len()).map_err(|_| "Invalid player count.")?;
    let mut inputs: Vec<_> = (0..players)
        .map(|player| PlayerInput {
            player: PlayerId(player),
            commands: Vec::new(),
        })
        .collect();
    inputs[usize::from(local_player.0)].commands = ordered
        .iter()
        .filter_map(|command| local.knowledge.translate_command(&local.sim, command))
        .collect();
    for seat in &mut local.opponents {
        inputs[usize::from(seat.knowledge.player().0)].commands = seat.commands(&local.sim);
    }
    Ok(Turn {
        tick: next_tick,
        inputs,
    })
}

/// Applies a full turn to the local simulation and refreshes every view.
fn apply_local(game: &mut Match, turn: &Turn) -> Result<u64, String> {
    let local = game
        .local_mut()
        .ok_or("The match has no local simulation.")?;
    turn.apply(&mut local.sim)
        .map_err(|error| format!("Cannot advance match: {error}"))?;
    let hash = local.sim.state_hash();
    game.observe_local();
    Ok(hash)
}

#[expect(
    clippy::too_many_lines,
    reason = "advancing solo, online, and playback modes stays clearest in one dispatch"
)]
fn advance(
    game: &mut Match,
    session: &mut Session,
    status: &mut SessionStatus,
    network: &mut Network,
    now: f64,
) -> Result<(), String> {
    match session.mode {
        Mode::Offline => {
            let next_tick = game
                .view
                .tick()
                .checked_add(1)
                .ok_or("Match tick limit reached.")?;
            let turn = local_turn(game, &mut session.local_frames, next_tick)?;
            let hash = apply_local(game, &turn)?;
            if let Some(Recording::Match(recording)) = &mut session.recording {
                recording
                    .push(turn, hash)
                    .map_err(|error| format!("Cannot record match: {error}"))?;
            }
            if let Some(result) = game.view.outcome() {
                status.result = Some(MatchResult::for_player(result, game.local_player));
                status.info("");
                stop_match(session, status, Some(game));
            }
        }
        Mode::Online => {
            let next = if session.applied_frames == 0 {
                0
            } else {
                game.view
                    .tick()
                    .checked_add(1)
                    .ok_or("Match tick limit reached.")?
            };
            let Some(frame) = session.frames.remove(&next) else {
                if now - session.last_step > 15.0 {
                    return Err("The server stopped responding. Play has stopped.".into());
                }
                if now - session.last_step > 0.25 {
                    status.info("Waiting for the server…");
                }
                return Ok(());
            };
            let lead = network.input_lead_ticks();
            let target = session
                .received_tick
                .unwrap_or(0)
                .checked_add(lead)
                .ok_or("Match tick limit reached.")?;
            if !game.pending.is_empty() && target > session.submitted_tick {
                network
                    .0
                    .as_mut()
                    .ok_or("Connection closed.")?
                    .send(&ClientMessage::Input {
                        tick: target,
                        commands: std::mem::take(&mut game.pending),
                    })?;
                session.submitted_tick = target;
            }
            game.view
                .apply(frame.clone())
                .map_err(|error| format!("Invalid server view: {error}"))?;
            game.record_volleys();
            session.applied_frames += 1;
            if let Some(Recording::View(recording)) = &mut session.recording {
                recording
                    .push(frame)
                    .map_err(|error| format!("Cannot record match: {error}"))?;
            }
            if status.message.starts_with("Waiting for") {
                status.info(format!("You are player {}.", game.local_player.0 + 1));
            }
        }
        Mode::Playback => match session
            .last_replay
            .as_ref()
            .ok_or("Replay is not loaded.")?
        {
            Recording::Match(replay) => {
                let tick = game.view.tick();
                let Some(turn) = replay.turns.get(tick as usize).cloned() else {
                    if game.local_sim().map(Sim::state_hash) != Some(replay.final_hash) {
                        return Err("Replay ended with a different match state.".into());
                    }
                    finish_playback(session, status, game, "Replay finished and verified.");
                    return Ok(());
                };
                apply_local(game, &turn)?;
            }
            Recording::View(replay) => {
                let Some(frame) = replay.frames.get(session.applied_frames).cloned() else {
                    finish_playback(session, status, game, "Replay finished.");
                    return Ok(());
                };
                game.view
                    .apply(frame)
                    .map_err(|error| format!("Invalid replay: {error}"))?;
                game.record_volleys();
                session.applied_frames += 1;
            }
        },
        Mode::Idle | Mode::Stopped => return Ok(()),
    }
    session.last_step = now;
    Ok(())
}

fn finish_playback(session: &mut Session, status: &mut SessionStatus, game: &Match, message: &str) {
    session.mode = Mode::Stopped;
    status.stopped = true;
    status.result = game
        .view
        .outcome()
        .map(|result| MatchResult::for_player(result, game.local_player));
    status.info(message);
}

fn cleanup_match(mut commands: Commands, mut selection: ResMut<Selection>) {
    selection.units.clear();
    commands.remove_resource::<Match>();
}

const fn solo_start_message(game: &Match) -> &'static str {
    if game.view.is_skirmish() {
        "Harvest resources and destroy the enemy headquarters."
    } else {
        "Select units, then give orders."
    }
}

fn restart_match(world: &mut World) {
    let RestartMatch(game) = world
        .remove_resource::<RestartMatch>()
        .expect("queued restart");
    world.insert_resource(game);
    world
        .resource_mut::<NextState<AppState>>()
        .set(AppState::InGame);
}
