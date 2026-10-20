use rts_base::{MapData, Rules, Sim};
use serde::{Deserialize, Serialize};

use crate::{NetError, PROTOCOL_VERSION, check_size, minify::minify_ron, validate_players};

pub mod wire;

/// Maximum bytes in each original rules/map document.
pub const MAX_SETUP_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchSetup {
    pub rules: String,
    pub map: String,
    /// Reserved for deterministic randomness; included in the setup checksum today.
    pub seed: u64,
    pub players: u8,
}

impl MatchSetup {
    /// Removes comments and unnecessary whitespace without changing RON literals.
    /// Validation remains the responsibility of [`Self::simulation`].
    #[must_use]
    pub fn minified(&self) -> Self {
        Self {
            rules: minify_ron(&self.rules),
            map: minify_ron(&self.map),
            seed: self.seed,
            players: self.players,
        }
    }

    /// FNV-1a over version, length-prefixed minified RON, seed and player count.
    /// All multibyte integers have explicit fixed widths and little-endian encoding.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        let compact = self.minified();
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for bytes in [
            PROTOCOL_VERSION.to_le_bytes().as_slice(),
            (compact.rules.len() as u64).to_le_bytes().as_slice(),
            compact.rules.as_bytes(),
            (compact.map.len() as u64).to_le_bytes().as_slice(),
            compact.map.as_bytes(),
            self.seed.to_le_bytes().as_slice(),
            &[self.players],
        ] {
            for &byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        hash
    }

    /// Loads the original data without reserializing already converted world coordinates.
    ///
    /// # Errors
    /// Returns an error for excessive data, invalid rules/map or a different player count.
    pub fn simulation(&self) -> Result<Sim, NetError> {
        validate_players(self.players)?;
        check_size(self.rules.len(), MAX_SETUP_BYTES)?;
        check_size(self.map.len(), MAX_SETUP_BYTES)?;
        // Base parsers also have RON's finite recursion limit (128).
        let rules = Rules::from_ron(&self.rules)?;
        let map = MapData::from_ron(&self.map)?;
        if map.players.len() != usize::from(self.players) {
            return Err(NetError::invalid(
                "setup player count differs from map slots",
            ));
        }
        Ok(Sim::new(rules, &map)?)
    }
}
