//! Transport-independent authoritative turn protocol, match setup and verified replay files.

mod minify;
mod replay;
mod setup;

use std::collections::{BTreeMap, BTreeSet};

use rts_base::{Command, MAX_PLAYERS, PlayerId, Sim};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub use replay::{MAX_REPLAY_BYTES, MAX_REPLAY_TURNS, Replay};
pub use rts_base::INPUT_DELAY_TICKS;
pub use setup::{MAX_SETUP_BYTES, MatchSetup};

pub const PROTOCOL_VERSION: u32 = 4;
/// Maximum client message and small server message size.
pub const MAX_MESSAGE_BYTES: usize = 65_536;
/// One bounded input per player, plus turn framing and authoritative hash overhead.
pub const MAX_TURN_MESSAGE_BYTES: usize = MAX_MESSAGE_BYTES * (MAX_PLAYERS + 1);
/// Bounds the binary setup header and both compressed RON documents.
pub const MAX_SERVER_MESSAGE_BYTES: usize = setup::wire::MAX_FRAME_BYTES;
/// Maximum commands in each player's batch for a turn.
pub const MAX_COMMANDS_PER_TURN: usize = 64;
pub const MAX_UNITS_PER_COMMAND: usize = 1024;
/// Total selected unit occurrences across a single player's batch.
pub const MAX_UNIT_SELECTIONS_PER_TURN: usize = 1024;
/// Allows ordered replacements such as Move then Stop without unbounded repeated pathfinding.
pub const MAX_ORDERS_PER_UNIT_PER_TURN: usize = 4;
const RON_RECURSION_LIMIT: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("invalid protocol data: {0}")]
    Invalid(String),
    #[error("failed to decode RON")]
    Decode(#[from] ron::error::SpannedError),
    #[error("failed to encode RON")]
    Encode(#[from] ron::Error),
    #[error("failed to load match setup")]
    Setup(#[from] rts_base::LoadError),
    #[error("final replay hash mismatch: expected {expected:016x}, got {actual:016x}")]
    HashMismatch { expected: u64, actual: u64 },
}

impl NetError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerInput {
    pub player: PlayerId,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    pub tick: u32,
    pub inputs: Vec<PlayerInput>,
}

impl Turn {
    /// Checks the next tick and exactly one ascending batch for every match player.
    ///
    /// # Errors
    /// Returns an error for a different tick, missing/duplicate players or excessive commands.
    pub fn validate(&self, expected_tick: u32, players: u8) -> Result<(), NetError> {
        validate_players(players)?;
        if self.tick == 0 || self.tick != expected_tick {
            return Err(NetError::invalid("turn tick is not the expected next tick"));
        }
        if self.inputs.len() != usize::from(players) {
            return Err(NetError::invalid(
                "turn must include every player exactly once",
            ));
        }
        for (slot, input) in (0..players).zip(&self.inputs) {
            if input.player != PlayerId(slot) {
                return Err(NetError::invalid(
                    "turn players must be in ascending slot order",
                ));
            }
            validate_commands(&input.commands)?;
        }
        Ok(())
    }

    /// Applies this turn at the next simulation tick without a second input delay.
    ///
    /// # Errors
    /// Returns an error without changing the simulation if this turn fails validation.
    pub fn apply(&self, sim: &mut Sim) -> Result<(), NetError> {
        let next = sim
            .tick()
            .checked_add(1)
            .ok_or_else(|| NetError::invalid("tick overflow"))?;
        let players = u8::try_from(sim.players().len())
            .map_err(|_| NetError::invalid("invalid simulation player count"))?;
        self.validate(next, players)?;
        sim.step_with_commands(self.inputs.iter().flat_map(|input| {
            input
                .commands
                .iter()
                .cloned()
                .map(|command| (input.player, command))
        }));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ClientMessage {
    Create { version: u32 },
    Join { version: u32, code: String },
    Ready { checksum: u64 },
    Start,
    Input { tick: u32, commands: Vec<Command> },
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ServerMessage {
    #[serde(skip)]
    Setup(MatchSetup),
    Lobby {
        code: String,
        player: PlayerId,
        players: u8,
        capacity: u8,
        ready: u8,
    },
    Started {
        players: u8,
        input_delay: u32,
    },
    Turn {
        turn: Turn,
        /// Authoritative simulation hash after applying this turn.
        hash: u64,
    },
    Ended {
        reason: String,
    },
    Error {
        message: String,
    },
}

/// An encoded WebSocket application message, without a socket-library dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerFrame {
    Text(String),
    Binary(Vec<u8>),
}

/// Validates a player's batch, bounding selections and repeated orders per unit.
///
/// # Errors
/// Returns an error for excessive commands/selections, duplicate selected IDs or too many
/// orders for one unit across the batch.
pub fn validate_commands(commands: &[Command]) -> Result<(), NetError> {
    if commands.len() > MAX_COMMANDS_PER_TURN {
        return Err(NetError::invalid("too many commands in player batch"));
    }
    let mut selections = 0;
    let mut orders = BTreeMap::new();
    for command in commands {
        let (Command::Move { units, .. } | Command::Stop { units }) = command;
        if units.len() > MAX_UNITS_PER_COMMAND {
            return Err(NetError::invalid("too many units in command"));
        }
        selections += units.len();
        if selections > MAX_UNIT_SELECTIONS_PER_TURN {
            return Err(NetError::invalid("too many selected units in player batch"));
        }
        let mut unique = BTreeSet::new();
        for &unit in units {
            if !unique.insert(unit) {
                return Err(NetError::invalid("duplicate unit in command"));
            }
            let count = orders.entry(unit).or_insert(0);
            *count += 1;
            if *count > MAX_ORDERS_PER_UNIT_PER_TURN {
                return Err(NetError::invalid(
                    "too many orders for one unit in player batch",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_players(players: u8) -> Result<(), NetError> {
    if players == 0 || usize::from(players) > MAX_PLAYERS {
        return Err(NetError::invalid("player count is out of range"));
    }
    Ok(())
}

pub(crate) fn validate_version(version: u32) -> Result<(), NetError> {
    if version != PROTOCOL_VERSION {
        return Err(NetError::invalid("unsupported protocol version"));
    }
    Ok(())
}

fn validate_code(code: &str) -> Result<(), NetError> {
    if code.len() != 6
        || !code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err(NetError::invalid(
            "room code must contain six uppercase letters or digits",
        ));
    }
    Ok(())
}

fn validate_tick(tick: u32) -> Result<(), NetError> {
    if tick == 0 {
        return Err(NetError::invalid("turn tick must be positive"));
    }
    Ok(())
}

impl ClientMessage {
    fn validate(&self) -> Result<(), NetError> {
        match self {
            Self::Create { version } => validate_version(*version),
            Self::Join { version, code, .. } => {
                validate_version(*version)?;
                validate_code(code)
            }
            Self::Input { tick, commands } => {
                validate_tick(*tick)?;
                validate_commands(commands)
            }
            Self::Ready { .. } | Self::Start | Self::Leave => Ok(()),
        }
    }
}

impl ServerMessage {
    const fn wire_limit(&self) -> usize {
        match self {
            Self::Setup(_) => MAX_SERVER_MESSAGE_BYTES,
            Self::Turn { .. } => MAX_TURN_MESSAGE_BYTES,
            _ => MAX_MESSAGE_BYTES,
        }
    }

    fn validate(&self) -> Result<(), NetError> {
        match self {
            Self::Setup(setup) => setup.simulation().map(|_| ()),
            Self::Lobby {
                code,
                player,
                players,
                capacity,
                ready,
            } => {
                validate_code(code)?;
                validate_players(*players)?;
                validate_players(*capacity)?;
                if players > capacity || player.0 >= *capacity || ready > players {
                    return Err(NetError::invalid("invalid lobby slots"));
                }
                Ok(())
            }
            Self::Started {
                players,
                input_delay,
            } => {
                validate_players(*players)?;
                if *input_delay != INPUT_DELAY_TICKS {
                    return Err(NetError::invalid("unsupported input delay"));
                }
                Ok(())
            }
            Self::Turn { turn, .. } => {
                let players = u8::try_from(turn.inputs.len())
                    .map_err(|_| NetError::invalid("too many turn inputs"))?;
                turn.validate(turn.tick, players)
            }
            Self::Ended { .. } | Self::Error { .. } => Ok(()),
        }
    }
}

pub(crate) fn check_size(size: usize, limit: usize) -> Result<(), NetError> {
    if size > limit {
        return Err(NetError::invalid(format!("data exceeds {limit} bytes")));
    }
    Ok(())
}

pub(crate) fn decode<T: DeserializeOwned>(text: &str, limit: usize) -> Result<T, NetError> {
    check_size(text.len(), limit)?;
    Ok(ron::Options::default()
        .with_recursion_limit(RON_RECURSION_LIMIT)
        .from_str(text)?)
}

pub(crate) fn encode<T: Serialize>(value: &T, limit: usize) -> Result<String, NetError> {
    let text = ron::Options::default()
        .with_recursion_limit(RON_RECURSION_LIMIT)
        .to_string(value)?;
    check_size(text.len(), limit)?;
    Ok(text)
}

/// # Errors
/// Returns an error for malformed, incompatible, or oversized messages.
pub fn decode_client(text: &str) -> Result<ClientMessage, NetError> {
    let message: ClientMessage = decode(text, MAX_MESSAGE_BYTES)?;
    message.validate()?;
    Ok(message)
}

/// Decodes a server text frame. Setup is accepted only by [`decode_server_binary`].
///
/// # Errors
/// Returns an error for malformed, incompatible, or oversized messages.
pub fn decode_server(text: &str) -> Result<ServerMessage, NetError> {
    let message: ServerMessage = decode(text, MAX_TURN_MESSAGE_BYTES)?;
    check_size(text.len(), message.wire_limit())?;
    message.validate()?;
    Ok(message)
}

/// Decodes a binary setup frame and validates its decompressed configuration.
///
/// # Errors
/// Returns an error for an incompatible header, invalid lengths, corrupt streams or setup.
pub fn decode_server_binary(bytes: &[u8]) -> Result<ServerMessage, NetError> {
    let message = ServerMessage::Setup(setup::wire::decode(bytes)?);
    message.validate()?;
    Ok(message)
}

/// # Errors
/// Returns an error for invalid or oversized messages or failed RON serialization.
pub fn encode_client(message: &ClientMessage) -> Result<String, NetError> {
    message.validate()?;
    encode(message, MAX_MESSAGE_BYTES)
}

/// Encodes setup as raw binary and other messages as RON text.
///
/// # Errors
/// Returns an error for invalid setup/messages, excessive sizes or failed serialization.
pub fn encode_server(message: &ServerMessage) -> Result<ServerFrame, NetError> {
    message.validate()?;
    match message {
        ServerMessage::Setup(setup) => setup::wire::encode(setup).map(ServerFrame::Binary),
        _ => encode(message, message.wire_limit()).map(ServerFrame::Text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ron_decoder_bounds_nested_data() {
        let nested = format!("{}0{}", "[".repeat(128), "]".repeat(128));
        let error = decode::<ron::Value>(&nested, MAX_MESSAGE_BYTES).unwrap_err();
        assert!(matches!(
            error,
            NetError::Decode(ron::error::SpannedError {
                code: ron::Error::ExceededRecursionLimit,
                ..
            })
        ));
    }
}
