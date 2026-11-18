//! The deterministic opponent for two-player skirmishes.

mod difficulty;

pub use difficulty::{Difficulty, ScoutPattern};

use rts_base::{
    Command, DepositId, IVec2, PlayerId, ResourceKind, Resources, Sim, TILE, UnitId, tile_of,
};

const WORKER_LIMIT: usize = 8;
const DEFENSE_RADIUS: i64 = 7 * TILE as i64;

/// Returns orders the player can submit through the ordinary turn pipeline.
#[must_use]
pub fn commands(sim: &Sim, player: PlayerId) -> Vec<Command> {
    let Some(base) = Base::read(sim, player) else {
        return Vec::new();
    };
    let mut orders = Vec::new();
    let builder = base.build(sim, &mut orders);
    base.gather(sim, builder, &mut orders);
    base.train(sim, builder.is_some(), &mut orders);
    base.fight(sim, &mut orders);
    orders
}

struct Base {
    player: PlayerId,
    headquarters: UnitId,
    enemy_headquarters: UnitId,
    center: IVec2,
    bank: Resources,
    workers: Vec<UnitId>,
    barracks: Vec<UnitId>,
    rifles: Vec<UnitId>,
}

impl Base {
    fn read(sim: &Sim, player: PlayerId) -> Option<Self> {
        if sim.outcome().is_some() || player.0 > 1 {
            return None;
        }
        let headquarters = sim.headquarters(player)?;
        let enemy_headquarters = sim.headquarters(PlayerId(1 - player.0))?;
        let worker_kind = sim.rules().unit_kind("worker")?;
        let barracks_kind = sim.rules().unit_kind("barracks")?;
        let mut base = Self {
            player,
            headquarters,
            enemy_headquarters,
            center: sim.unit(headquarters)?.pos,
            bank: sim.resources(player)?,
            workers: Vec::new(),
            barracks: Vec::new(),
            rifles: Vec::new(),
        };
        for (id, unit) in sim.units().filter(|(_, unit)| unit.owner == player) {
            if unit.kind == worker_kind {
                base.workers.push(id);
            } else if unit.kind == barracks_kind {
                base.barracks.push(id);
            } else if sim.rules().unit(unit.kind).weapon.is_some() {
                base.rifles.push(id);
            }
        }
        Some(base)
    }

    fn build(&self, sim: &Sim, orders: &mut Vec<Command>) -> Option<UnitId> {
        let rules = sim.rules().economy()?;
        if !self.barracks.is_empty()
            || !self.bank.can_afford(rules.barracks_cost)
            || self
                .workers
                .iter()
                .any(|&id| sim.build_target(id).is_some())
        {
            return None;
        }
        let tile = tile_of(self.center);
        let toward_enemy = (sim.unit(self.enemy_headquarters)?.pos.x - self.center.x).signum();
        let target = [
            IVec2::new(toward_enemy * 3, 0),
            IVec2::new(toward_enemy * 3, -2),
            IVec2::new(toward_enemy * 3, 2),
            IVec2::new(0, -3),
            IVec2::new(0, 3),
            IVec2::new(-toward_enemy * 3, 0),
        ]
        .into_iter()
        .map(|offset| (tile + offset) * TILE + IVec2::splat(TILE / 2))
        .find(|&target| sim.can_place_barracks(self.player, target))?;
        let builder =
            self.workers.iter().copied().min_by_key(|&id| {
                distance_squared(sim.unit(id).expect("living worker").pos, target)
            })?;
        orders.push(Command::BuildBarracks {
            units: vec![builder],
            target,
        });
        Some(builder)
    }

    fn gather(&self, sim: &Sim, builder: Option<UnitId>, orders: &mut Vec<Command>) {
        let mut gatherers = [0, 0];
        for &worker in &self.workers {
            if Some(worker) != builder
                && let Some(deposit) = sim.gather_target(worker).and_then(|id| sim.deposit(id))
                && deposit.remaining > 0
            {
                gatherers[resource_index(deposit.kind)] += 1;
            }
        }
        for &worker in &self.workers {
            if Some(worker) == builder
                || sim.build_target(worker).is_some()
                || sim.gather_target(worker).is_some()
            {
                continue;
            }
            let position = sim.unit(worker).expect("living worker").pos;
            let Some((deposit, kind)) = self.choose_deposit(sim, position, gatherers) else {
                continue;
            };
            gatherers[resource_index(kind)] += 1;
            orders.push(Command::Gather {
                units: vec![worker],
                deposit,
            });
        }
    }

    fn choose_deposit(
        &self,
        sim: &Sim,
        position: IVec2,
        gatherers: [usize; 2],
    ) -> Option<(DepositId, ResourceKind)> {
        sim.deposits()
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

    fn train(&self, sim: &Sim, placed_barracks: bool, orders: &mut Vec<Command>) {
        let Some(rules) = sim.rules().economy() else {
            return;
        };
        let mut bank = self.bank;
        if placed_barracks {
            spend(&mut bank, rules.barracks_cost);
        }
        for &producer in &self.barracks {
            if bank.can_afford(rules.rifle_cost) && idle_producer(sim, producer) {
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
                .any(|&id| sim.build_target(id).is_some());
        if can_expand
            && self.workers.len() < WORKER_LIMIT
            && bank.can_afford(rules.worker_cost)
            && idle_producer(sim, self.headquarters)
        {
            orders.push(Command::TrainWorker {
                units: vec![self.headquarters],
            });
        }
    }

    fn fight(&self, sim: &Sim, orders: &mut Vec<Command>) {
        if self.rifles.is_empty() {
            return;
        }
        let target = sim
            .units()
            .filter(|(_, unit)| {
                unit.owner != self.player && sim.rules().unit(unit.kind).weapon.is_some()
            })
            .map(|(id, unit)| (distance_squared(unit.pos, self.center), id))
            .filter(|&(distance, _)| distance <= DEFENSE_RADIUS.pow(2))
            .min()
            .map_or(self.enemy_headquarters, |(_, id)| id);
        let units: Vec<_> = self
            .rifles
            .iter()
            .copied()
            .filter(|&id| sim.attack_target(id) != Some(target))
            .collect();
        if !units.is_empty() {
            orders.push(Command::Attack { units, target });
        }
    }
}

fn idle_producer(sim: &Sim, unit: UnitId) -> bool {
    sim.production(unit).is_some_and(<[_]>::is_empty)
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
