//! Worker commands, harvesting and construction.

use std::{
    collections::BTreeSet,
    hash::{Hash, Hasher},
};

use crate::{IVec2, PlayerId, Sim, UnitId, UnitKind, movement::MoveOrder, tile_of};

use super::{DepositId, ResourceKind, tile_center};

#[derive(Debug, Clone, Copy)]
pub(super) enum Work {
    Gather { deposit: DepositId, progress: u32 },
    Build { target: IVec2, remaining: u32 },
}

impl Hash for Work {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Gather { deposit, progress } => {
                0_u8.hash(state);
                deposit.hash(state);
                progress.hash(state);
            }
            Self::Build { target, remaining } => {
                1_u8.hash(state);
                target.hash(state);
                remaining.hash(state);
            }
        }
    }
}

impl Sim {
    #[must_use]
    pub fn gather_target(&self, worker: UnitId) -> Option<DepositId> {
        match self.economy()?.work.get(&worker)? {
            Work::Gather { deposit, .. } => Some(*deposit),
            Work::Build { .. } => None,
        }
    }

    #[must_use]
    pub fn build_target(&self, worker: UnitId) -> Option<IVec2> {
        match self.economy()?.work.get(&worker)? {
            Work::Build { target, .. } => Some(*target),
            Work::Gather { .. } => None,
        }
    }

    #[must_use]
    pub fn construction_remaining(&self, worker: UnitId) -> Option<u32> {
        match self.economy()?.work.get(&worker)? {
            Work::Build { remaining, .. } => Some(*remaining),
            Work::Gather { .. } => None,
        }
    }

    #[must_use]
    pub fn cargo(&self, worker: UnitId) -> Option<(ResourceKind, u32)> {
        self.economy()?.cargo.get(&worker).copied()
    }

    /// Whether the target tile is traversable, unoccupied, and free of deposits and construction.
    #[must_use]
    pub fn can_place_barracks(&self, player: PlayerId, target: IVec2) -> bool {
        let Some(economy) = self.economy() else {
            return false;
        };
        if self.resources(player).is_none()
            || self.outcome().is_some()
            || !self.can_target(player, target)
            || self.terrain.speed_modifier(target, &self.rules) == 0
        {
            return false;
        }
        let tile = tile_of(target);
        !self.units().any(|(_, unit)| tile_of(unit.pos) == tile)
            && !economy
                .deposits
                .iter()
                .any(|deposit| tile_of(deposit.pos) == tile)
            && !economy
                .work
                .values()
                .any(|work| matches!(work, Work::Build { target, .. } if tile_of(*target) == tile))
    }

    pub(crate) fn reserved_build_tiles(&self) -> BTreeSet<(i32, i32)> {
        self.economy()
            .into_iter()
            .flat_map(|economy| economy.work.values())
            .filter_map(|work| match *work {
                Work::Build { target, .. } => {
                    let tile = tile_of(target);
                    Some((tile.x, tile.y))
                }
                Work::Gather { .. } => None,
            })
            .collect()
    }

    pub(crate) fn cancel_work(&mut self, worker: UnitId) {
        let Some(economy) = self.economy_mut() else {
            return;
        };
        if matches!(economy.work.remove(&worker), Some(Work::Build { .. })) {
            let owner = self.units[&worker].owner;
            let cost = self
                .rules
                .economy()
                .expect("skirmish economy")
                .barracks_cost;
            self.economy_mut().expect("skirmish economy").banks[usize::from(owner.0)].refund(cost);
        }
    }

    pub(crate) fn order_gather(
        &mut self,
        player: PlayerId,
        mut units: Vec<UnitId>,
        deposit: DepositId,
    ) {
        let Some(economy) = self.economy() else {
            return;
        };
        let Some(target) = economy
            .deposit(deposit)
            .filter(|deposit| self.can_target(player, deposit.pos) && deposit.remaining > 0)
        else {
            return;
        };
        let pos = target.pos;
        let worker_kind = economy.worker;
        units.sort_unstable();
        units.dedup();
        for id in units {
            let Some(unit) = self
                .unit(id)
                .filter(|unit| unit.owner == player && unit.kind == worker_kind)
            else {
                continue;
            };
            if self.gather_target(id) == Some(deposit)
                || self.work_route(player, unit.pos, pos).is_none()
            {
                continue;
            }
            self.replace_work(
                id,
                Work::Gather {
                    deposit,
                    progress: 0,
                },
            );
        }
    }

    pub(crate) fn order_barracks(
        &mut self,
        player: PlayerId,
        mut units: Vec<UnitId>,
        target: IVec2,
    ) {
        if !self.can_place_barracks(player, target) {
            return;
        }
        let worker_kind = self.economy().expect("skirmish placement").worker;
        let rules = *self.rules.economy().expect("skirmish economy");
        let Some(bank) = self.resources(player) else {
            return;
        };
        if !bank.can_afford(rules.barracks_cost) {
            return;
        }
        let target = tile_center(tile_of(target));
        units.sort_unstable();
        let Some((worker, approach)) = units.into_iter().find_map(|id| {
            let unit = self
                .unit(id)
                .filter(|unit| unit.owner == player && unit.kind == worker_kind)?;
            self.approach(player, unit.pos, target, worker_kind)
                .map(|order| (id, order))
        }) else {
            return;
        };
        self.replace_work(
            worker,
            Work::Build {
                target,
                remaining: rules.barracks_ticks,
            },
        );
        self.economy_mut().expect("skirmish economy").banks[usize::from(player.0)]
            .spend(rules.barracks_cost);
        self.move_orders.insert(worker, approach);
    }

    fn replace_work(&mut self, worker: UnitId, work: Work) {
        self.cancel_work(worker);
        self.cancel_exploration(worker);
        self.move_orders.remove(&worker);
        self.attack_orders.remove(&worker);
        self.economy_mut()
            .expect("skirmish economy")
            .work
            .insert(worker, work);
    }

    pub(super) fn advance_work(&mut self, worker: UnitId) {
        let work = self.economy().expect("skirmish economy").work[&worker];
        match work {
            Work::Gather { deposit, progress } => self.advance_gather(worker, deposit, progress),
            Work::Build { target, remaining } => {
                self.advance_construction(worker, target, remaining);
            }
        }
    }

    fn advance_gather(&mut self, worker: UnitId, id: DepositId, progress: u32) {
        let economy = self.economy().expect("skirmish economy");
        let deposit = *economy.deposit(id).expect("active deposit");
        let cargo = self.cargo(worker);
        let rules = *self.rules.economy().expect("skirmish rules");
        let unit = &self.units[&worker];
        let (owner, pos, kind) = (unit.owner, unit.pos, unit.kind);
        // Remote stock changes remain unknown until this player can see the deposit again.
        let depleted = self.can_target(owner, deposit.pos) && deposit.remaining == 0;
        if cargo.is_some_and(|(kind, amount)| {
            kind != deposit.kind || amount == rules.carry_capacity || depleted
        }) {
            if let Some(hq) = self.headquarters(owner) {
                let target = self.units[&hq].pos;
                if self.deliver_if_adjacent(worker) {
                    self.move_orders.remove(&worker);
                } else {
                    self.move_to_work_site(worker, pos, target, kind);
                }
            }
        } else if depleted {
            self.cancel_work(worker);
            self.move_orders.remove(&worker);
        } else if pos == deposit.pos {
            let economy = self.economy_mut().expect("skirmish economy");
            let progress = progress + 1;
            if progress == rules.harvest_ticks {
                let deposit = &mut economy.deposits[usize::try_from(id.0).expect("active deposit")];
                let (_, amount) = economy.cargo.entry(worker).or_insert((deposit.kind, 0));
                let harvested = (rules.carry_capacity - *amount).min(deposit.remaining);
                deposit.remaining -= harvested;
                *amount += harvested;
                economy.work.insert(
                    worker,
                    Work::Gather {
                        deposit: id,
                        progress: 0,
                    },
                );
            } else {
                economy.work.insert(
                    worker,
                    Work::Gather {
                        deposit: id,
                        progress,
                    },
                );
            }
        } else if !self.move_orders.contains_key(&worker)
            && let Some(order) = self.work_route(owner, pos, deposit.pos)
        {
            self.move_orders.insert(worker, order);
        }
    }

    pub(super) fn deliver_if_adjacent(&mut self, worker: UnitId) -> bool {
        let unit = &self.units[&worker];
        let owner = unit.owner;
        if !self
            .headquarters(owner)
            .is_some_and(|id| adjacent(unit.pos, self.units[&id].pos))
        {
            return false;
        }
        let economy = self.economy_mut().expect("skirmish economy");
        let (kind, amount) = economy.cargo.remove(&worker).expect("delivery cargo");
        *economy.banks[usize::from(owner.0)].count_mut(kind) += amount;
        true
    }

    fn advance_construction(&mut self, worker: UnitId, target: IVec2, remaining: u32) {
        let unit = &self.units[&worker];
        let (owner, pos, kind) = (unit.owner, unit.pos, unit.kind);
        if adjacent(pos, target) {
            self.move_orders.remove(&worker);
            let economy = self.economy_mut().expect("skirmish economy");
            if remaining == 1 {
                economy.work.remove(&worker);
                let kind = economy.barracks;
                self.spawn_unit(kind, owner, target);
            } else {
                economy.work.insert(
                    worker,
                    Work::Build {
                        target,
                        remaining: remaining - 1,
                    },
                );
            }
        } else {
            self.move_to_work_site(worker, pos, target, kind);
        }
    }

    fn move_to_work_site(&mut self, worker: UnitId, from: IVec2, site: IVec2, kind: UnitKind) {
        let player = self.units[&worker].owner;
        let current = self.move_target(worker);
        if current.is_some_and(|current| {
            !self.can_target(player, current)
                || self
                    .available_neighbors(site, kind)
                    .any(|pos| pos == current)
        }) {
            return;
        }
        if let Some(route) = self.approach(player, from, site, kind) {
            self.move_orders.insert(worker, route);
        }
    }

    fn approach(
        &self,
        player: PlayerId,
        from: IVec2,
        target: IVec2,
        kind: UnitKind,
    ) -> Option<MoveOrder> {
        if adjacent(from, target) {
            return self.work_route(player, from, from);
        }
        let mut candidates: Vec<_> = self
            .available_neighbors(target, kind)
            .filter(|&pos| self.can_target(player, pos))
            .collect();
        // Prefer the nearest approach; stable sorting retains row-major order on ties.
        candidates.sort_by_key(|&pos| pos.x.abs_diff(from.x) + pos.y.abs_diff(from.y));
        let mut blockers = self.work_route_blockers(player);
        let site = tile_of(target);
        blockers.insert((site.x, site.y));
        candidates.into_iter().find_map(|pos| {
            self.route_known(player, from, pos, &|tile| {
                !blockers.contains(&(tile.x, tile.y))
            })
        })
    }

    fn work_route(&self, player: PlayerId, from: IVec2, to: IVec2) -> Option<MoveOrder> {
        let blockers = self.work_route_blockers(player);
        self.route_known(player, from, to, &|tile| {
            !blockers.contains(&(tile.x, tile.y))
        })
    }

    fn work_route_blockers(&self, player: PlayerId) -> BTreeSet<(i32, i32)> {
        let mut blockers = self.reserved_build_tiles();
        blockers.retain(|&(x, y)| self.can_target(player, tile_center(IVec2::new(x, y))));
        blockers.extend(
            self.units()
                .filter(|(_, unit)| {
                    self.rules.unit(unit.kind).building && self.can_target(player, unit.pos)
                })
                .map(|(_, unit)| {
                    let tile = tile_of(unit.pos);
                    (tile.x, tile.y)
                }),
        );
        blockers
    }
}

const fn adjacent(from: IVec2, to: IVec2) -> bool {
    let (a, b) = (tile_of(from), tile_of(to));
    a.x.abs_diff(b.x) + a.y.abs_diff(b.y) == 1
}
