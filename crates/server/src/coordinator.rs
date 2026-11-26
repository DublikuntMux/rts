use std::{collections::BTreeMap, time::Duration};

use rand::{RngExt, rngs::StdRng};
use rts_base::{Command, MapData, PlayerId, PlayerKnowledge, Sim, TICK_RATE};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, LobbySetup, LobbySlot, MAX_LISTED_ROOMS, MAX_MAP_NAME_CHARS,
    MatchSetup, PROTOCOL_VERSION, PlayerInput, RoomInfo, ServerMessage, Turn, encode_client,
    validate_commands, validate_name,
};

use crate::{
    assets::ServerAsset,
    layout::{RANDOM_MAP_NAME, random_setup},
};

/// Connection identity allocated by the socket runtime, never supplied by a client.
pub type ClientId = u64;

/// Maximum buffered input lead relative to the authoritative simulation tick.
pub const MAX_FUTURE_TICKS: u32 = 16;

/// Resource limits, socket timeouts and server-owned match content.
#[derive(Clone, Debug)]
pub struct Config {
    /// Rules and seed for every match, and the fixed map when `random_layout` is off.
    pub setup: MatchSetup,
    /// Generates a private layout for each match from a server-only seed. A fixed map is
    /// public knowledge to anyone who has the map file.
    pub random_layout: bool,
    /// Limits terrain, units and orders to each player's sight by default.
    pub fog_of_war: bool,
    /// Optional client assets (`textures/**/*.png`, `locales/*.lang.ron`) served on connect.
    pub assets: BTreeMap<String, ServerAsset>,
    pub max_connections: usize,
    pub max_rooms: usize,
    pub event_queue_capacity: usize,
    pub outbound_queue_capacity: usize,
    pub peer_timeout: Duration,
    pub lobby_timeout: Duration,
    pub handshake_timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            setup: MatchSetup {
                rules: include_str!("../../../assets/data/core.rules.ron").to_owned(),
                map: include_str!("../../../assets/maps/frontier.map.ron").to_owned(),
                seed: 0,
                players: 2,
            },
            random_layout: true,
            fog_of_war: true,
            assets: BTreeMap::new(),
            max_connections: 1024,
            max_rooms: 256,
            event_queue_capacity: 256,
            outbound_queue_capacity: 64,
            peer_timeout: Duration::from_secs(15),
            lobby_timeout: Duration::from_mins(30),
            handshake_timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(10),
        }
    }
}

/// A message addressed to a particular authenticated connection.
#[derive(Debug)]
pub struct Delivery {
    pub client: ClientId,
    pub message: ServerMessage,
}

struct Peer {
    room: Option<String>,
    last_seen: Duration,
}

#[derive(Clone)]
struct Slot {
    client: ClientId,
    name: String,
    loaded: bool,
    ready: bool,
}

impl Slot {
    const fn new(client: ClientId, name: String) -> Self {
        Self {
            client,
            name,
            loaded: false,
            ready: false,
        }
    }
}

struct Room {
    slots: Vec<Option<Slot>>,
    public: bool,
    game: Option<Game>,
}

const TICK_DURATION: Duration = Duration::from_millis(1000 / TICK_RATE as u64);
/// A long scheduler stall advances at most this many turns before resetting the deadline.
pub const MAX_CATCH_UP_TURNS: usize = 4;

struct Game {
    sim: Sim,
    /// What each slot's player knows. Every client receives only its own frames.
    knowledge: Vec<PlayerKnowledge>,
    last_input: Vec<u32>,
    inputs: BTreeMap<u32, Vec<Vec<Command>>>,
    next_turn: Duration,
}

/// Characters of a room code: uppercase letters and digits.
const CODE_ALPHABET: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const CODE_LEN: usize = 6;

/// Pure room coordinator. Callers supply monotonically increasing elapsed time and the
/// random source for room codes, so the coordinator itself never reads clocks or the OS.
pub struct Coordinator {
    config: Config,
    /// The validated fixed match, or `None` when the setup cannot run.
    initial_sim: Option<Sim>,
    lobby_setup: LobbySetup,
    setup_checksum: u64,
    peers: BTreeMap<ClientId, Peer>,
    rooms: BTreeMap<String, Room>,
    /// Chooses room codes and layout seeds. Codes admit players to rooms and seeds decide
    /// hidden layouts, so seed it from OS entropy.
    rng: StdRng,
}

impl Coordinator {
    /// `rng` chooses room codes and random layouts. Anyone who knows a waiting room's code
    /// can join it, and a known seed reveals a layout, so production servers seed it from OS
    /// entropy; tests may use a fixed seed.
    #[must_use]
    pub fn new(config: Config, rng: StdRng) -> Self {
        let initial_sim = if config.random_layout {
            random_setup(&config.setup.rules, 0)
                .ok()
                .and_then(|setup| setup.simulation().ok())
        } else {
            config.setup.simulation().ok()
        };
        let map = if config.random_layout {
            RANDOM_MAP_NAME.to_owned()
        } else {
            MapData::from_ron(&config.setup.map).map_or_else(
                |_| String::new(),
                |map| {
                    map.name
                        .chars()
                        .take(MAX_MAP_NAME_CHARS)
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .collect()
                },
            )
        };
        let players = initial_sim
            .as_ref()
            .and_then(|sim| u8::try_from(sim.players().len()).ok())
            .unwrap_or(config.setup.players);
        let lobby_setup = LobbySetup {
            rules: config.setup.rules.clone(),
            map,
            players,
        };
        Self {
            config,
            initial_sim,
            setup_checksum: lobby_setup.checksum(),
            lobby_setup,
            peers: BTreeMap::new(),
            rooms: BTreeMap::new(),
            rng,
        }
    }

    /// Whether the configured rules and map can start matches.
    #[must_use]
    pub const fn can_host(&self) -> bool {
        self.initial_sim.is_some()
    }

    /// The public room setup every member receives.
    #[must_use]
    pub const fn lobby_setup(&self) -> &LobbySetup {
        &self.lobby_setup
    }

    /// Reserves a connection, including connections still doing their handshake.
    pub fn connect(&mut self, client: ClientId, now: Duration) -> bool {
        if self.initial_sim.is_none()
            || self.peers.len() >= self.config.max_connections
            || self.peers.contains_key(&client)
        {
            return false;
        }
        self.peers.insert(
            client,
            Peer {
                room: None,
                last_seen: now,
            },
        );
        true
    }

    #[must_use]
    pub fn is_connected(&self, client: ClientId) -> bool {
        self.peers.contains_key(&client)
    }

    /// Marks socket activity, including WebSocket control frames.
    pub fn touch(&mut self, client: ClientId, now: Duration) {
        if let Some(peer) = self.peers.get_mut(&client) {
            peer.last_seen = now;
        }
    }

    /// Handles one decoded message. Invalid requests do not mutate match progress,
    /// and return retryable errors without consuming a lobby membership.
    pub fn receive(
        &mut self,
        client: ClientId,
        message: ClientMessage,
        now: Duration,
    ) -> Vec<Delivery> {
        if !self.peers.contains_key(&client) {
            return Vec::new();
        }
        self.touch(client, now);
        let result = match message {
            ClientMessage::Rooms { version } => self.list_rooms(client, version),
            ClientMessage::Create {
                version,
                name,
                public,
            } => self.create(client, version, name, public),
            ClientMessage::Join {
                version,
                code,
                name,
            } => self.join(client, version, &code, name),
            message => self.in_room(client, message, now),
        };
        result.unwrap_or_else(|message| vec![error(client, message)])
    }

    /// Removes a connection; a running match ends for every remaining member.
    pub fn disconnect(&mut self, client: ClientId) -> Vec<Delivery> {
        let Some(peer) = self.peers.remove(&client) else {
            return Vec::new();
        };
        let Some(code) = peer.room else {
            return Vec::new();
        };
        let Some(mut room) = self.rooms.remove(&code) else {
            return Vec::new();
        };
        if room.game.is_some()
            || room.slots[0]
                .as_ref()
                .is_some_and(|slot| slot.client == client)
        {
            return self.end_room(room, "a player disconnected");
        }
        for slot in &mut room.slots {
            if slot.as_ref().is_some_and(|slot| slot.client == client) {
                *slot = None;
            }
        }
        let messages = lobby(&code, &room);
        self.rooms.insert(code, room);
        messages
    }

    /// Expires silent sockets. Match progress never depends on input or hash reports.
    /// Pregame sockets have a separate, generous lobby timeout.
    pub fn expire(&mut self, now: Duration) -> Vec<Delivery> {
        let expired: Vec<_> = self
            .peers
            .iter()
            .filter_map(|(&id, peer)| {
                let waiting = peer
                    .room
                    .as_ref()
                    .and_then(|code| self.rooms.get(code))
                    .is_some_and(|room| room.game.is_none());
                let timeout = if waiting {
                    self.config.lobby_timeout
                } else {
                    self.config.peer_timeout
                };
                (now.saturating_sub(peer.last_seen) >= timeout).then_some(id)
            })
            .collect();
        let mut messages = Vec::new();
        for id in expired {
            messages.extend(self.disconnect(id));
        }
        messages
    }

    /// Advances every match on its own 20 Hz clock, with bounded catch-up after stalls.
    /// The first turn is due 50 ms after Started; missing player input is empty.
    pub fn advance(&mut self, now: Duration) -> Vec<Delivery> {
        let mut messages = Vec::new();
        let mut failed = Vec::new();
        for (code, room) in &mut self.rooms {
            let Some(game) = &mut room.game else { continue };
            match game.advance(now) {
                Ok(views) => {
                    for (slot, message) in views {
                        messages.extend(to_slot(room, slot, message));
                    }
                }
                Err(reason) => failed.push((code.clone(), reason)),
            }
        }
        for (code, reason) in failed {
            if let Some(room) = self.rooms.remove(&code) {
                messages.extend(self.end_room(room, reason));
            }
        }
        messages
    }

    fn list_rooms(&self, client: ClientId, version: u32) -> Result<Vec<Delivery>, &'static str> {
        self.available(client, version)?;
        let rooms = self
            .rooms
            .iter()
            .filter(|(_, room)| {
                room.public && room.game.is_none() && room.slots.iter().any(Option::is_none)
            })
            .take(MAX_LISTED_ROOMS)
            .map(|(code, room)| RoomInfo {
                code: code.clone(),
                host: room.slots[0].as_ref().expect("host slot").name.clone(),
                map: self.lobby_setup.map.clone(),
                players: u8::try_from(room.slots.iter().flatten().count())
                    .expect("bounded player count"),
                capacity: u8::try_from(room.slots.len()).expect("bounded player count"),
            })
            .collect();
        Ok(vec![Delivery {
            client,
            message: ServerMessage::Rooms { rooms },
        }])
    }

    fn create(
        &mut self,
        client: ClientId,
        version: u32,
        name: String,
        public: bool,
    ) -> Result<Vec<Delivery>, &'static str> {
        self.available(client, version)?;
        validate_name(&name).map_err(|_| "invalid player display name")?;
        if self.rooms.len() >= self.config.max_rooms {
            return Err("server room limit reached");
        }
        let code = self.allocate_code();
        let mut slots = vec![None; usize::from(self.lobby_setup.players)];
        slots[0] = Some(Slot::new(client, name));
        let room = Room {
            public,
            slots,
            game: None,
        };
        self.peers
            .get_mut(&client)
            .expect("registered connection")
            .room = Some(code.clone());
        let mut messages = vec![Delivery {
            client,
            message: ServerMessage::Setup(self.lobby_setup.clone()),
        }];
        messages.extend(lobby(&code, &room));
        self.rooms.insert(code, room);
        Ok(messages)
    }

    fn join(
        &mut self,
        client: ClientId,
        version: u32,
        code: &str,
        name: String,
    ) -> Result<Vec<Delivery>, &'static str> {
        self.available(client, version)?;
        validate_name(&name).map_err(|_| "invalid player display name")?;
        if code.len() != CODE_LEN || !code.bytes().all(|b| CODE_ALPHABET.contains(&b)) {
            return Err("invalid room code");
        }
        let room = self.rooms.get_mut(code).ok_or("room does not exist")?;
        if room.game.is_some() {
            return Err("match has already started");
        }
        let slot = room
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or("room is full")?;
        *slot = Some(Slot::new(client, name));
        self.peers
            .get_mut(&client)
            .expect("registered connection")
            .room = Some(code.to_owned());
        let mut messages = vec![Delivery {
            client,
            message: ServerMessage::Setup(self.lobby_setup.clone()),
        }];
        messages.extend(lobby(code, room));
        Ok(messages)
    }

    fn available(&self, client: ClientId, version: u32) -> Result<(), &'static str> {
        if version != PROTOCOL_VERSION {
            return Err("protocol version does not match");
        }
        if self.peers[&client].room.is_some() {
            return Err("connection is already in a room");
        }
        Ok(())
    }

    /// Draws uniformly random codes until one is not used by a live room.
    fn allocate_code(&mut self) -> String {
        loop {
            let code: String = (0..CODE_LEN)
                .map(|_| char::from(CODE_ALPHABET[self.rng.random_range(0..CODE_ALPHABET.len())]))
                .collect();
            if !self.rooms.contains_key(&code) {
                return code;
            }
        }
    }

    fn in_room(
        &mut self,
        client: ClientId,
        message: ClientMessage,
        now: Duration,
    ) -> Result<Vec<Delivery>, &'static str> {
        let code = self.peers[&client]
            .room
            .clone()
            .ok_or("connection is not in a room")?;
        let mut room = self
            .rooms
            .remove(&code)
            .expect("membership references a room");
        let slot = room
            .slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|slot| slot.client == client))
            .expect("room member");
        if matches!(message, ClientMessage::Leave) {
            if slot == 0 || room.game.is_some() {
                return Ok(self.end_room(room, "a player left the room"));
            }
            room.slots[slot] = None;
            self.peers
                .get_mut(&client)
                .expect("registered connection")
                .room = None;
            let mut messages = vec![Delivery {
                client,
                message: ServerMessage::Ended {
                    reason: "left the room".into(),
                },
            }];
            messages.extend(lobby(&code, &room));
            self.rooms.insert(code, room);
            return Ok(messages);
        }
        let action = match message {
            ClientMessage::Loaded { checksum } => {
                if room.game.is_some() {
                    Err("match has already started")
                } else if checksum != self.setup_checksum {
                    Err("setup checksum does not match")
                } else {
                    room.slots[slot].as_mut().expect("room member").loaded = true;
                    Ok(lobby(&code, &room))
                }
            }
            ClientMessage::Ready { ready } => {
                let member = room.slots[slot].as_mut().expect("room member");
                if room.game.is_some() {
                    Err("match has already started")
                } else if ready && !member.loaded {
                    Err("load the match setup first")
                } else {
                    member.ready = ready;
                    Ok(lobby(&code, &room))
                }
            }
            ClientMessage::Start => self.start(&mut room, slot, now).map(|messages| {
                for member in room.slots.iter().flatten() {
                    self.touch(member.client, now);
                }
                messages
                    .into_iter()
                    .filter_map(|(slot, message)| to_slot(&room, slot, message))
                    .collect()
            }),
            message => room.receive(slot, message).map(|()| Vec::new()),
        };
        self.rooms.insert(code, room);
        action
    }

    fn end_room(&mut self, room: Room, reason: &str) -> Vec<Delivery> {
        for member in room.slots.iter().flatten() {
            if let Some(peer) = self.peers.get_mut(&member.client) {
                peer.room = None;
            }
        }
        broadcast(
            &room,
            ServerMessage::Ended {
                reason: reason.to_owned(),
            },
        )
    }
}

impl Coordinator {
    /// Starts a full, loaded, ready room. Each member learns only its own perspective.
    fn start(
        &mut self,
        room: &mut Room,
        slot: usize,
        now: Duration,
    ) -> Result<Vec<(usize, ServerMessage)>, &'static str> {
        if slot != 0 {
            return Err("only the host can start the match");
        }
        if room.game.is_some() {
            return Err("match has already started");
        }
        if room.slots.iter().any(Option::is_none) {
            return Err("room is not full");
        }
        if !room.slots.iter().flatten().all(|slot| slot.loaded) {
            return Err("all players must acknowledge the setup");
        }
        if !room.slots.iter().flatten().all(|slot| slot.ready) {
            return Err("all players must be ready");
        }
        let sim = if self.config.random_layout {
            let seed = self.rng.random();
            random_setup(&self.config.setup.rules, seed)
                .ok()
                .and_then(|setup| setup.simulation().ok())
                .ok_or("invalid server setup")?
        } else {
            self.initial_sim.clone().ok_or("invalid server setup")?
        };
        if sim.players().len() != room.slots.len() {
            return Err("invalid server setup");
        }
        let mut sim = sim;
        if self.config.fog_of_war {
            sim.enforce_fog();
        }
        let players = u8::try_from(room.slots.len()).expect("bounded player count");
        let mut knowledge: Vec<_> = (0..players)
            .map(|player| PlayerKnowledge::with_fog(&sim, PlayerId(player), self.config.fog_of_war))
            .collect();
        let mut messages = Vec::new();
        for (slot, knowledge) in knowledge.iter_mut().enumerate() {
            messages.push((
                slot,
                ServerMessage::Started {
                    players,
                    input_delay: INPUT_DELAY_TICKS,
                    view: knowledge.intro(&sim),
                },
            ));
            messages.push((
                slot,
                ServerMessage::View {
                    frame: knowledge.observe(&sim),
                },
            ));
        }
        room.game = Some(Game {
            sim,
            knowledge,
            last_input: vec![0; room.slots.len()],
            inputs: BTreeMap::new(),
            next_turn: now.saturating_add(TICK_DURATION),
        });
        Ok(messages)
    }
}

impl Room {
    fn receive(&mut self, slot: usize, message: ClientMessage) -> Result<(), &'static str> {
        let game = self.game.as_mut().ok_or("match has not started")?;
        match message {
            ClientMessage::Input { tick, commands } => game.input(slot, tick, &commands),
            _ => Err("invalid message for a running match"),
        }
    }
}

impl Game {
    /// Accepts a player's batch for a future tick. Commands use that player's view IDs; they
    /// are bound to the player now, so units it does not own, targets it cannot see, and
    /// deposits or sites it has not explored are dropped.
    fn input(&mut self, slot: usize, tick: u32, commands: &[Command]) -> Result<(), &'static str> {
        if tick <= self.sim.tick() || tick <= self.last_input[slot] {
            return Err("input tick is past, duplicate or out of order");
        }
        if tick > self.sim.tick().saturating_add(MAX_FUTURE_TICKS) {
            return Err("input tick exceeds the permitted future lead");
        }
        validate_commands(commands).map_err(|_| "input exceeds command limits")?;
        encode_client(&ClientMessage::Input {
            tick,
            commands: commands.to_vec(),
        })
        .map_err(|_| "input exceeds message limits")?;
        let knowledge = &self.knowledge[slot];
        let commands = commands
            .iter()
            .filter_map(|command| knowledge.translate_command(&self.sim, command))
            .collect();
        let inputs = self
            .inputs
            .entry(tick)
            .or_insert_with(|| vec![Vec::new(); self.last_input.len()]);
        inputs[slot] = commands;
        self.last_input[slot] = tick;
        Ok(())
    }

    /// Simulates due ticks and returns each player's frame for every tick, by slot.
    fn advance(&mut self, now: Duration) -> Result<Vec<(usize, ServerMessage)>, &'static str> {
        let mut messages = Vec::new();
        let mut turns = 0;
        while now >= self.next_turn && turns < MAX_CATCH_UP_TURNS {
            let tick = self
                .sim
                .tick()
                .checked_add(1)
                .ok_or("match tick overflow")?;
            let inputs = self
                .inputs
                .remove(&tick)
                .unwrap_or_else(|| vec![Vec::new(); self.last_input.len()])
                .into_iter()
                .zip(&self.knowledge)
                .map(|(commands, knowledge)| {
                    // Knowledge may have changed during the input delay.
                    commands
                        .into_iter()
                        .filter_map(|command| knowledge.revalidate(&self.sim, command))
                        .collect()
                })
                .collect();
            make_turn(tick, inputs)
                .apply(&mut self.sim)
                .map_err(|_| "invalid authoritative turn")?;
            for (slot, knowledge) in self.knowledge.iter_mut().enumerate() {
                messages.push((
                    slot,
                    ServerMessage::View {
                        frame: knowledge.observe(&self.sim),
                    },
                ));
            }
            turns += 1;
            self.next_turn = self.next_turn.saturating_add(TICK_DURATION);
        }
        if now >= self.next_turn {
            self.next_turn = now.saturating_add(TICK_DURATION);
        }
        Ok(messages)
    }
}

fn make_turn(tick: u32, inputs: Vec<Vec<Command>>) -> Turn {
    Turn {
        tick,
        inputs: inputs
            .into_iter()
            .enumerate()
            .map(|(slot, commands)| PlayerInput {
                player: PlayerId(u8::try_from(slot).expect("bounded player count")),
                commands,
            })
            .collect(),
    }
}

fn error(client: ClientId, message: &str) -> Delivery {
    Delivery {
        client,
        message: ServerMessage::Error {
            message: message.to_owned(),
        },
    }
}

fn lobby(code: &str, room: &Room) -> Vec<Delivery> {
    let slots: Vec<_> = room
        .slots
        .iter()
        .map(|slot| {
            slot.as_ref().map(|slot| LobbySlot {
                name: slot.name.clone(),
                ready: slot.ready,
            })
        })
        .collect();
    room.slots
        .iter()
        .enumerate()
        .filter_map(|(player, slot)| {
            slot.as_ref().map(|slot| Delivery {
                client: slot.client,
                message: ServerMessage::Lobby {
                    code: code.to_owned(),
                    player: PlayerId(u8::try_from(player).expect("bounded player count")),
                    public: room.public,
                    slots: slots.clone(),
                },
            })
        })
        .collect()
}

fn to_slot(room: &Room, slot: usize, message: ServerMessage) -> Option<Delivery> {
    room.slots[slot].as_ref().map(|member| Delivery {
        client: member.client,
        message,
    })
}

fn broadcast(room: &Room, message: ServerMessage) -> Vec<Delivery> {
    room.slots
        .iter()
        .flatten()
        .map(|slot| Delivery {
            client: slot.client,
            message: message.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;

    use super::*;

    fn coordinator(seed: u64) -> Coordinator {
        Coordinator::new(Config::default(), StdRng::seed_from_u64(seed))
    }

    #[test]
    fn codes_use_the_alphabet_and_vary_with_the_seed() {
        let codes: Vec<_> = (0..4)
            .map(|seed| coordinator(seed).allocate_code())
            .collect();
        for code in &codes {
            assert_eq!(code.len(), CODE_LEN);
            assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)), "{code}");
        }
        assert!(codes.windows(2).any(|pair| pair[0] != pair[1]), "{codes:?}");
    }

    #[test]
    fn allocation_skips_codes_of_live_rooms() {
        let mut server = coordinator(7);
        let taken = server.allocate_code();
        server.rooms.insert(
            taken.clone(),
            Room {
                slots: vec![None],
                public: false,
                game: None,
            },
        );
        // Replaying the same random sequence would produce the taken code first.
        server.rng = StdRng::seed_from_u64(7);
        let next = server.allocate_code();
        assert_ne!(next, taken);
        assert!(!server.rooms.contains_key(&next));
    }
}
