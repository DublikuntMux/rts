use rts_base::{Command, PlayerView, Rules, Sim, ViewFrame, ViewIntro};
use serde::{Deserialize, Serialize};

use crate::{
    MAX_MAP_NAME_CHARS, MatchSetup, NetError, Turn, check_size, decode, encode, validate_map_name,
};

pub const MAX_REPLAY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_REPLAY_TURNS: usize = 1_000_000;

/// Version of full replays: the setup and every player's turns. Version 9 matches enforce
/// combat sight; version 8 recordings keep their original combat rules.
pub const REPLAY_VERSION: u32 = 9;
const LEGACY_REPLAY_VERSION: u32 = 8;
/// Version of player-perspective replays: one player's view frames.
pub const VIEW_REPLAY_VERSION: u32 = 9;
// Version 7's sandbox simulation has the same deterministic hash stream.
const SANDBOX_REPLAY_VERSION: u32 = 7;

fn validate_version(version: u32) -> Result<(), NetError> {
    if version != REPLAY_VERSION && version != LEGACY_REPLAY_VERSION {
        return Err(NetError::invalid("replay version is not supported"));
    }
    Ok(())
}

/// A full recording of a local match: the setup and every player's turns. Saved games and solo
/// replays use it; playback verifies the final state hash.
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
        let final_hash = match_simulation(&setup, REPLAY_VERSION)?.state_hash();
        Ok(Self {
            version: REPLAY_VERSION,
            setup,
            turns: Vec::new(),
            final_hash,
        })
    }

    /// The initial simulation with the combat rules this recording's version uses.
    ///
    /// # Errors
    /// Returns an error for an invalid setup.
    pub fn simulation(&self) -> Result<Sim, NetError> {
        match_simulation(&self.setup, self.version)
    }

    /// Records one applied turn and its resulting hash.
    ///
    /// # Errors
    /// Returns an error for unsupported versions, excessive turns or a nonsequential turn.
    pub fn push(&mut self, turn: Turn, hash: u64) -> Result<(), NetError> {
        if self.version != SANDBOX_REPLAY_VERSION {
            validate_version(self.version)?;
        }
        if self.turns.len() >= MAX_REPLAY_TURNS {
            return Err(NetError::invalid("too many replay turns"));
        }
        let tick = u32::try_from(self.turns.len() + 1)
            .map_err(|_| NetError::invalid("replay tick overflow"))?;
        turn.validate(tick, self.setup.players)?;
        self.validate_turn_version(&turn)?;
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
        Self::from_ron_with_sim(text).map(|(replay, _)| replay)
    }

    /// Like [`Self::from_ron`], and also returns the verified final simulation, so callers that
    /// continue the match do not play it back twice.
    ///
    /// # Errors
    /// Returns an error for malformed, excessive or invalid recordings, including corruption.
    pub fn from_ron_with_sim(text: &str) -> Result<(Self, Sim), NetError> {
        let replay: Self = decode(text, MAX_REPLAY_BYTES)?;
        let sim = replay.playback()?;
        Ok((replay, sim))
    }

    /// Reconstructs the match and verifies the final state against the recorded hash.
    ///
    /// # Errors
    /// Returns an error for invalid setup, version, turns or a final hash mismatch.
    pub fn playback(&self) -> Result<Sim, NetError> {
        if self.version != SANDBOX_REPLAY_VERSION {
            validate_version(self.version)?;
        }
        if self.turns.len() > MAX_REPLAY_TURNS {
            return Err(NetError::invalid("too many replay turns"));
        }
        let mut sim = self.simulation()?;
        if self.version == SANDBOX_REPLAY_VERSION && sim.is_skirmish() {
            return Err(NetError::invalid("skirmish requires replay version 8"));
        }
        for turn in &self.turns {
            self.validate_turn_version(turn)?;
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

    fn validate_turn_version(&self, turn: &Turn) -> Result<(), NetError> {
        if self.version == SANDBOX_REPLAY_VERSION
            && turn
                .inputs
                .iter()
                .flat_map(|input| &input.commands)
                .any(|command| {
                    !matches!(
                        command,
                        Command::Move { .. } | Command::Stop { .. } | Command::Attack { .. },
                    )
                })
        {
            return Err(NetError::invalid(
                "economy commands require replay version 8",
            ));
        }
        Ok(())
    }
}

/// Builds a match for a full replay version: current recordings fight only what they see.
fn match_simulation(setup: &MatchSetup, version: u32) -> Result<Sim, NetError> {
    let mut sim = setup.simulation()?;
    if version >= REPLAY_VERSION {
        sim.enforce_combat_sight();
    }
    Ok(sim)
}

/// One player's recording of a match: the frames that player received. Online matches record
/// this because clients never receive the full setup or other players' commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewReplay {
    pub version: u32,
    /// Display name of the map.
    pub map: String,
    pub rules: String,
    pub intro: ViewIntro,
    pub frames: Vec<ViewFrame>,
}

impl ViewReplay {
    /// Starts an empty recording.
    ///
    /// # Errors
    /// Returns an error for invalid rules or an inconsistent intro.
    pub fn new(map: String, rules: String, intro: ViewIntro) -> Result<Self, NetError> {
        let replay = Self {
            version: VIEW_REPLAY_VERSION,
            map,
            rules,
            intro,
            frames: Vec::new(),
        };
        replay.view()?;
        Ok(replay)
    }

    /// An empty view ready for the first frame.
    ///
    /// # Errors
    /// Returns an error for an unsupported version, invalid rules, or an inconsistent intro.
    pub fn view(&self) -> Result<PlayerView, NetError> {
        if self.version != VIEW_REPLAY_VERSION {
            return Err(NetError::invalid("view replay version is not supported"));
        }
        validate_map_name(&self.map)?;
        check_size(self.map.len(), MAX_MAP_NAME_CHARS * 4)?;
        check_size(self.rules.len(), crate::MAX_SETUP_BYTES)?;
        Ok(PlayerView::new(Rules::from_ron(&self.rules)?, self.intro)?)
    }

    /// Records the next frame.
    ///
    /// # Errors
    /// Returns an error when the recording is full.
    pub fn push(&mut self, frame: ViewFrame) -> Result<(), NetError> {
        if self.frames.len() >= MAX_REPLAY_TURNS {
            return Err(NetError::invalid("too many replay frames"));
        }
        self.frames.push(frame);
        Ok(())
    }

    /// Ticks after the first frame.
    #[must_use]
    pub fn ticks(&self) -> u32 {
        u32::try_from(self.frames.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    /// Applies every frame, checking that each one is valid and in sequence.
    ///
    /// # Errors
    /// Returns an error for an invalid header or frame.
    pub fn playback(&self) -> Result<PlayerView, NetError> {
        if self.frames.len() > MAX_REPLAY_TURNS {
            return Err(NetError::invalid("too many replay frames"));
        }
        let mut view = self.view()?;
        for frame in &self.frames {
            view.apply(frame.clone())?;
        }
        Ok(view)
    }
}

/// Either kind of recorded match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recording {
    Match(Replay),
    View(ViewReplay),
}

impl Recording {
    /// Decodes and verifies either kind of recording. A full replay also returns its final
    /// simulation.
    ///
    /// # Errors
    /// Returns an error for malformed, excessive, or invalid recordings.
    pub fn from_ron(text: &str) -> Result<(Self, Option<Sim>), NetError> {
        if let Ok(replay) = decode::<ViewReplay>(text, MAX_REPLAY_BYTES) {
            replay.playback()?;
            return Ok((Self::View(replay), None));
        }
        let (replay, sim) = Replay::from_ron_with_sim(text)?;
        Ok((Self::Match(replay), Some(sim)))
    }

    /// # Errors
    /// Returns an error for invalid recordings or excessive output.
    pub fn to_ron(&self) -> Result<String, NetError> {
        match self {
            Self::Match(replay) => replay.to_ron(),
            Self::View(replay) => {
                replay.playback()?;
                encode(replay, MAX_REPLAY_BYTES)
            }
        }
    }

    /// Recorded ticks.
    #[must_use]
    pub fn ticks(&self) -> u32 {
        match self {
            Self::Match(replay) => u32::try_from(replay.turns.len()).unwrap_or(u32::MAX),
            Self::View(replay) => replay.ticks(),
        }
    }

    #[must_use]
    pub const fn players(&self) -> u8 {
        match self {
            Self::Match(replay) => replay.setup.players,
            Self::View(replay) => replay.intro.players,
        }
    }
}
