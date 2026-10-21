use std::{collections::BTreeMap, time::Duration};

use rand::{RngExt, rngs::StdRng};
use rts_base::{Command, PlayerId, Sim, TICK_RATE};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, MatchSetup, PROTOCOL_VERSION, PlayerInput, ServerMessage,
    Turn, encode_client, encode_server, validate_commands,
};

/// Connection identity allocated by the socket runtime, never supplied by a client.
pub type ClientId = u64;

/// Maximum buffered input lead relative to the authoritative simulation tick.
pub const MAX_FUTURE_TICKS: u32 = 8;

/// Resource limits, socket timeouts and server-owned match content.
#[derive(Clone, Debug)]
pub struct Config {
    pub setup: MatchSetup,
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
                map: include_str!("../../../assets/maps/proving_grounds.map.ron").to_owned(),
                seed: 0,
                players: 2,
            },
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

struct Room {
    slots: Vec<Option<ClientId>>,
    ready: Vec<bool>,
    game: Option<Game>,
}

const TICK_DURATION: Duration = Duration::from_millis(1000 / TICK_RATE as u64);
/// A long scheduler stall advances at most this many turns before resetting the deadline.
pub const MAX_CATCH_UP_TURNS: usize = 4;

struct Game {
    sim: Sim,
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
    initial_sim: Option<Sim>,
    peers: BTreeMap<ClientId, Peer>,
    rooms: BTreeMap<String, Room>,
    /// Chooses room codes. Codes admit players to rooms, so seed it from OS entropy.
    codes: StdRng,
}

impl Coordinator {
    /// `codes` chooses room codes. Anyone who knows a waiting room's code can join it, so
    /// production servers seed it from OS entropy; tests may use a fixed seed.
    #[must_use]
    pub fn new(config: Config, codes: StdRng) -> Self {
        let initial_sim = config.setup.simulation().ok();
        Self {
            config,
            initial_sim,
            peers: BTreeMap::new(),
            rooms: BTreeMap::new(),
            codes,
        }
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
            ClientMessage::Create { version } => self.create(client, version),
            ClientMessage::Join { version, code } => self.join(client, version, &code),
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
        if room.game.is_some() || room.slots[0] == Some(client) {
            return self.end_room(room, "a player disconnected");
        }
        for (slot, ready) in room.slots.iter_mut().zip(&mut room.ready) {
            if *slot == Some(client) {
                *slot = None;
                *ready = false;
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
                Ok(turns) => {
                    for turn in turns {
                        messages.extend(broadcast(room, turn));
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

    fn create(&mut self, client: ClientId, version: u32) -> Result<Vec<Delivery>, &'static str> {
        self.available(client, version)?;
        if self.rooms.len() >= self.config.max_rooms {
            return Err("server room limit reached");
        }
        let code = self.allocate_code();
        let mut slots = vec![None; usize::from(self.config.setup.players)];
        slots[0] = Some(client);
        let room = Room {
            ready: vec![false; slots.len()],
            slots,
            game: None,
        };
        self.peers
            .get_mut(&client)
            .expect("registered connection")
            .room = Some(code.clone());
        let mut messages = vec![Delivery {
            client,
            message: ServerMessage::Setup(self.config.setup.clone()),
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
    ) -> Result<Vec<Delivery>, &'static str> {
        self.available(client, version)?;
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
        *slot = Some(client);
        self.peers
            .get_mut(&client)
            .expect("registered connection")
            .room = Some(code.to_owned());
        let mut messages = vec![Delivery {
            client,
            message: ServerMessage::Setup(self.config.setup.clone()),
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
                .map(|_| char::from(CODE_ALPHABET[self.codes.random_range(0..CODE_ALPHABET.len())]))
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
            .position(|slot| *slot == Some(client))
            .expect("room member");
        if matches!(message, ClientMessage::Leave) {
            if slot == 0 || room.game.is_some() {
                return Ok(self.end_room(room, "a player left the room"));
            }
            room.slots[slot] = None;
            room.ready[slot] = false;
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
        let action = if let ClientMessage::Ready { checksum } = message {
            if room.game.is_some() {
                Err("match has already started")
            } else if checksum != self.config.setup.checksum() {
                Err("setup checksum does not match")
            } else {
                room.ready[slot] = true;
                Ok(lobby(&code, &room))
            }
        } else {
            room.receive(slot, message, now, self.initial_sim.as_ref())
                .map(|messages| {
                    if messages
                        .iter()
                        .any(|m| matches!(m, ServerMessage::Started { .. }))
                    {
                        for &id in room.slots.iter().flatten() {
                            self.touch(id, now);
                        }
                    }
                    messages
                        .into_iter()
                        .flat_map(|m| broadcast(&room, m))
                        .collect()
                })
        };
        self.rooms.insert(code, room);
        action
    }

    fn end_room(&mut self, room: Room, reason: &str) -> Vec<Delivery> {
        for id in room.slots.iter().flatten() {
            if let Some(peer) = self.peers.get_mut(id) {
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

impl Room {
    fn receive(
        &mut self,
        slot: usize,
        message: ClientMessage,
        now: Duration,
        initial_sim: Option<&Sim>,
    ) -> Result<Vec<ServerMessage>, &'static str> {
        if matches!(message, ClientMessage::Start) {
            if slot != 0 {
                return Err("only the host can start the match");
            }
            if self.game.is_some() {
                return Err("match has already started");
            }
            if self.slots.iter().any(Option::is_none) {
                return Err("room is not full");
            }
            if !self.ready.iter().all(|&ready| ready) {
                return Err("all players must acknowledge the setup");
            }
            let sim = initial_sim.ok_or("invalid server setup")?.clone();
            let players = u8::try_from(self.slots.len()).expect("bounded player count");
            self.game = Some(Game {
                sim,
                last_input: vec![0; self.slots.len()],
                inputs: BTreeMap::new(),
                next_turn: now.saturating_add(TICK_DURATION),
            });
            return Ok(vec![ServerMessage::Started {
                players,
                input_delay: INPUT_DELAY_TICKS,
            }]);
        }
        let game = self.game.as_mut().ok_or("match has not started")?;
        match message {
            ClientMessage::Input { tick, commands } => {
                game.input(slot, tick, commands)?;
                Ok(Vec::new())
            }
            _ => Err("invalid message for a running match"),
        }
    }
}

impl Game {
    fn input(
        &mut self,
        slot: usize,
        tick: u32,
        mut commands: Vec<Command>,
    ) -> Result<(), &'static str> {
        if tick <= self.sim.tick() || tick <= self.last_input[slot] {
            return Err("input tick is past, duplicate or out of order");
        }
        if tick > self.sim.tick().saturating_add(MAX_FUTURE_TICKS) {
            return Err("input tick exceeds the permitted future lead");
        }
        validate_commands(&commands).map_err(|_| "input exceeds command limits")?;
        encode_client(&ClientMessage::Input {
            tick,
            commands: commands.clone(),
        })
        .map_err(|_| "input exceeds message limits")?;
        let player = PlayerId(u8::try_from(slot).expect("bounded player count"));
        for command in &mut commands {
            let (Command::Move { units, .. } | Command::Stop { units }) = command;
            units.retain(|&id| self.sim.unit(id).is_some_and(|unit| unit.owner == player));
        }
        commands.retain(|command| {
            let (Command::Move { units, .. } | Command::Stop { units }) = command;
            !units.is_empty()
        });
        let mut inputs = self
            .inputs
            .get(&tick)
            .cloned()
            .unwrap_or_else(|| vec![Vec::new(); self.last_input.len()]);
        inputs[slot] = commands;
        // Bound the combined broadcast too, reserving the longest possible hash encoding.
        encode_server(&ServerMessage::Turn {
            turn: make_turn(tick, inputs.clone()),
            hash: u64::MAX,
        })
        .map_err(|_| "combined turn exceeds message limits")?;
        self.inputs.insert(tick, inputs);
        self.last_input[slot] = tick;
        Ok(())
    }

    fn advance(&mut self, now: Duration) -> Result<Vec<ServerMessage>, &'static str> {
        let mut messages = Vec::new();
        while now >= self.next_turn && messages.len() < MAX_CATCH_UP_TURNS {
            let tick = self
                .sim
                .tick()
                .checked_add(1)
                .ok_or("match tick overflow")?;
            let inputs = self
                .inputs
                .remove(&tick)
                .unwrap_or_else(|| vec![Vec::new(); self.last_input.len()]);
            let turn = make_turn(tick, inputs);
            turn.apply(&mut self.sim)
                .map_err(|_| "invalid authoritative turn")?;
            messages.push(ServerMessage::Turn {
                turn,
                hash: self.sim.state_hash(),
            });
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
    let players = u8::try_from(room.slots.iter().flatten().count()).expect("bounded player count");
    let capacity = u8::try_from(room.slots.len()).expect("bounded player count");
    room.slots
        .iter()
        .enumerate()
        .filter_map(|(player, &client)| {
            client.map(|client| Delivery {
                client,
                message: ServerMessage::Lobby {
                    code: code.to_owned(),
                    player: PlayerId(u8::try_from(player).expect("bounded player count")),
                    players,
                    capacity,
                    ready: u8::try_from(room.ready.iter().filter(|&&ready| ready).count())
                        .expect("bounded player count"),
                },
            })
        })
        .collect()
}

fn broadcast(room: &Room, message: ServerMessage) -> Vec<Delivery> {
    room.slots
        .iter()
        .flatten()
        .map(|&client| Delivery {
            client,
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
                ready: vec![false],
                game: None,
            },
        );
        // Replaying the same random sequence would produce the taken code first.
        server.codes = StdRng::seed_from_u64(7);
        let next = server.allocate_code();
        assert_ne!(next, taken);
        assert!(!server.rooms.contains_key(&next));
    }
}
