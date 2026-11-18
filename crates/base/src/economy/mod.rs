//! Deterministic skirmish economy and production.

mod workers;

use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
};

use serde::{Deserialize, Serialize};
use workers::Work;

use crate::{
    IVec2, LoadError, PlayerId, Sim, SkirmishMap, TILE, UnitId, UnitKind, coords::de_tile_pos,
    tile_of,
};

/// Maximum number of paid training jobs per producer, including its current job.
pub const PRODUCTION_QUEUE_LIMIT: usize = 5;
const NEIGHBORS: [IVec2; 4] = [IVec2::NEG_Y, IVec2::NEG_X, IVec2::X, IVec2::Y];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceKind {
    Wood,
    Gold,
}

impl Hash for ResourceKind {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self as u8).hash(state);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    pub wood: u32,
    pub gold: u32,
}

impl Resources {
    #[must_use]
    pub const fn get(self, kind: ResourceKind) -> u32 {
        match kind {
            ResourceKind::Wood => self.wood,
            ResourceKind::Gold => self.gold,
        }
    }

    #[must_use]
    pub const fn can_afford(self, cost: Self) -> bool {
        self.wood >= cost.wood && self.gold >= cost.gold
    }

    const fn count_mut(&mut self, kind: ResourceKind) -> &mut u32 {
        match kind {
            ResourceKind::Wood => &mut self.wood,
            ResourceKind::Gold => &mut self.gold,
        }
    }

    const fn spend(&mut self, cost: Self) {
        self.wood -= cost.wood;
        self.gold -= cost.gold;
    }

    const fn refund(&mut self, cost: Self) {
        self.wood += cost.wood;
        self.gold += cost.gold;
    }

    const fn has_both(self) -> bool {
        self.wood > 0 && self.gold > 0
    }
}

/// Price and duration of scouts trained at a barracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoutTrainingRules {
    pub cost: Resources,
    pub ticks: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomyRules {
    pub initial: Resources,
    pub worker_cost: Resources,
    pub barracks_cost: Resources,
    pub rifle_cost: Resources,
    pub worker_ticks: u32,
    pub barracks_ticks: u32,
    pub rifle_ticks: u32,
    /// Absent in rules written before scouts, which then cannot be trained.
    #[serde(default)]
    pub scout: Option<ScoutTrainingRules>,
    pub harvest_ticks: u32,
    pub carry_capacity: u32,
}

impl EconomyRules {
    pub(super) fn validate(self) -> Result<(), LoadError> {
        if !self.worker_cost.has_both()
            || !self.barracks_cost.has_both()
            || !self.rifle_cost.has_both()
        {
            return Err(LoadError::invalid(
                "each skirmish price needs wood and gold",
            ));
        }
        if [
            self.worker_ticks,
            self.barracks_ticks,
            self.rifle_ticks,
            self.harvest_ticks,
            self.carry_capacity,
        ]
        .contains(&0)
        {
            return Err(LoadError::invalid(
                "skirmish durations and carry capacity must be positive",
            ));
        }
        if self
            .scout
            .is_some_and(|scout| !scout.cost.has_both() || scout.ticks == 0)
        {
            return Err(LoadError::invalid(
                "scout training needs wood, gold, and a positive duration",
            ));
        }
        Ok(())
    }
}

/// Stable index of a resource deposit in the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DepositId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deposit {
    pub kind: ResourceKind,
    #[serde(deserialize_with = "de_tile_pos")]
    pub pos: IVec2,
    pub remaining: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingKind {
    Worker,
    RifleSquad,
    Scout,
}

impl Hash for TrainingKind {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self as u8).hash(state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Training {
    pub kind: TrainingKind,
    /// Ticks until completion, or zero while waiting for a free spawn tile.
    pub remaining: u32,
}

#[derive(Debug, Clone, Hash)]
pub struct Economy {
    banks: Vec<Resources>,
    deposits: Vec<Deposit>,
    work: BTreeMap<UnitId, Work>,
    cargo: BTreeMap<UnitId, (ResourceKind, u32)>,
    production: BTreeMap<UnitId, Vec<Training>>,
    worker: UnitKind,
    rifle: UnitKind,
    barracks: UnitKind,
}

impl Economy {
    pub(super) fn new(sim: &Sim, map: &SkirmishMap) -> Result<Self, LoadError> {
        let economy = sim
            .rules
            .economy()
            .ok_or_else(|| LoadError::invalid("skirmish maps need economy rules"))?;
        let get_kind = |name, building| {
            sim.rules
                .unit_kind(name)
                .filter(|&kind| {
                    let unit = sim.rules.unit(kind);
                    unit.building == building
                        && (!building || unit.size == sim.rules.tile_capacity())
                })
                .ok_or_else(|| {
                    LoadError::invalid(format!("missing or invalid skirmish kind `{name}`"))
                })
        };
        let mut stock = economy.initial;
        let mut tiles = BTreeSet::new();
        for deposit in &map.deposits {
            let tile = tile_of(deposit.pos);
            if deposit.remaining == 0
                || sim.terrain.speed_modifier(deposit.pos, &sim.rules) == 0
                || !tiles.insert((tile.x, tile.y))
                || sim.units().any(|(_, unit)| {
                    tile_of(unit.pos) == tile && sim.rules.unit(unit.kind).building
                })
            {
                return Err(LoadError::invalid(
                    "deposits must be nonempty and occupy distinct traversable tiles without buildings",
                ));
            }
            let count = stock.count_mut(deposit.kind);
            *count = count.checked_add(deposit.remaining).ok_or_else(|| {
                LoadError::invalid("combined skirmish resource stock is too large")
            })?;
        }
        if stock.wood == economy.initial.wood || stock.gold == economy.initial.gold {
            return Err(LoadError::invalid(
                "skirmish maps need wood and gold deposits",
            ));
        }
        let rifle = get_kind("soldier", false)?;
        if sim.rules.unit(rifle).weapon.is_none() {
            return Err(LoadError::invalid("skirmish soldiers need a weapon"));
        }
        if economy.scout.is_some() && sim.rules.unit(get_kind("scout", false)?).weapon.is_none() {
            return Err(LoadError::invalid("skirmish scouts need a weapon"));
        }
        Ok(Self {
            banks: vec![economy.initial; sim.players.len()],
            deposits: map.deposits.clone(),
            work: BTreeMap::new(),
            cargo: BTreeMap::new(),
            production: BTreeMap::new(),
            worker: get_kind("worker", false)?,
            rifle,
            barracks: get_kind("barracks", true)?,
        })
    }

    fn deposit(&self, id: DepositId) -> Option<&Deposit> {
        self.deposits.get(usize::try_from(id.0).ok()?)
    }
}

impl Sim {
    fn economy(&self) -> Option<&Economy> {
        self.skirmish.as_ref().map(|s| &s.economy)
    }

    fn economy_mut(&mut self) -> Option<&mut Economy> {
        self.skirmish.as_mut().map(|s| &mut s.economy)
    }

    /// Current stockpile for a player in a skirmish.
    #[must_use]
    pub fn resources(&self, player: PlayerId) -> Option<Resources> {
        self.economy()?.banks.get(usize::from(player.0)).copied()
    }

    /// The resource deposit with this map ID, including depleted deposits.
    #[must_use]
    pub fn deposit(&self, id: DepositId) -> Option<&Deposit> {
        self.economy()?.deposit(id)
    }

    /// Resource deposits in stable map order, including depleted deposits.
    pub fn deposits(&self) -> impl Iterator<Item = (DepositId, &Deposit)> {
        self.economy()
            .into_iter()
            .flat_map(|economy| (0_u32..).map(DepositId).zip(economy.deposits.iter()))
    }

    /// The paid jobs on a producer in execution order; an eligible idle producer has an empty queue.
    #[must_use]
    pub fn production(&self, producer: UnitId) -> Option<&[Training]> {
        let unit = self.unit(producer)?;
        let economy = self.economy()?;
        if unit.kind != economy.barracks && self.headquarters(unit.owner) != Some(producer) {
            return None;
        }
        Some(economy.production.get(&producer).map_or(&[], Vec::as_slice))
    }

    pub(crate) fn remove_economy_unit(&mut self, id: UnitId) {
        if let Some(economy) = self.economy_mut() {
            economy.cargo.remove(&id);
            economy.production.remove(&id);
        }
    }

    pub(crate) fn order_training(
        &mut self,
        player: PlayerId,
        mut units: Vec<UnitId>,
        kind: TrainingKind,
    ) {
        let Some(economy) = self.economy() else {
            return;
        };
        let Some(rules) = self.rules.economy() else {
            return;
        };
        let barracks = economy.barracks;
        let (cost, remaining) = match kind {
            TrainingKind::Worker => (rules.worker_cost, rules.worker_ticks),
            TrainingKind::Scout => {
                let Some(scout) = rules.scout else {
                    return;
                };
                (scout.cost, scout.ticks)
            }
            TrainingKind::RifleSquad => (rules.rifle_cost, rules.rifle_ticks),
        };
        units.sort_unstable();
        units.dedup();
        for id in units {
            let Some(unit) = self.unit(id).filter(|unit| unit.owner == player) else {
                continue;
            };
            let eligible = match kind {
                TrainingKind::Worker => self.headquarters(player) == Some(id),
                TrainingKind::RifleSquad | TrainingKind::Scout => unit.kind == barracks,
            };
            if !eligible {
                continue;
            }
            let economy = self.economy_mut().expect("skirmish economy");
            let bank = &mut economy.banks[usize::from(player.0)];
            if !bank.can_afford(cost) {
                break;
            }
            let queue = economy.production.entry(id).or_default();
            if queue.len() < PRODUCTION_QUEUE_LIMIT {
                bank.spend(cost);
                queue.push(Training { kind, remaining });
            }
        }
    }

    pub(crate) fn advance_economy(&mut self) {
        let Some(economy) = self.economy() else {
            return;
        };
        let workers: Vec<_> = economy.work.keys().copied().collect();
        let manual_deliveries: Vec<_> = economy
            .cargo
            .keys()
            .filter(|id| !economy.work.contains_key(id))
            .copied()
            .collect();
        for worker in manual_deliveries {
            self.deliver_if_adjacent(worker);
        }
        for worker in workers {
            self.advance_work(worker);
        }
        let producers: Vec<_> = self
            .economy()
            .expect("skirmish economy")
            .production
            .keys()
            .copied()
            .collect();
        for producer in producers {
            self.advance_training(producer);
        }
    }

    fn advance_training(&mut self, producer: UnitId) {
        let economy = self.economy_mut().expect("skirmish economy");
        let job = &mut economy.production.get_mut(&producer).expect("active queue")[0];
        job.remaining = job.remaining.saturating_sub(1);
        if job.remaining > 0 {
            return;
        }
        let kind = match job.kind {
            TrainingKind::Worker => economy.worker,
            TrainingKind::RifleSquad => economy.rifle,
            TrainingKind::Scout => self
                .rules
                .unit_kind("scout")
                .expect("validated skirmish scout"),
        };
        let unit = &self.units[&producer];
        let owner = unit.owner;
        let Some(pos) = self.available_neighbors(unit.pos, kind).next() else {
            return;
        };
        let queues = &mut self.economy_mut().expect("skirmish economy").production;
        let queue = queues.get_mut(&producer).expect("active queue");
        queue.remove(0);
        if queue.is_empty() {
            queues.remove(&producer);
        }
        self.spawn_unit(kind, owner, pos);
    }

    fn available_neighbors(&self, target: IVec2, kind: UnitKind) -> impl Iterator<Item = IVec2> {
        let tile = tile_of(target);
        let size = u32::from(self.rules.unit(kind).size);
        let reserved = self.reserved_build_tiles();
        NEIGHBORS.into_iter().filter_map(move |offset| {
            let tile = tile + offset;
            let pos = tile_center(tile);
            if self.terrain.speed_modifier(pos, &self.rules) == 0
                || reserved.contains(&(tile.x, tile.y))
            {
                return None;
            }
            let used: u32 = self
                .units()
                .filter(|(_, unit)| tile_of(unit.pos) == tile)
                .map(|(_, unit)| u32::from(self.rules.unit(unit.kind).size))
                .sum();
            (used + size <= u32::from(self.rules.tile_capacity())).then_some(pos)
        })
    }
}

fn tile_center(tile: IVec2) -> IVec2 {
    tile * TILE + IVec2::splat(TILE / 2)
}
