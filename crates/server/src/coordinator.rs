use std::{collections::BTreeMap, time::Duration};

use rts_base::{Command, MAX_PLAYERS, PlayerId};
use rts_net::{
    ClientMessage, INPUT_DELAY_TICKS, PROTOCOL_VERSION, PlayerInput, ServerMessage, Turn,
    validate_commands,
};

/// Connection identity allocated by the socket runtime, never supplied by a client.
pub type ClientId = u64;

/// Maximum buffered input/hash lead. Also bounds retained, unconfirmed turns.
pub const MAX_FUTURE_TICKS: u32 = 8;

/// Resource and timeout limits for one relay process.
#[derive(Clone, Debug)]
pub struct Config {
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
            max_connections: 1024,
            max_rooms: 256,
            event_queue_capacity: 256,
            outbound_queue_capacity: 64,
            peer_timeout: Duration::from_secs(15),
            lobby_timeout: Duration::from_mins(30),
            handshake_timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(250),
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
    checksum: u64,
    slots: Vec<Option<ClientId>>,
    game: Option<Game>,
}

struct Game {
    next_input: Vec<u32>,
    next_hash: Vec<u32>,
    inputs: BTreeMap<u32, Vec<Option<Vec<Command>>>>,
    hashes: BTreeMap<u32, Vec<Option<u64>>>,
    emitted: u32,
    confirmed: u32,
    last_turn: Duration,
    last_hash: Duration,
}

/// Pure room coordinator. Callers supply monotonically increasing elapsed time.
pub struct Coordinator {
    config: Config,
    peers: BTreeMap<ClientId, Peer>,
    rooms: BTreeMap<String, Room>,
    next_code: u64,
}

impl Coordinator {
    #[must_use]
    pub const fn new(config: Config) -> Self {
        Self {
            config,
            peers: BTreeMap::new(),
            rooms: BTreeMap::new(),
            next_code: 0,
        }
    }

    /// Reserves a connection, including connections still doing their handshake.
    pub fn connect(&mut self, client: ClientId, now: Duration) -> bool {
        if self.peers.len() >= self.config.max_connections || self.peers.contains_key(&client) {
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
            ClientMessage::Create {
                version,
                checksum,
                players,
            } => self.create(client, version, checksum, players),
            ClientMessage::Join {
                version,
                code,
                checksum,
            } => self.join(client, version, &code, checksum),
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
        for slot in &mut room.slots {
            if *slot == Some(client) {
                *slot = None;
            }
        }
        let messages = lobby(&code, &room);
        self.rooms.insert(code, room);
        messages
    }

    /// Expires silent sockets and matches with stalled inputs or hash reports.
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
        let stalled: Vec<_> = self
            .rooms
            .iter()
            .filter_map(|(code, room)| {
                room.game
                    .as_ref()
                    .filter(|game| {
                        now.saturating_sub(game.last_turn) >= self.config.peer_timeout
                            || now.saturating_sub(game.last_hash) >= self.config.peer_timeout
                    })
                    .map(|_| code.clone())
            })
            .collect();
        for code in stalled {
            if let Some(room) = self.rooms.remove(&code) {
                messages
                    .extend(self.end_room(room, "match timed out waiting for inputs or hashes"));
            }
        }
        messages
    }

    fn create(
        &mut self,
        client: ClientId,
        version: u32,
        checksum: u64,
        players: u8,
    ) -> Result<Vec<Delivery>, &'static str> {
        self.available(client, version)?;
        if players == 0 || usize::from(players) > MAX_PLAYERS {
            return Err("invalid player count");
        }
        if self.rooms.len() >= self.config.max_rooms {
            return Err("server room limit reached");
        }
        let code = self.allocate_code();
        let mut slots = vec![None; usize::from(players)];
        slots[0] = Some(client);
        let room = Room {
            checksum,
            slots,
            game: None,
        };
        self.peers
            .get_mut(&client)
            .expect("registered connection")
            .room = Some(code.clone());
        let messages = lobby(&code, &room);
        self.rooms.insert(code, room);
        Ok(messages)
    }

    fn join(
        &mut self,
        client: ClientId,
        version: u32,
        code: &str,
        checksum: u64,
    ) -> Result<Vec<Delivery>, &'static str> {
        self.available(client, version)?;
        if code.len() != 6
            || !code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err("invalid room code");
        }
        let room = self.rooms.get_mut(code).ok_or("room does not exist")?;
        if room.game.is_some() {
            return Err("match has already started");
        }
        if room.checksum != checksum {
            return Err("setup checksum does not match");
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
        Ok(lobby(code, room))
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

    fn allocate_code(&mut self) -> String {
        const ALPHABET: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        loop {
            let mut value = self.next_code;
            self.next_code = (self.next_code + 1) % 36_u64.pow(6);
            let mut bytes = [b'A'; 6];
            for byte in bytes.iter_mut().rev() {
                *byte = ALPHABET[usize::try_from(value % 36).expect("base 36 digit")];
                value /= 36;
            }
            let code = String::from_utf8(bytes.to_vec()).expect("ASCII room code");
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
        let action = room.receive(slot, message, now);
        match action {
            Ok(RoomAction::End(reason)) => Ok(self.end_room(room, reason)),
            Ok(RoomAction::Messages(messages)) => {
                // Lobby silence is allowed for much longer than match silence.
                // Give every slot a complete heartbeat window after starting.
                if messages
                    .iter()
                    .any(|message| matches!(message, ServerMessage::Started { .. }))
                {
                    for &id in room.slots.iter().flatten() {
                        self.touch(id, now);
                    }
                }
                let result = messages
                    .into_iter()
                    .flat_map(|message| broadcast(&room, message))
                    .collect();
                self.rooms.insert(code, room);
                Ok(result)
            }
            Err(error) => {
                self.rooms.insert(code, room);
                Err(error)
            }
        }
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

enum RoomAction {
    Messages(Vec<ServerMessage>),
    End(&'static str),
}

impl Room {
    fn receive(
        &mut self,
        slot: usize,
        message: ClientMessage,
        now: Duration,
    ) -> Result<RoomAction, &'static str> {
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
            let players = u8::try_from(self.slots.len()).expect("bounded player count");
            self.game = Some(Game {
                next_input: vec![2; self.slots.len()],
                next_hash: vec![1; self.slots.len()],
                inputs: BTreeMap::new(),
                hashes: BTreeMap::new(),
                emitted: 1,
                confirmed: 0,
                last_turn: now,
                last_hash: now,
            });
            return Ok(RoomAction::Messages(vec![
                ServerMessage::Started {
                    players,
                    input_delay: INPUT_DELAY_TICKS,
                },
                ServerMessage::Turn(Turn {
                    tick: 1,
                    inputs: (0..players)
                        .map(|player| PlayerInput {
                            player: PlayerId(player),
                            commands: Vec::new(),
                        })
                        .collect(),
                }),
            ]));
        }
        let game = self.game.as_mut().ok_or("match has not started")?;
        match message {
            ClientMessage::Input { tick, commands } => game
                .input(slot, tick, commands, now)
                .map(RoomAction::Messages),
            ClientMessage::Hash { tick, hash } => game.hash(slot, tick, hash, now),
            _ => Err("invalid message for a running match"),
        }
    }
}

impl Game {
    fn input(
        &mut self,
        slot: usize,
        tick: u32,
        commands: Vec<Command>,
        now: Duration,
    ) -> Result<Vec<ServerMessage>, &'static str> {
        if tick != self.next_input[slot] {
            return Err("input ticks must be sequential without duplicates");
        }
        if tick == u32::MAX
            || tick > self.emitted.saturating_add(MAX_FUTURE_TICKS)
            || tick > self.confirmed.saturating_add(MAX_FUTURE_TICKS)
        {
            return Err("input tick exceeds the permitted future lead");
        }
        validate_commands(&commands).map_err(|_| "input exceeds command limits")?;
        let players = self.next_input.len();
        self.inputs
            .entry(tick)
            .or_insert_with(|| vec![None; players])[slot] = Some(commands);
        self.next_input[slot] += 1;
        let mut messages = Vec::new();
        while self
            .inputs
            .get(&(self.emitted + 1))
            .is_some_and(|inputs| inputs.iter().all(Option::is_some))
        {
            self.emitted += 1;
            let inputs = self
                .inputs
                .remove(&self.emitted)
                .expect("complete input frame");
            messages.push(ServerMessage::Turn(Turn {
                tick: self.emitted,
                inputs: inputs
                    .into_iter()
                    .enumerate()
                    .map(|(player, commands)| PlayerInput {
                        player: PlayerId(u8::try_from(player).expect("bounded player count")),
                        commands: commands.expect("complete input frame"),
                    })
                    .collect(),
            }));
            self.last_turn = now;
        }
        Ok(messages)
    }

    fn hash(
        &mut self,
        slot: usize,
        tick: u32,
        hash: u64,
        now: Duration,
    ) -> Result<RoomAction, &'static str> {
        if tick != self.next_hash[slot]
            || tick > self.emitted
            || tick == u32::MAX
            || tick > self.confirmed.saturating_add(MAX_FUTURE_TICKS)
        {
            return Err(
                "hash ticks must be sequential and refer to emitted turns within the permitted lead",
            );
        }
        let players = self.next_hash.len();
        let hashes = self
            .hashes
            .entry(tick)
            .or_insert_with(|| vec![None; players]);
        if hashes.iter().flatten().any(|&reported| reported != hash) {
            return Ok(RoomAction::End("state hash mismatch"));
        }
        hashes[slot] = Some(hash);
        self.next_hash[slot] += 1;
        let mut messages = Vec::new();
        while self
            .hashes
            .get(&(self.confirmed + 1))
            .is_some_and(|hashes| hashes.iter().all(Option::is_some))
        {
            self.confirmed += 1;
            let hashes = self
                .hashes
                .remove(&self.confirmed)
                .expect("complete hash frame");
            messages.push(ServerMessage::HashConfirmed {
                tick: self.confirmed,
                hash: hashes[0].expect("complete hash frame"),
            });
            self.last_hash = now;
        }
        Ok(RoomAction::Messages(messages))
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
