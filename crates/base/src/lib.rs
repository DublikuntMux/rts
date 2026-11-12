//! Deterministic RTS simulation.
//!
//! Every peer in a match runs this simulation from the same setup and the same commands, so the
//! code here must produce bit-identical results on every platform. No float arithmetic, no
//! hashed collections, no clocks or OS randomness. See `docs/architecture/determinism.rst`.
#![deny(clippy::float_arithmetic, clippy::disallowed_types)]

mod combat;
mod command;
mod coords;
mod hash;
mod map;
mod movement;
mod occupancy;
mod path;
mod rules;

use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
};

pub use command::{Command, INPUT_DELAY_TICKS};
pub use coords::{TILE, tile_of};
pub use glam::IVec2;
pub use map::{MAX_MAP_SIDE, MapData, PlayerSlot, Terrain, UnitPlacement};
pub use rules::{
    MAX_DAMAGE_BONUS, RankDef, Rules, SPEED_SCALE, SquadGrid, TerrainDef, TerrainId, UnitDef,
    UnitKind, VisualBounds, Weapon,
};

use crate::hash::StateHasher;
use crate::movement::{MoveGroupId, MoveOrder};
use crate::occupancy::Occupancy;
use serde::{Deserialize, Serialize};

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UnitId(pub u32);

/// Index of a player slot in the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u8);

#[derive(Debug, Clone, Hash)]
pub struct Unit {
    pub kind: UnitKind,
    pub owner: PlayerId,
    /// Center in world units.
    pub pos: IVec2,
    /// Health of each soldier by squad slot; zero marks a fallen soldier. Slots keep their
    /// place in the ranks, so survivors do not shuffle when a neighbor falls.
    pub soldiers: Vec<i32>,
    /// Ticks until the weapon can fire again.
    pub cooldown: u32,
    /// Enemy soldiers this unit has defeated. Determines its rank.
    pub kills: u32,
}

impl Unit {
    /// Combined health of the living soldiers.
    #[must_use]
    pub fn health(&self) -> i64 {
        self.soldiers.iter().map(|&health| i64::from(health)).sum()
    }

    /// Number of living soldiers.
    #[must_use]
    pub fn alive(&self) -> usize {
        self.soldiers.iter().filter(|&&health| health > 0).count()
    }
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
    commands: BTreeMap<u32, Vec<(PlayerId, Command)>>,
    move_orders: BTreeMap<UnitId, MoveOrder>,
    /// Ordered attack targets by attacker.
    attack_orders: BTreeMap<UnitId, UnitId>,
}

/// Derived live group usage and pace; rebuilt after commands, never stored or hashed.
struct MovingGroup {
    speed: i32,
    tiles: BTreeMap<(i32, i32), u32>,
}

impl MovingGroup {
    fn add(&mut self, pos: IVec2, size: u16) {
        let tile = tile_of(pos);
        *self.tiles.entry((tile.x, tile.y)).or_default() += u32::from(size);
    }

    fn remove(&mut self, pos: IVec2, size: u16) {
        let tile = tile_of(pos);
        *self.tiles.get_mut(&(tile.x, tile.y)).unwrap() -= u32::from(size);
    }

    fn used(&self, tile: IVec2) -> u32 {
        self.tiles.get(&(tile.x, tile.y)).copied().unwrap_or(0)
    }
}

impl Sim {
    /// Sets up a match on `map`.
    ///
    /// # Errors
    ///
    /// Returns an error if the rules are invalid or the map does not fit them: unknown terrain
    /// or unit types, ragged rows, a bad player count, starts/units outside traversable terrain,
    /// or over-capacity tiles.
    pub fn new(rules: Rules, map: &MapData) -> Result<Self, LoadError> {
        // Rules can also be deserialized directly, bypassing Rules::from_ron's validation.
        rules.validate()?;
        let terrain = Terrain::parse(&map.tiles, &rules)?;
        if map.players.is_empty() || map.players.len() > MAX_PLAYERS {
            return Err(LoadError::invalid(format!(
                "map has {} players; expected 1..={MAX_PLAYERS}",
                map.players.len()
            )));
        }
        let mut players = Vec::with_capacity(map.players.len());
        for (index, slot) in map.players.iter().enumerate() {
            if terrain.speed_modifier(slot.start, &rules) == 0 {
                return Err(LoadError::invalid(format!(
                    "player {index} starts outside the map or on blocked terrain"
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
            commands: BTreeMap::new(),
            move_orders: BTreeMap::new(),
            attack_orders: BTreeMap::new(),
        };
        let mut occupancy = Occupancy::new(sim.terrain.size());
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
            if sim.terrain.speed_modifier(placement.pos, &sim.rules) == 0 {
                return Err(LoadError::invalid(format!(
                    "unit {index} is outside the map or on blocked terrain"
                )));
            }
            let size = sim.rules.unit(kind).size;
            if !occupancy.fits(tile_of(placement.pos), size, sim.rules.tile_capacity()) {
                return Err(LoadError::invalid(format!(
                    "unit {index} exceeds tile capacity at {:?}",
                    tile_of(placement.pos)
                )));
            }
            occupancy.add(placement.pos, size);
            sim.spawn_unit(kind, PlayerId(placement.owner), placement.pos);
        }
        Ok(sim)
    }

    /// Advances the match by one tick.
    pub fn step(&mut self) {
        self.step_with_commands(std::iter::empty());
    }

    /// Advances one tick, applying agreed commands in their supplied order before pursuit,
    /// movement, and combat.
    /// Unlike [`Self::queue_command`], these commands incur no additional input delay.
    /// Local commands already queued for this tick execute first.
    pub fn step_with_commands(&mut self, commands: impl IntoIterator<Item = (PlayerId, Command)>) {
        self.tick += 1;
        let queued = self.commands.remove(&self.tick).unwrap_or_default();
        for (index, (player, command)) in (0_u64..).zip(queued.into_iter().chain(commands)) {
            let group = (self.tick, index);
            self.apply_command(player, command, group);
        }
        self.pursue_targets();
        self.advance_movement();
        self.advance_combat();
    }

    fn advance_movement(&mut self) {
        if self.move_orders.is_empty() {
            return;
        }
        let mut occupancy = Occupancy::new(self.terrain.size());
        for unit in self.units.values() {
            occupancy.add(unit.pos, self.rules.unit(unit.kind).size);
        }
        let mut groups = BTreeMap::<MoveGroupId, MovingGroup>::new();
        for (id, order) in &self.move_orders {
            if let Some(group) = order.group {
                let unit = &self.units[id];
                let definition = self.rules.unit(unit.kind);
                let group = groups.entry(group).or_insert_with(|| MovingGroup {
                    speed: definition.speed,
                    tiles: BTreeMap::new(),
                });
                group.speed = group.speed.min(definition.speed);
                group.add(unit.pos, definition.size);
            }
        }
        // Stable IDs resolve contention. Each move updates occupancy before the next unit plans.
        self.move_orders.retain(|id, order| {
            let Some(unit) = self.units.get_mut(id) else {
                return false;
            };
            let definition = self.rules.unit(unit.kind);
            occupancy.remove(unit.pos, definition.size);
            let mut group = order.group.and_then(|id| groups.get_mut(&id));
            // Exclude the member itself from both totals to avoid discounting it twice.
            if let Some(group) = &mut group {
                group.remove(unit.pos, definition.size);
            }
            let can_plan = |tile| {
                occupancy.fits_excluding(
                    tile,
                    definition.size,
                    self.rules.tile_capacity(),
                    group.as_ref().map_or(0, |group| group.used(tile)),
                )
            };
            let can_enter =
                |tile| occupancy.fits(tile, definition.size, self.rules.tile_capacity());
            let active = !order.refresh(unit.pos, &self.terrain, &self.rules, &can_plan)
                || order.advance(
                    &mut unit.pos,
                    group.as_ref().map_or(definition.speed, |group| group.speed),
                    &self.terrain,
                    &self.rules,
                    &can_enter,
                );
            occupancy.add(unit.pos, definition.size);
            // A member that arrived is now a stationary obstacle, even to its former group.
            if active && let Some(group) = &mut group {
                group.add(unit.pos, definition.size);
            }
            active
        });
    }

    /// Schedules a local command at the current tick plus [`INPUT_DELAY_TICKS`].
    /// Commands on the same tick retain submission order. Invalid players are ignored.
    pub fn queue_command(&mut self, player: PlayerId, command: Command) {
        if usize::from(player.0) >= self.players.len() {
            return;
        }
        if let Some(tick) = self.tick.checked_add(INPUT_DELAY_TICKS) {
            self.commands
                .entry(tick)
                .or_default()
                .push((player, command));
        }
    }

    fn apply_command(&mut self, player: PlayerId, command: Command, group: MoveGroupId) {
        let (mut units, target) = match command {
            Command::Move { units, target } => {
                if self.terrain.speed_modifier(target, &self.rules) == 0 {
                    return;
                }
                (units, Some(target))
            }
            Command::Stop { units } => (units, None),
            Command::Attack { units, target } => {
                self.order_attack(player, units, target);
                return;
            }
        };
        units.sort_unstable();
        units.dedup();
        let mut orders = Vec::new();
        for id in units {
            let Some(unit) = self.units.get(&id).filter(|unit| unit.owner == player) else {
                continue;
            };
            if let Some(target) = target {
                // An unreachable replacement leaves the current order intact.
                if let Some(order) = MoveOrder::new(unit.pos, target, &self.terrain, &self.rules) {
                    orders.push((id, order));
                    self.attack_orders.remove(&id);
                }
            } else {
                self.move_orders.remove(&id);
                self.attack_orders.remove(&id);
            }
        }
        let group = (orders.len() > 1).then_some(group);
        for (id, mut order) in orders {
            order.group = group;
            self.move_orders.insert(id, order);
        }
    }

    /// Orders the player's armed units to attack another player's unit. Pursuit starts on
    /// this tick and replaces any move order.
    fn order_attack(&mut self, player: PlayerId, mut units: Vec<UnitId>, target: UnitId) {
        if self
            .units
            .get(&target)
            .is_none_or(|target| target.owner == player)
        {
            return;
        }
        units.sort_unstable();
        units.dedup();
        for id in units {
            let armed = self.units.get(&id).is_some_and(|unit| {
                unit.owner == player && self.rules.unit(unit.kind).weapon.is_some()
            });
            if armed {
                self.move_orders.remove(&id);
                self.attack_orders.insert(id, target);
            }
        }
    }

    /// Target of the unit's attack order, if any.
    #[must_use]
    pub fn attack_target(&self, id: UnitId) -> Option<UnitId> {
        self.attack_orders.get(&id).copied()
    }

    /// Destination of the unit's active move order, if any.
    #[must_use]
    pub fn move_target(&self, id: UnitId) -> Option<IVec2> {
        self.move_orders.get(&id).map(MoveOrder::target)
    }

    fn spawn_unit(&mut self, kind: UnitKind, owner: PlayerId, pos: IVec2) -> UnitId {
        let id = UnitId(self.next_unit_id);
        self.next_unit_id += 1;
        let definition = self.rules.unit(kind);
        let soldiers = vec![definition.max_health; usize::from(definition.squad)];
        self.units.insert(
            id,
            Unit {
                kind,
                owner,
                pos,
                soldiers,
                cooldown: 0,
                kills: 0,
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

    /// Removes a unit and its orders. Its ID is never reused, and its occupied capacity is
    /// available to pathfinding on the next tick. Orders that target it end on the next tick.
    /// Simulation actions must call this identically on every peer.
    pub fn remove_unit(&mut self, id: UnitId) -> Option<Unit> {
        self.move_orders.remove(&id);
        self.attack_orders.remove(&id);
        self.units.remove(&id)
    }

    /// Hash of everything that changes during a match. Peers compare it to detect desyncs.
    /// Rules and terrain are fixed at setup and are not included.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::default();
        self.tick.hash(&mut hasher);
        self.next_unit_id.hash(&mut hasher);
        self.units.hash(&mut hasher);
        self.commands.hash(&mut hasher);
        self.move_orders.hash(&mut hasher);
        self.attack_orders.hash(&mut hasher);
        hasher.finish()
    }
}
