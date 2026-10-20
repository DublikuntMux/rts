use rts_base::Sim;
use serde::{Deserialize, Serialize};

use crate::{MatchSetup, NetError, PROTOCOL_VERSION, Turn, decode, encode, validate_version};

pub const MAX_REPLAY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_REPLAY_TURNS: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    pub version: u32,
    pub setup: MatchSetup,
    pub turns: Vec<Turn>,
    pub final_hash: u64,
}

impl Replay {
    /// Starts a recording with the hash of the validated initial simulation.
    ///
    /// # Errors
    /// Returns an error if the match setup is invalid.
    pub fn new(setup: MatchSetup) -> Result<Self, NetError> {
        let final_hash = setup.simulation()?.state_hash();
        Ok(Self {
            version: PROTOCOL_VERSION,
            setup,
            turns: Vec::new(),
            final_hash,
        })
    }

    /// Records one applied turn and its resulting hash.
    ///
    /// # Errors
    /// Returns an error for unsupported versions, excessive turns or a nonsequential turn.
    pub fn push(&mut self, turn: Turn, hash: u64) -> Result<(), NetError> {
        validate_version(self.version)?;
        if self.turns.len() >= MAX_REPLAY_TURNS {
            return Err(NetError::invalid("too many replay turns"));
        }
        let tick = u32::try_from(self.turns.len() + 1)
            .map_err(|_| NetError::invalid("replay tick overflow"))?;
        turn.validate(tick, self.setup.players)?;
        self.turns.push(turn);
        self.final_hash = hash;
        Ok(())
    }

    /// # Errors
    /// Returns an error for invalid recordings, final hash mismatches or excessive output.
    pub fn to_ron(&self) -> Result<String, NetError> {
        self.playback()?;
        encode(self, MAX_REPLAY_BYTES)
    }

    /// # Errors
    /// Returns an error for malformed, excessive or invalid recordings, including corruption.
    pub fn from_ron(text: &str) -> Result<Self, NetError> {
        let replay: Self = decode(text, MAX_REPLAY_BYTES)?;
        replay.playback()?;
        Ok(replay)
    }

    /// Reconstructs the match and verifies the final state against the recorded hash.
    ///
    /// # Errors
    /// Returns an error for invalid setup, version, turns or a final hash mismatch.
    pub fn playback(&self) -> Result<Sim, NetError> {
        validate_version(self.version)?;
        if self.turns.len() > MAX_REPLAY_TURNS {
            return Err(NetError::invalid("too many replay turns"));
        }
        let mut sim = self.setup.simulation()?;
        for turn in &self.turns {
            turn.apply(&mut sim)?;
        }
        let actual = sim.state_hash();
        if actual != self.final_hash {
            return Err(NetError::HashMismatch {
                expected: self.final_hash,
                actual,
            });
        }
        Ok(sim)
    }
}
