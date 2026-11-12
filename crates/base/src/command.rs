use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::{IVec2, UnitId};

/// Commands issued by a player. The simulation checks ownership when applying them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Command {
    Move {
        units: Vec<UnitId>,
        target: IVec2,
    },
    Stop {
        units: Vec<UnitId>,
    },
    /// Armed units pursue and fire at another player's unit until it dies.
    Attack {
        units: Vec<UnitId>,
        target: UnitId,
    },
}

impl Command {
    /// The units this command orders.
    #[must_use]
    pub fn units(&self) -> &[UnitId] {
        match self {
            Self::Move { units, .. } | Self::Stop { units } | Self::Attack { units, .. } => units,
        }
    }

    /// Mutable access to the ordered units, for filtering before submission.
    pub const fn units_mut(&mut self) -> &mut Vec<UnitId> {
        match self {
            Self::Move { units, .. } | Self::Stop { units } | Self::Attack { units, .. } => units,
        }
    }
}

impl Hash for Command {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Do not derive Hash: enum discriminants may use a platform-sized integer.
        // These explicit tags are part of the cross-platform state-hash encoding.
        match self {
            Self::Move { units, target } => {
                0_u8.hash(state);
                units.hash(state);
                target.hash(state);
            }
            Self::Stop { units } => {
                1_u8.hash(state);
                units.hash(state);
            }
            Self::Attack { units, target } => {
                2_u8.hash(state);
                units.hash(state);
                target.hash(state);
            }
        }
    }
}

/// Local loopback delay. Multiplayer will supply the same ordered tick bundles to every peer.
pub const INPUT_DELAY_TICKS: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::StateHasher;

    #[test]
    fn command_tags_are_fixed_width() {
        for (command, tag) in [
            (
                Command::Move {
                    units: vec![UnitId(3)],
                    target: IVec2::new(7, 9),
                },
                0_u8,
            ),
            (
                Command::Stop {
                    units: vec![UnitId(3)],
                },
                1_u8,
            ),
            (
                Command::Attack {
                    units: vec![UnitId(3)],
                    target: UnitId(4),
                },
                2_u8,
            ),
        ] {
            let mut actual = StateHasher::default();
            command.hash(&mut actual);
            let mut expected = StateHasher::default();
            tag.hash(&mut expected);
            vec![UnitId(3)].hash(&mut expected);
            if tag == 0 {
                IVec2::new(7, 9).hash(&mut expected);
            }
            if tag == 2 {
                UnitId(4).hash(&mut expected);
            }
            assert_eq!(actual.finish(), expected.finish());
        }
    }
}
