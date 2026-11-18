//! The deterministic opponent for two-player skirmishes.
//!
//! The opponent reads only its own player's [`PlayerView`]: the terrain it has explored, the
//! deposits it has seen, its own units, and enemies currently in sight. It finds the enemy by
//! training scouts and sending them to unexplored cells, and remembers enemy buildings it has
//! seen. Its commands use view IDs; the caller translates them with
//! [`rts_base::PlayerKnowledge::translate_command`].

mod difficulty;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

pub use difficulty::{Difficulty, ScoutPattern};
use rts_base::{
    Command, DepositId, IVec2, PlayerView, ResourceKind, Resources, TILE, TileVisibility,
    TrainingKind, UnitId, UnitKind, tile_of,
};

const WORKER_LIMIT: usize = 8;
const DEFENSE_RADIUS: i64 = 7 * TILE as i64;
/// Side of a scouting cell, in tiles.
const CELL: i32 = 4;

/// One player's opponent. Feed it every frame of its view, in order; it keeps a memory of what
/// it has seen, and issues orders only on its difficulty's cadence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opponent {
    difficulty: Difficulty,
    /// Enemy buildings by view ID, where they were last seen.
    enemy_buildings: BTreeMap<UnitId, IVec2>,
    /// Scouting cells already chosen as destinations, by cell coordinates.
    claimed: BTreeSet<(i32, i32)>,
    /// The army has gathered and launched its attack.
    attacking: bool,
}

impl Opponent {
    #[must_use]
    pub const fn new(_view: &PlayerView, difficulty: Difficulty) -> Self {
        Self {
            difficulty,
            enemy_buildings: BTreeMap::new(),
            claimed: BTreeSet::new(),
            attacking: false,
        }
    }

    #[must_use]
    pub const fn difficulty(&self) -> Difficulty {
        self.difficulty
    }

    /// Observes the view's latest frame and returns orders in view IDs. Call once per tick.
    pub fn commands(&mut self, view: &PlayerView) -> Vec<Command> {
        self.remember(view);
        if view.outcome().is_some()
            || !view.is_skirmish()
            || !view.tick().is_multiple_of(self.difficulty.cadence())
        {
            return Vec::new();
        }
        let Some(base) = Base::read(view) else {
            return Vec::new();
        };
        let mut orders = Vec::new();
        let builder = base.build(view, &mut orders);
        base.gather(view, builder, &mut orders);
        base.train(view, builder.is_some(), &mut orders);
        self.scout(view, &base, &mut orders);
        self.fight(view, &base, &mut orders);
        orders
    }

    /// Records enemy buildings in sight and forgets those whose site is in sight but empty.
    fn remember(&mut self, view: &PlayerView) {
        let player = view.player();
        for (id, unit) in view.units() {
            if unit.owner != player && view.rules().unit(unit.kind).building {
                self.enemy_buildings.insert(id, unit.pos);
            }
        }
        self.enemy_buildings.retain(|&id, &mut pos| {
            view.unit(id).is_some() || view.visibility(tile_of(pos)) != TileVisibility::Visible
        });
    }

    fn scout(&mut self, view: &PlayerView, base: &Base, orders: &mut Vec<Command>) {
        for &scout in &base.scouts {
            if view.move_target(scout).is_some() {
                continue;
            }
            let from = tile_of(view.unit(scout).expect("own scout").pos);
            let reference = match self.difficulty.scout_pattern() {
                ScoutPattern::Nearest => from,
                ScoutPattern::Center => view.size() / 2,
                ScoutPattern::Mirror => mirror_tile(view),
            };
            if let Some(target) = self.claim_cell(view, reference) {
                orders.push(Command::Move {
                    units: vec![scout],
                    target,
                });
            }
        }
    }

    /// Claims the unexplored cell nearest `reference` and returns its center in world units.
    /// Once every unexplored cell has been tried, the claims are forgotten and tried again.
    fn claim_cell(&mut self, view: &PlayerView, reference: IVec2) -> Option<IVec2> {
        let cells = (view.size() + IVec2::splat(CELL - 1)) / CELL;
        let pick = |claimed: &BTreeSet<(i32, i32)>| {
            (0..cells.y)
                .flat_map(|y| (0..cells.x).map(move |x| IVec2::new(x, y)))
                .filter(|cell| !claimed.contains(&(cell.x, cell.y)))
                .map(|cell| (cell, cell_center(view, cell)))
                .filter(|&(_, tile)| view.visibility(tile) == TileVisibility::Unexplored)
                .min_by_key(|&(cell, tile)| (distance_squared(tile, reference), cell.y, cell.x))
        };
        let (cell, tile) = pick(&self.claimed).or_else(|| {
            self.claimed.clear();
            pick(&self.claimed)
        })?;
        self.claimed.insert((cell.x, cell.y));
        Some(tile * TILE + IVec2::splat(TILE / 2))
    }

    fn fight(&mut self, view: &PlayerView, base: &Base, orders: &mut Vec<Command>) {
        if base.rifles.is_empty() {
            self.attacking = false;
            return;
        }
        let player = view.player();
        let armed = |kind: UnitKind| view.rules().unit(kind).weapon.is_some();
        let threat = view
            .units()
            .filter(|(_, unit)| unit.owner != player && armed(unit.kind))
            .map(|(id, unit)| (distance_squared(unit.pos, base.center), id))
            .filter(|&(distance, _)| distance <= DEFENSE_RADIUS.pow(2))
            .min();
        if let Some((_, target)) = threat {
            attack(view, &base.rifles, target, orders);
            return;
        }
        if !self.attacking {
            if base.rifles.len() >= self.difficulty.attack_squads()
                && view.tick() >= self.difficulty.first_attack_tick()
            {
                self.attacking = true;
            } else {
                rally(view, base, orders);
                return;
            }
        }
        let army = centroid(view, &base.rifles);
        // Armed enemies first, then anything else in sight, nearest the army.
        let in_sight = view
            .units()
            .filter(|(_, unit)| unit.owner != player)
            .map(|(id, unit)| (!armed(unit.kind), distance_squared(unit.pos, army), id))
            .min();
        if let Some((_, _, target)) = in_sight {
            attack(view, &base.rifles, target, orders);
            return;
        }
        let remembered = self
            .enemy_buildings
            .iter()
            .min_by_key(|&(&id, &pos)| (distance_squared(pos, army), id))
            .map(|(_, &pos)| pos);
        if let Some(target) = remembered {
            let units: Vec<_> = base
                .rifles
                .iter()
                .copied()
                .filter(|&id| view.move_target(id) != Some(target))
                .collect();
            if !units.is_empty() {
                orders.push(Command::Move { units, target });
            }
            return;
        }
        let idle: Vec<_> = base
            .rifles
            .iter()
            .copied()
            .filter(|&id| view.move_target(id).is_none() && view.attack_target(id).is_none())
            .collect();
        if !idle.is_empty()
            && let Some(target) = self.claim_cell(view, mirror_tile(view))
        {
            orders.push(Command::Move {
                units: idle,
                target,
            });
        }
    }
}

struct Base {
    headquarters: UnitId,
    center: IVec2,
    /// Toward the far side of the map, along x: -1 or 1.
    forward: i32,
    bank: Resources,
    workers: Vec<UnitId>,
    barracks: Vec<UnitId>,
    rifles: Vec<UnitId>,
    scouts: Vec<UnitId>,
    scout_kind: Option<UnitKind>,
}

impl Base {
    fn read(view: &PlayerView) -> Option<Self> {
        let rules = view.rules();
        let headquarters = view.headquarters()?;
        let worker_kind = rules.unit_kind("worker")?;
        let barracks_kind = rules.unit_kind("barracks")?;
        let scout_kind = rules
            .economy()
            .and_then(|economy| economy.scout)
            .and_then(|_| rules.unit_kind("scout"));
        let center = view.unit(headquarters)?.pos;
        let mut base = Self {
            headquarters,
            center,
            forward: if mirror_tile(view).x < tile_of(center).x {
                -1
            } else {
                1
            },
            bank: view.resources()?,
            workers: Vec::new(),
            barracks: Vec::new(),
            rifles: Vec::new(),
            scouts: Vec::new(),
            scout_kind,
        };
        for (id, unit) in view.units().filter(|(_, unit)| unit.owner == view.player()) {
            if unit.kind == worker_kind {
                base.workers.push(id);
            } else if unit.kind == barracks_kind {
                base.barracks.push(id);
            } else if Some(unit.kind) == scout_kind {
                base.scouts.push(id);
            } else if rules.unit(unit.kind).weapon.is_some() {
                base.rifles.push(id);
            }
        }
        Some(base)
    }

    fn build(&self, view: &PlayerView, orders: &mut Vec<Command>) -> Option<UnitId> {
        let rules = view.rules().economy()?;
        if !self.barracks.is_empty()
            || !self.bank.can_afford(rules.barracks_cost)
            || self
                .workers
                .iter()
                .any(|&id| view.build_target(id).is_some())
        {
            return None;
        }
        let tile = tile_of(self.center);
        let forward = self.forward;
        let target = [
            IVec2::new(forward * 3, 0),
            IVec2::new(forward * 3, -2),
            IVec2::new(forward * 3, 2),
            IVec2::new(0, -3),
            IVec2::new(0, 3),
            IVec2::new(-forward * 3, 0),
        ]
        .into_iter()
        .map(|offset| (tile + offset) * TILE + IVec2::splat(TILE / 2))
        .find(|&target| view.can_place_barracks(target))?;
        let builder =
            self.workers.iter().copied().min_by_key(|&id| {
                distance_squared(view.unit(id).expect("own worker").pos, target)
            })?;
        orders.push(Command::BuildBarracks {
            units: vec![builder],
            target,
        });
        Some(builder)
    }

    fn gather(&self, view: &PlayerView, builder: Option<UnitId>, orders: &mut Vec<Command>) {
        let mut gatherers = [0, 0];
        for &worker in &self.workers {
            if Some(worker) != builder
                && let Some(deposit) = view.gather_target(worker).and_then(|id| view.deposit(id))
                && deposit.remaining > 0
            {
                gatherers[resource_index(deposit.kind)] += 1;
            }
        }
        for &worker in &self.workers {
            if Some(worker) == builder
                || view.build_target(worker).is_some()
                || view.gather_target(worker).is_some()
            {
                continue;
            }
            let position = view.unit(worker).expect("own worker").pos;
            let Some((deposit, kind)) = self.choose_deposit(view, position, gatherers) else {
                continue;
            };
            gatherers[resource_index(kind)] += 1;
            orders.push(Command::Gather {
                units: vec![worker],
                deposit,
            });
        }
    }

    /// The best known deposit with stock left when last seen.
    fn choose_deposit(
        &self,
        view: &PlayerView,
        position: IVec2,
        gatherers: [usize; 2],
    ) -> Option<(DepositId, ResourceKind)> {
        view.deposits()
            .filter(|(_, deposit)| deposit.remaining > 0)
            .min_by_key(|&(id, deposit)| {
                (
                    gatherers[resource_index(deposit.kind)],
                    distance_squared(self.center, deposit.pos) * 2
                        + distance_squared(position, deposit.pos),
                    id,
                )
            })
            .map(|(id, deposit)| (id, deposit.kind))
    }

    fn train(&self, view: &PlayerView, placed_barracks: bool, orders: &mut Vec<Command>) {
        let Some(rules) = view.rules().economy() else {
            return;
        };
        let mut bank = self.bank;
        if placed_barracks {
            spend(&mut bank, rules.barracks_cost);
        }
        let scout_training = self.barracks.iter().any(|&id| {
            view.production(id)
                .is_some_and(|queue| queue.iter().any(|job| job.kind == TrainingKind::Scout))
        });
        let mut need_scout = self.scout_kind.is_some() && self.scouts.is_empty() && !scout_training;
        for &producer in &self.barracks {
            if !idle_producer(view, producer) {
                continue;
            }
            if need_scout && let Some(scout) = rules.scout {
                if bank.can_afford(scout.cost) {
                    spend(&mut bank, scout.cost);
                    orders.push(Command::TrainScout {
                        units: vec![producer],
                    });
                }
                need_scout = false;
            } else if bank.can_afford(rules.rifle_cost) {
                spend(&mut bank, rules.rifle_cost);
                orders.push(Command::TrainRifleSquad {
                    units: vec![producer],
                });
            }
        }
        let can_expand = !self.barracks.is_empty()
            || self.workers.len() < 4
            || self
                .workers
                .iter()
                .any(|&id| view.build_target(id).is_some());
        if can_expand
            && self.workers.len() < WORKER_LIMIT
            && bank.can_afford(rules.worker_cost)
            && idle_producer(view, self.headquarters)
        {
            orders.push(Command::TrainWorker {
                units: vec![self.headquarters],
            });
        }
    }
}

/// Orders every rifle squad not already on `target` to attack it.
fn attack(view: &PlayerView, rifles: &[UnitId], target: UnitId, orders: &mut Vec<Command>) {
    let units: Vec<_> = rifles
        .iter()
        .copied()
        .filter(|&id| view.attack_target(id) != Some(target))
        .collect();
    if !units.is_empty() {
        orders.push(Command::Attack { units, target });
    }
}

/// Gathers idle squads on known open ground in front of the headquarters.
fn rally(view: &PlayerView, base: &Base, orders: &mut Vec<Command>) {
    let tile = tile_of(base.center);
    let Some(point) = [
        IVec2::new(base.forward * 4, 0),
        IVec2::new(base.forward * 4, -2),
        IVec2::new(base.forward * 4, 2),
        IVec2::new(base.forward * 2, 0),
    ]
    .into_iter()
    .map(|offset| (tile + offset) * TILE + IVec2::splat(TILE / 2))
    .find(|&pos| view.speed_modifier(pos).is_some_and(|speed| speed > 0)) else {
        return;
    };
    let units: Vec<_> = base
        .rifles
        .iter()
        .copied()
        .filter(|&id| {
            view.move_target(id).is_none()
                && view.attack_target(id).is_none()
                && distance_squared(view.unit(id).expect("own squad").pos, point)
                    > i64::from(2 * TILE).pow(2)
        })
        .collect();
    if !units.is_empty() {
        orders.push(Command::Move {
            units,
            target: point,
        });
    }
}

/// The tile opposite the player's own start through the map center: where a symmetric map
/// would put the enemy. Derived only from the player's start and the map size.
fn mirror_tile(view: &PlayerView) -> IVec2 {
    view.size() - IVec2::ONE - tile_of(view.start())
}

fn cell_center(view: &PlayerView, cell: IVec2) -> IVec2 {
    (cell * CELL + IVec2::splat(CELL / 2)).min(view.size() - IVec2::ONE)
}

fn centroid(view: &PlayerView, units: &[UnitId]) -> IVec2 {
    let count = i32::try_from(units.len()).expect("bounded unit count");
    units
        .iter()
        .map(|&id| view.unit(id).expect("own unit").pos)
        .fold(IVec2::ZERO, |sum, pos| sum + pos)
        / count
}

fn idle_producer(view: &PlayerView, unit: UnitId) -> bool {
    view.production(unit).is_some_and(<[_]>::is_empty)
}

const fn resource_index(kind: ResourceKind) -> usize {
    match kind {
        ResourceKind::Wood => 0,
        ResourceKind::Gold => 1,
    }
}

const fn spend(bank: &mut Resources, cost: Resources) {
    bank.wood -= cost.wood;
    bank.gold -= cost.gold;
}

fn distance_squared(a: IVec2, b: IVec2) -> i64 {
    let dx = i64::from(a.x) - i64::from(b.x);
    let dy = i64::from(a.y) - i64::from(b.y);
    dx * dx + dy * dy
}
