//! Fog of war: what one player knows about a match.
//!
//! The authority keeps a [`PlayerKnowledge`] per player. Every tick it observes the
//! authoritative [`Sim`] and produces a [`ViewFrame`]: the newly explored terrain and the changes
//! to everything that player can currently see. A [`PlayerView`] applies frames and offers a
//! read API shaped like [`Sim`]'s, so the client and the AI read the same filtered knowledge.
//!
//! Unit and deposit IDs in a view are per-player aliases, assigned in the order the player first
//! observes them, so they reveal neither authoritative IDs nor how many units were created out of
//! sight. [`PlayerKnowledge::translate_command`] maps view IDs back and validates targets.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    Command, Deposit, DepositId, IVec2, LoadError, MAX_MAP_SIDE, MAX_PLAYERS, MatchOutcome,
    PlayerId, ResourceKind, Resources, Rules, Sim, TILE, TerrainId, Training, Unit, UnitId, Volley,
    tile_of,
};

/// How much of a tile a player knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileVisibility {
    /// Never seen: terrain and everything on it are unknown.
    #[default]
    Unexplored,
    /// Seen before: terrain is known, current contents are not.
    Remembered,
    /// In sight of one of the player's units now.
    Visible,
}

/// Explored and currently visible tiles of one player.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FogVisibility {
    size: IVec2,
    explored: Vec<bool>,
    visible: Vec<bool>,
}

impl FogVisibility {
    /// An unexplored grid of `size` tiles.
    ///
    /// # Panics
    /// Panics if a side is outside `1..=MAX_MAP_SIDE`.
    #[must_use]
    pub fn new(size: IVec2) -> Self {
        assert!(size.min_element() > 0 && size.max_element() <= MAX_MAP_SIDE);
        let len = usize::try_from(size.x * size.y).expect("bounded map size");
        Self {
            size,
            explored: vec![false; len],
            visible: vec![false; len],
        }
    }

    /// Size in tiles.
    #[must_use]
    pub const fn size(&self) -> IVec2 {
        self.size
    }

    /// Replaces the current sight with the discs around `sources` (world position, sight radius
    /// in tiles) and returns the tile indices explored for the first time, in ascending order.
    ///
    /// # Panics
    /// Never: map sizes are bounded by [`MAX_MAP_SIDE`].
    pub fn update(&mut self, sources: impl IntoIterator<Item = (IVec2, u8)>) -> Vec<u32> {
        self.visible.fill(false);
        for (pos, radius) in sources {
            let center = tile_of(pos);
            let r = i32::from(radius);
            for y in (center.y - r).max(0)..=(center.y + r).min(self.size.y - 1) {
                for x in (center.x - r).max(0)..=(center.x + r).min(self.size.x - 1) {
                    if tile_in_sight(center, IVec2::new(x, y), r) {
                        let index = usize::try_from(y * self.size.x + x).expect("inside map");
                        self.visible[index] = true;
                    }
                }
            }
        }
        let mut discovered = Vec::new();
        for (index, (&visible, explored)) in self.visible.iter().zip(&mut self.explored).enumerate()
        {
            if visible && !*explored {
                *explored = true;
                discovered.push(u32::try_from(index).expect("bounded map size"));
            }
        }
        discovered
    }

    /// Visibility of a tile; tiles outside the map are unexplored.
    #[must_use]
    pub fn tile(&self, tile: IVec2) -> TileVisibility {
        self.index(tile)
            .map_or(TileVisibility::Unexplored, |index| {
                if self.visible[index] {
                    TileVisibility::Visible
                } else if self.explored[index] {
                    TileVisibility::Remembered
                } else {
                    TileVisibility::Unexplored
                }
            })
    }

    /// Whether a world position lies on a currently visible tile.
    #[must_use]
    pub fn sees(&self, pos: IVec2) -> bool {
        self.tile(tile_of(pos)) == TileVisibility::Visible
    }

    fn index(&self, tile: IVec2) -> Option<usize> {
        (tile.cmpge(IVec2::ZERO).all() && tile.cmplt(self.size).all())
            .then(|| usize::try_from(tile.y * self.size.x + tile.x).expect("inside map"))
    }
}

/// Sight from a source tile. The square bound and r² + r give rounded tile discs.
pub(crate) fn tile_in_sight(center: IVec2, tile: IVec2, radius: i32) -> bool {
    let delta = (tile - center).abs();
    delta.max_element() <= radius
        && delta.x * delta.x + delta.y * delta.y <= radius * radius + radius
}

fn sight_sources<'a>(
    rules: &'a Rules,
    units: impl Iterator<Item = &'a Unit> + 'a,
    player: PlayerId,
) -> impl Iterator<Item = (IVec2, u8)> + 'a {
    units
        .filter(move |unit| unit.owner == player)
        .map(|unit| (unit.pos, rules.unit(unit.kind).sight))
}

/// Public facts about a match from one player's perspective, known before the first frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewIntro {
    pub player: PlayerId,
    pub players: u8,
    /// Map size in tiles.
    pub size: IVec2,
    /// The player's own start, in world units.
    pub start: IVec2,
    pub skirmish: bool,
}

/// One tick of a player's knowledge, relative to the previous frame.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewFrame {
    pub tick: u32,
    /// Tiles explored for the first time, as `(row-major index, terrain)`.
    pub explored: Vec<(u32, TerrainId)>,
    /// Units that came into sight or changed.
    pub units: Vec<ViewUnit>,
    /// Units that left sight or were destroyed.
    pub removed: Vec<UnitId>,
    /// Deposits seen for the first time or seen with a different stock.
    pub deposits: Vec<ViewDeposit>,
    /// The player's stockpile, when it changed.
    pub resources: Option<Resources>,
    /// The player's headquarters, when it changed.
    pub headquarters: Option<UnitId>,
    /// The match result, once known.
    pub outcome: Option<MatchOutcome>,
    /// Volleys of this tick fired between visible positions.
    pub volleys: Vec<Volley>,
}

impl ViewFrame {
    /// Whether the frame changes nothing but the tick.
    #[must_use]
    pub const fn is_quiet(&self) -> bool {
        self.explored.is_empty()
            && self.units.is_empty()
            && self.removed.is_empty()
            && self.deposits.is_empty()
            && self.resources.is_none()
            && self.headquarters.is_none()
            && self.outcome.is_none()
            && self.volleys.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewUnit {
    pub id: UnitId,
    /// Enemy units show their rank threshold instead of exact kills, and no cooldown.
    pub unit: Unit,
    /// Orders and work, only for the player's own units.
    pub own: Option<OwnUnitState>,
}

/// Orders and work of one of the player's own units.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnUnitState {
    pub move_target: Option<IVec2>,
    pub attack_target: Option<UnitId>,
    pub gather_target: Option<DepositId>,
    pub build_target: Option<IVec2>,
    pub construction_remaining: Option<u32>,
    pub cargo: Option<(ResourceKind, u32)>,
    pub production: Option<Vec<Training>>,
}

/// A deposit as last seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewDeposit {
    pub id: DepositId,
    pub pos: IVec2,
    pub kind: ResourceKind,
    pub remaining: u32,
}

impl ViewDeposit {
    #[must_use]
    pub const fn deposit(&self) -> Deposit {
        Deposit {
            kind: self.kind,
            pos: self.pos,
            remaining: self.remaining,
        }
    }
}

#[derive(Debug, Clone)]
struct Aliases<Id> {
    by_authority: BTreeMap<Id, u32>,
    by_view: Vec<Id>,
}

impl<Id: Copy + Ord> Aliases<Id> {
    const fn new() -> Self {
        Self {
            by_authority: BTreeMap::new(),
            by_view: Vec::new(),
        }
    }

    fn observe(&mut self, id: Id) -> u32 {
        *self.by_authority.entry(id).or_insert_with(|| {
            let alias = u32::try_from(self.by_view.len()).expect("fewer than 2^32 aliases");
            self.by_view.push(id);
            alias
        })
    }

    fn get(&self, id: Id) -> Option<u32> {
        self.by_authority.get(&id).copied()
    }

    fn resolve(&self, alias: u32) -> Option<Id> {
        self.by_view.get(usize::try_from(alias).ok()?).copied()
    }
}

/// The authority's record of what one player knows, and the source of that player's frames.
#[derive(Debug, Clone)]
pub struct PlayerKnowledge {
    player: PlayerId,
    fog: FogVisibility,
    units: Aliases<UnitId>,
    deposits: Aliases<DepositId>,
    sent_units: BTreeMap<UnitId, ViewUnit>,
    sent_deposits: BTreeMap<DepositId, ViewDeposit>,
    sent_resources: Option<Resources>,
    sent_headquarters: Option<UnitId>,
    sent_outcome: Option<MatchOutcome>,
}

impl PlayerKnowledge {
    /// Knowledge of `player`, who has seen nothing yet.
    ///
    /// # Panics
    /// Panics if the player is not in this match.
    #[must_use]
    pub fn new(sim: &Sim, player: PlayerId) -> Self {
        assert!(usize::from(player.0) < sim.players().len());
        Self {
            player,
            fog: FogVisibility::new(sim.terrain().size()),
            units: Aliases::new(),
            deposits: Aliases::new(),
            sent_units: BTreeMap::new(),
            sent_deposits: BTreeMap::new(),
            sent_resources: None,
            sent_headquarters: None,
            sent_outcome: None,
        }
    }

    #[must_use]
    pub const fn player(&self) -> PlayerId {
        self.player
    }

    /// Visibility as of the latest observation.
    #[must_use]
    pub const fn visibility(&self) -> &FogVisibility {
        &self.fog
    }

    /// The facts the player may know before the first frame.
    ///
    /// # Panics
    /// Panics if the match has more than 255 players, which [`Sim::new`] rejects.
    #[must_use]
    pub fn intro(&self, sim: &Sim) -> ViewIntro {
        ViewIntro {
            player: self.player,
            players: u8::try_from(sim.players().len()).expect("bounded player count"),
            size: sim.terrain().size(),
            start: sim.players()[usize::from(self.player.0)].start,
            skirmish: sim.is_skirmish(),
        }
    }

    /// The player's alias for an authoritative unit, if the player has ever seen it.
    #[must_use]
    pub fn unit_alias(&self, id: UnitId) -> Option<UnitId> {
        self.units.get(id).map(UnitId)
    }

    /// Observes the simulation's current tick and returns what changed for this player since the
    /// previous observation. The first observation describes everything the player knows.
    ///
    /// # Panics
    /// Panics if `sim` is not the match this knowledge was created for.
    pub fn observe(&mut self, sim: &Sim) -> ViewFrame {
        assert_eq!(sim.terrain().size(), self.fog.size(), "same match");
        let explored = self
            .fog
            .update(sight_sources(
                sim.rules(),
                sim.units().map(|(_, u)| u),
                self.player,
            ))
            .into_iter()
            .map(|index| (index, sim.terrain().tiles()[index as usize]))
            .collect();

        let mut current = BTreeMap::new();
        for (id, unit) in sim.units() {
            if unit.owner == self.player || self.fog.sees(unit.pos) {
                let alias = UnitId(self.units.observe(id));
                current.insert(alias, (id, unit));
            }
        }
        let current: BTreeMap<UnitId, ViewUnit> = current
            .into_iter()
            .map(|(alias, (id, unit))| (alias, self.project_unit(sim, alias, id, unit)))
            .collect();
        let removed = self
            .sent_units
            .keys()
            .filter(|id| !current.contains_key(id))
            .copied()
            .collect();
        let units = current
            .values()
            .filter(|unit| self.sent_units.get(&unit.id) != Some(unit))
            .cloned()
            .collect();
        self.sent_units = current;

        let mut deposits = Vec::new();
        for (id, deposit) in sim.deposits() {
            if self.fog.sees(deposit.pos) {
                let id = DepositId(self.deposits.observe(id));
                let seen = ViewDeposit {
                    id,
                    pos: deposit.pos,
                    kind: deposit.kind,
                    remaining: deposit.remaining,
                };
                if self.sent_deposits.insert(id, seen) != Some(seen) {
                    deposits.push(seen);
                }
            }
        }

        let volleys = sim
            .volleys()
            .iter()
            .filter(|volley| self.fog.sees(volley.from) && self.fog.sees(volley.to))
            .map(|volley| Volley {
                shooter: UnitId(self.units.observe(volley.shooter)),
                target: UnitId(self.units.observe(volley.target)),
                ..volley.clone()
            })
            .collect();

        let resources = sim.resources(self.player);
        let headquarters = sim
            .headquarters(self.player)
            .and_then(|id| self.unit_alias(id));
        let outcome = sim.outcome();
        ViewFrame {
            tick: sim.tick(),
            explored,
            units,
            removed,
            deposits,
            resources: changed(&mut self.sent_resources, resources),
            headquarters: changed(&mut self.sent_headquarters, headquarters),
            outcome: changed(&mut self.sent_outcome, outcome),
            volleys,
        }
    }

    fn project_unit(&self, sim: &Sim, alias: UnitId, id: UnitId, unit: &Unit) -> ViewUnit {
        let mut unit = unit.clone();
        let own = if unit.owner == self.player {
            let attack = sim.attack_target(id);
            let attack_target = attack.and_then(|target| {
                let target_unit = sim.unit(target)?;
                self.fog.sees(target_unit.pos).then_some(())?;
                self.unit_alias(target)
            });
            Some(OwnUnitState {
                // Pursuit of a hidden target would reveal where it is.
                move_target: if attack.is_none() || attack_target.is_some() {
                    sim.move_target(id)
                } else {
                    None
                },
                attack_target,
                gather_target: sim
                    .gather_target(id)
                    .and_then(|deposit| self.deposits.get(deposit))
                    .map(DepositId),
                build_target: sim.build_target(id),
                construction_remaining: sim.construction_remaining(id),
                cargo: sim.cargo(id),
                production: sim.production(id).map(<[_]>::to_vec),
            })
        } else {
            unit.cooldown = 0;
            unit.kills = sim
                .rules()
                .ranks()
                .iter()
                .rev()
                .find(|rank| rank.kills <= unit.kills)
                .map_or(0, |rank| rank.kills);
            None
        };
        ViewUnit {
            id: alias,
            unit,
            own,
        }
    }

    /// Maps a command written with this player's view IDs to authoritative IDs.
    ///
    /// Units the player does not own are dropped. Attack targets, gather deposits, and build
    /// sites must be in sight now, so the answer never depends on hidden or remembered state.
    /// Returns `None` if nothing valid remains.
    #[must_use]
    pub fn translate_command(&self, sim: &Sim, command: &Command) -> Option<Command> {
        let mut command = command.clone();
        match &mut command {
            Command::Attack { target, .. } => {
                let id = self.units.resolve(target.0)?;
                let unit = sim.unit(id)?;
                if unit.owner == self.player || !self.fog.sees(unit.pos) {
                    return None;
                }
                *target = id;
            }
            Command::Gather { deposit, .. } => {
                let id = self.deposits.resolve(deposit.0)?;
                if !self.fog.sees(sim.deposit(id)?.pos) {
                    return None;
                }
                *deposit = id;
            }
            Command::BuildBarracks { target, .. } if !self.fog.sees(*target) => return None,
            _ => {}
        }
        let units = command.units_mut();
        units.retain_mut(|unit| {
            let Some(id) = self.units.resolve(unit.0) else {
                return false;
            };
            if sim.unit(id).is_none_or(|unit| unit.owner != self.player) {
                return false;
            }
            *unit = id;
            true
        });
        (!units.is_empty()).then_some(command)
    }
}

impl PlayerKnowledge {
    /// Re-checks a command already bound by [`Self::translate_command`] when it executes.
    ///
    /// Commands wait for the input delay, and a target in sight at submission may have left
    /// sight since. Attack targets must still be living enemies in sight, gather deposits and
    /// build sites must still be in sight, and units must still belong to the player. Returns
    /// `None` if nothing valid remains.
    #[must_use]
    pub fn revalidate(&self, sim: &Sim, mut command: Command) -> Option<Command> {
        match &command {
            Command::Attack { target, .. } => {
                let unit = sim.unit(*target)?;
                if unit.owner == self.player || !self.fog.sees(unit.pos) {
                    return None;
                }
            }
            Command::Gather { deposit, .. } => {
                self.deposits.get(*deposit)?;
                if !self.fog.sees(sim.deposit(*deposit)?.pos) {
                    return None;
                }
            }
            Command::BuildBarracks { target, .. } if !self.fog.sees(*target) => return None,
            _ => {}
        }
        let units = command.units_mut();
        units.retain(|&id| sim.unit(id).is_some_and(|unit| unit.owner == self.player));
        (!units.is_empty()).then_some(command)
    }
}

fn changed<T: Copy + PartialEq>(sent: &mut Option<T>, current: Option<T>) -> Option<T> {
    if *sent == current {
        return None;
    }
    *sent = current;
    current
}

/// One player's knowledge of a match, rebuilt from [`ViewFrame`]s.
///
/// Read methods mirror [`Sim`]'s, restricted to what the player knows. Frames from a remote
/// server are untrusted; [`Self::apply`] validates them before changing anything.
#[derive(Debug, Clone)]
pub struct PlayerView {
    rules: Rules,
    intro: ViewIntro,
    tick: Option<u32>,
    terrain: Vec<Option<TerrainId>>,
    fog: FogVisibility,
    units: BTreeMap<UnitId, ViewUnit>,
    deposits: BTreeMap<DepositId, ViewDeposit>,
    resources: Option<Resources>,
    headquarters: Option<UnitId>,
    outcome: Option<MatchOutcome>,
    volleys: Vec<Volley>,
}

impl PlayerView {
    /// An empty view awaiting its first frame.
    ///
    /// # Errors
    /// Returns an error if the intro is inconsistent: bad size, player, or start.
    ///
    /// # Panics
    /// Never: the size is checked first.
    pub fn new(rules: Rules, intro: ViewIntro) -> Result<Self, LoadError> {
        if intro.size.min_element() < 1 || intro.size.max_element() > MAX_MAP_SIDE {
            return Err(LoadError::invalid("view map size is out of range"));
        }
        if intro.players == 0
            || usize::from(intro.players) > MAX_PLAYERS
            || intro.player.0 >= intro.players
            || (intro.skirmish && intro.players != 2)
        {
            return Err(LoadError::invalid("view players are out of range"));
        }
        if !inside(intro.size, intro.start) {
            return Err(LoadError::invalid("view start is outside the map"));
        }
        if intro.skirmish && rules.economy().is_none() {
            return Err(LoadError::invalid("skirmish views need economy rules"));
        }
        let len = usize::try_from(intro.size.x * intro.size.y).expect("bounded map size");
        Ok(Self {
            rules,
            intro,
            tick: None,
            terrain: vec![None; len],
            fog: FogVisibility::new(intro.size),
            units: BTreeMap::new(),
            deposits: BTreeMap::new(),
            resources: None,
            headquarters: None,
            outcome: None,
            volleys: Vec::new(),
        })
    }

    /// The full current knowledge of `player` in `sim`, starting from nothing. Convenient for
    /// tests and tools; live code keeps a [`PlayerKnowledge`] and applies every frame.
    ///
    /// # Panics
    /// Panics if the player is not in this match.
    #[must_use]
    pub fn observe(sim: &Sim, player: PlayerId) -> (PlayerKnowledge, Self) {
        let mut knowledge = PlayerKnowledge::new(sim, player);
        let mut view =
            Self::new(sim.rules().clone(), knowledge.intro(sim)).expect("valid simulation");
        view.apply(knowledge.observe(sim))
            .expect("frames from the authority");
        (knowledge, view)
    }

    /// Applies the next frame. The first frame may have any tick; later ones must follow it.
    ///
    /// # Errors
    /// Returns an error, leaving the view unchanged, if the frame is out of sequence or refers
    /// to data the view cannot hold.
    pub fn apply(&mut self, frame: ViewFrame) -> Result<(), LoadError> {
        self.validate(&frame)?;
        self.tick = Some(frame.tick);
        for (index, terrain) in frame.explored {
            self.terrain[index as usize] = Some(terrain);
        }
        for id in frame.removed {
            self.units.remove(&id);
        }
        for unit in frame.units {
            self.units.insert(unit.id, unit);
        }
        for deposit in frame.deposits {
            self.deposits.insert(deposit.id, deposit);
        }
        if frame.resources.is_some() {
            self.resources = frame.resources;
        }
        if frame.headquarters.is_some() {
            self.headquarters = frame.headquarters;
        }
        if frame.outcome.is_some() {
            self.outcome = frame.outcome;
        }
        self.volleys = frame.volleys;
        let player = self.intro.player;
        self.fog.update(sight_sources(
            &self.rules,
            self.units.values().map(|unit| &unit.unit),
            player,
        ));
        Ok(())
    }

    fn validate(&self, frame: &ViewFrame) -> Result<(), LoadError> {
        let invalid = |message: &str| Err(LoadError::invalid(format!("view frame: {message}")));
        if let Some(tick) = self.tick
            && Some(frame.tick) != tick.checked_add(1)
        {
            return invalid("tick out of sequence");
        }
        if frame.explored.iter().any(|&(index, terrain)| {
            self.terrain.get(index as usize).is_none_or(Option::is_some)
                || usize::from(terrain.0) >= self.rules.terrain_types().len()
        }) {
            return invalid("bad explored tile");
        }
        let mut seen = BTreeSet::new();
        for id in &frame.removed {
            if !self.units.contains_key(id) || !seen.insert(*id) {
                return invalid("removed an unknown unit");
            }
        }
        let mut updated = BTreeSet::new();
        for view in &frame.units {
            let unit = &view.unit;
            let own = unit.owner == self.intro.player;
            let valid = updated.insert(view.id)
                && unit.owner.0 < self.intro.players
                && inside(self.intro.size, unit.pos)
                && own == view.own.is_some()
                && self.rules.get_unit(unit.kind).is_some_and(|def| {
                    unit.soldiers.len() == usize::from(def.squad)
                        && unit
                            .soldiers
                            .iter()
                            .all(|&health| (0..=def.max_health).contains(&health))
                });
            if !valid {
                return invalid("bad unit");
            }
        }
        for deposit in &frame.deposits {
            if !self.intro.skirmish || !inside(self.intro.size, deposit.pos) {
                return invalid("bad deposit");
            }
        }
        if frame.resources.is_some() && !self.intro.skirmish {
            return invalid("resources outside a skirmish");
        }
        if let Some(id) = frame.headquarters {
            let known =
                frame.units.iter().any(|unit| unit.id == id) || self.units.contains_key(&id);
            if !self.intro.skirmish || !known {
                return invalid("bad headquarters");
            }
        }
        if let Some(MatchOutcome::Winner(player)) = frame.outcome
            && player.0 >= self.intro.players
        {
            return invalid("bad outcome");
        }
        for volley in &frame.volleys {
            if !inside(self.intro.size, volley.from)
                || !inside(self.intro.size, volley.to)
                || self.rules.get_unit(volley.shooter_kind).is_none()
                || self.rules.get_unit(volley.target_kind).is_none()
            {
                return invalid("bad volley");
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn rules(&self) -> &Rules {
        &self.rules
    }

    #[must_use]
    pub const fn intro(&self) -> &ViewIntro {
        &self.intro
    }

    /// The player whose knowledge this is.
    #[must_use]
    pub const fn player(&self) -> PlayerId {
        self.intro.player
    }

    /// Number of players in the match.
    #[must_use]
    pub const fn players(&self) -> u8 {
        self.intro.players
    }

    /// The player's own start, in world units.
    #[must_use]
    pub const fn start(&self) -> IVec2 {
        self.intro.start
    }

    /// Tick of the latest frame, or zero before the first.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick.unwrap_or(0)
    }

    #[must_use]
    pub const fn is_skirmish(&self) -> bool {
        self.intro.skirmish
    }

    /// Map size in tiles.
    #[must_use]
    pub const fn size(&self) -> IVec2 {
        self.intro.size
    }

    /// Map size in world units.
    #[must_use]
    pub fn world_size(&self) -> IVec2 {
        self.intro.size * TILE
    }

    /// Known terrain row by row, top row first; `None` is unexplored.
    #[must_use]
    pub fn terrain_tiles(&self) -> &[Option<TerrainId>] {
        &self.terrain
    }

    /// Known terrain at a tile; `None` if unexplored or outside the map.
    #[must_use]
    pub fn terrain(&self, tile: IVec2) -> Option<TerrainId> {
        inside_tiles(self.intro.size, tile)
            .then(|| self.terrain[usize::try_from(tile.y * self.intro.size.x + tile.x).ok()?])
            .flatten()
    }

    #[must_use]
    pub fn visibility(&self, tile: IVec2) -> TileVisibility {
        self.fog.tile(tile)
    }

    #[must_use]
    pub const fn fog(&self) -> &FogVisibility {
        &self.fog
    }

    /// Speed multiplier of known terrain at a world position; `None` if unexplored or outside.
    #[must_use]
    pub fn speed_modifier(&self, pos: IVec2) -> Option<i32> {
        self.terrain(tile_of(pos))
            .map(|terrain| self.rules.terrain(terrain).speed_modifier)
    }

    /// Own units and enemies in sight, in view ID order.
    pub fn units(&self) -> impl Iterator<Item = (UnitId, &Unit)> {
        self.units.iter().map(|(&id, unit)| (id, &unit.unit))
    }

    #[must_use]
    pub fn unit(&self, id: UnitId) -> Option<&Unit> {
        self.units.get(&id).map(|unit| &unit.unit)
    }

    /// Orders and work of one of the player's own units.
    #[must_use]
    pub fn own(&self, id: UnitId) -> Option<&OwnUnitState> {
        self.units.get(&id)?.own.as_ref()
    }

    #[must_use]
    pub fn move_target(&self, id: UnitId) -> Option<IVec2> {
        self.own(id)?.move_target
    }

    #[must_use]
    pub fn attack_target(&self, id: UnitId) -> Option<UnitId> {
        self.own(id)?.attack_target
    }

    #[must_use]
    pub fn gather_target(&self, id: UnitId) -> Option<DepositId> {
        self.own(id)?.gather_target
    }

    #[must_use]
    pub fn build_target(&self, id: UnitId) -> Option<IVec2> {
        self.own(id)?.build_target
    }

    #[must_use]
    pub fn construction_remaining(&self, id: UnitId) -> Option<u32> {
        self.own(id)?.construction_remaining
    }

    #[must_use]
    pub fn cargo(&self, id: UnitId) -> Option<(ResourceKind, u32)> {
        self.own(id)?.cargo
    }

    #[must_use]
    pub fn production(&self, id: UnitId) -> Option<&[Training]> {
        self.own(id)?.production.as_deref()
    }

    /// Deposits seen so far, with the stock last seen, in view ID order.
    pub fn deposits(&self) -> impl Iterator<Item = (DepositId, Deposit)> {
        self.deposits
            .iter()
            .map(|(&id, deposit)| (id, deposit.deposit()))
    }

    #[must_use]
    pub fn deposit(&self, id: DepositId) -> Option<Deposit> {
        self.deposits.get(&id).map(ViewDeposit::deposit)
    }

    /// The player's own stockpile in a skirmish.
    #[must_use]
    pub const fn resources(&self) -> Option<Resources> {
        self.resources
    }

    /// The player's own living headquarters.
    #[must_use]
    pub fn headquarters(&self) -> Option<UnitId> {
        self.headquarters.filter(|id| self.units.contains_key(id))
    }

    #[must_use]
    pub const fn outcome(&self) -> Option<MatchOutcome> {
        self.outcome
    }

    /// Visible volleys of the latest tick.
    #[must_use]
    pub fn volleys(&self) -> &[Volley] {
        &self.volleys
    }

    /// Whether a barracks site in sight looks free. Sites out of sight are refused, as the
    /// authority refuses them. The authority decides.
    #[must_use]
    pub fn can_place_barracks(&self, target: IVec2) -> bool {
        if self.resources.is_none()
            || self.outcome.is_some()
            || !self.fog.sees(target)
            || self.speed_modifier(target).is_none_or(|speed| speed == 0)
        {
            return false;
        }
        let tile = tile_of(target);
        !self.units().any(|(_, unit)| tile_of(unit.pos) == tile)
            && !self
                .deposits
                .values()
                .any(|deposit| tile_of(deposit.pos) == tile)
            && !self.units.values().any(|unit| {
                unit.own
                    .as_ref()
                    .and_then(|own| own.build_target)
                    .is_some_and(|site| tile_of(site) == tile)
            })
    }
}

fn inside_tiles(size: IVec2, tile: IVec2) -> bool {
    tile.cmpge(IVec2::ZERO).all() && tile.cmplt(size).all()
}

fn inside(size: IVec2, pos: IVec2) -> bool {
    pos.cmpge(IVec2::ZERO).all() && pos.cmplt(size * TILE).all()
}

#[cfg(test)]
mod tests;
