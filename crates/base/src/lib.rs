//! Deterministic RTS simulation.
//!
//! Every peer in a match runs this simulation from the same setup and the same commands, so the
//! code here must produce bit-identical results on every platform. No float arithmetic, no
//! hashed collections, no clocks or OS randomness. See `docs/SPEC.md`.
#![deny(clippy::float_arithmetic, clippy::disallowed_types)]

mod coords;
mod hash;
mod map;
mod rules;

use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
};

pub use coords::{TILE, tile_of};
pub use glam::IVec2;
pub use map::{MAX_MAP_SIDE, MapData, PlayerSlot, Terrain, UnitPlacement};
pub use rules::{Rules, TerrainDef, TerrainId, UnitDef, UnitKind};

use crate::hash::StateHasher;

/// Simulation ticks per second.
pub const TICK_RATE: u32 = 20;

/// Largest supported number of players in a match.
pub const MAX_PLAYERS: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Parse(#[from] ron::error::SpannedError),
    #[error("{0}")]
    Invalid(String),
}

impl LoadError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

/// Stable unit identifier. IDs are never reused within a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(pub u32);

/// Index of a player slot in the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u8);

#[derive(Debug, Clone, Hash)]
pub struct Unit {
    pub kind: UnitKind,
    pub owner: PlayerId,
    /// Center in world units.
    pub pos: IVec2,
    pub health: i32,
}

#[derive(Debug, Clone)]
pub struct Player {
    /// Start position in world units.
    pub start: IVec2,
}

/// State of one match.
#[derive(Debug, Clone)]
pub struct Sim {
    rules: Rules,
    terrain: Terrain,
    players: Vec<Player>,
    tick: u32,
    units: BTreeMap<UnitId, Unit>,
    next_unit_id: u32,
}

impl Sim {
    /// Sets up a match on `map`.
    ///
    /// # Errors
    ///
    /// Returns an error if the map does not fit the rules: unknown terrain or unit types, ragged
    /// rows, a bad player count, or starts and units outside the map or on unwalkable terrain.
    pub fn new(rules: Rules, map: &MapData) -> Result<Self, LoadError> {
        let terrain = Terrain::parse(&map.tiles, &rules)?;
        if map.players.is_empty() || map.players.len() > MAX_PLAYERS {
            return Err(LoadError::invalid(format!(
                "map has {} players; expected 1..={MAX_PLAYERS}",
                map.players.len()
            )));
        }
        let mut players = Vec::with_capacity(map.players.len());
        for (index, slot) in map.players.iter().enumerate() {
            if !terrain.is_walkable(slot.start, &rules) {
                return Err(LoadError::invalid(format!(
                    "player {index} starts outside the map or on unwalkable terrain"
                )));
            }
            players.push(Player { start: slot.start });
        }
        let mut sim = Self {
            rules,
            terrain,
            players,
            tick: 0,
            units: BTreeMap::new(),
            next_unit_id: 0,
        };
        for (index, placement) in map.units.iter().enumerate() {
            let kind = sim.rules.unit_kind(&placement.kind).ok_or_else(|| {
                LoadError::invalid(format!(
                    "unit {index} has unknown kind `{}`",
                    placement.kind
                ))
            })?;
            if usize::from(placement.owner) >= sim.players.len() {
                return Err(LoadError::invalid(format!(
                    "unit {index} belongs to missing player {}",
                    placement.owner
                )));
            }
            if !sim.terrain.is_walkable(placement.pos, &sim.rules) {
                return Err(LoadError::invalid(format!(
                    "unit {index} is outside the map or on unwalkable terrain"
                )));
            }
            sim.spawn_unit(kind, PlayerId(placement.owner), placement.pos);
        }
        Ok(sim)
    }

    /// Advances the match by one tick.
    pub const fn step(&mut self) {
        self.tick += 1;
    }

    fn spawn_unit(&mut self, kind: UnitKind, owner: PlayerId, pos: IVec2) -> UnitId {
        let id = UnitId(self.next_unit_id);
        self.next_unit_id += 1;
        let health = self.rules.unit(kind).max_health;
        self.units.insert(
            id,
            Unit {
                kind,
                owner,
                pos,
                health,
            },
        );
        id
    }

    /// Ticks simulated so far.
    #[must_use]
    pub const fn tick(&self) -> u32 {
        self.tick
    }

    #[must_use]
    pub const fn rules(&self) -> &Rules {
        &self.rules
    }

    #[must_use]
    pub const fn terrain(&self) -> &Terrain {
        &self.terrain
    }

    #[must_use]
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Living units in ID order.
    pub fn units(&self) -> impl Iterator<Item = (UnitId, &Unit)> {
        self.units.iter().map(|(&id, unit)| (id, unit))
    }

    #[must_use]
    pub fn unit(&self, id: UnitId) -> Option<&Unit> {
        self.units.get(&id)
    }

    /// Hash of everything that changes during a match. Peers compare it to detect desyncs.
    /// Rules and terrain are fixed at setup and are not included.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::default();
        self.tick.hash(&mut hasher);
        self.next_unit_id.hash(&mut hasher);
        self.units.hash(&mut hasher);
        hasher.finish()
    }
}
