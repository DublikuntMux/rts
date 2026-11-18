//! Opponent difficulty. Every level sees and earns exactly the same; levels differ only in how
//! the opponent scouts, how often it reacts, and when it attacks.

use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

/// Where idle scouts go next. Every pattern picks only unexplored cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScoutPattern {
    /// The unexplored cell nearest the scout: a wandering search.
    Nearest,
    /// The unexplored cell nearest the home base: an expanding ring.
    Rings,
    /// Straight to the mirror image of the home base, then outward from there.
    Mirror,
}

impl Difficulty {
    pub const ALL: [Self; 3] = [Self::Easy, Self::Normal, Self::Hard];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Normal => "Normal",
            Self::Hard => "Hard",
        }
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Easy => "Scouts aimlessly, reacts slowly, and attacks late with a large army.",
            Self::Normal => "Scouts outward from its base and attacks once its army is ready.",
            Self::Hard => "Scouts toward your likely base, reacts quickly, and attacks early.",
        }
    }

    /// Ticks between decisions.
    #[must_use]
    pub const fn cadence(self) -> u32 {
        match self {
            Self::Easy => 40,
            Self::Normal => 20,
            Self::Hard => 8,
        }
    }

    #[must_use]
    pub const fn scout_pattern(self) -> ScoutPattern {
        match self {
            Self::Easy => ScoutPattern::Nearest,
            Self::Normal => ScoutPattern::Rings,
            Self::Hard => ScoutPattern::Mirror,
        }
    }

    /// Rifle squads gathered before the first attack.
    #[must_use]
    pub const fn attack_squads(self) -> usize {
        match self {
            Self::Easy => 4,
            Self::Normal => 3,
            Self::Hard => 2,
        }
    }

    /// The earliest tick of the first attack.
    #[must_use]
    pub const fn first_attack_tick(self) -> u32 {
        match self {
            Self::Easy => 20 * 240,
            Self::Normal => 20 * 150,
            Self::Hard => 20 * 90,
        }
    }
}
