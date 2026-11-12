//! Weapons, attack orders, and casualties.
//!
//! Each tick, units with attack orders first close in on their targets. After movement,
//! every armed unit that is not moving fires a volley at its ordered target, or else at the
//! nearest enemy in range. Every living soldier fires one shot, and each shot hits the living
//! soldier of the target squad standing nearest the shooter, so the facing rank falls first.
//! Defeated soldiers count as kills; reaching a new rank restores the whole squad.

use crate::{IVec2, PlayerId, Sim, UnitId, movement::MoveOrder, rules::Weapon, tile_of};

/// Squared distance between two world positions, without overflow.
fn distance_squared(a: IVec2, b: IVec2) -> i64 {
    let dx = i64::from(a.x) - i64::from(b.x);
    let dy = i64::from(a.y) - i64::from(b.y);
    dx * dx + dy * dy
}

fn in_range(from: IVec2, to: IVec2, weapon: &Weapon) -> bool {
    distance_squared(from, to) <= i64::from(weapon.range) * i64::from(weapon.range)
}

impl Sim {
    /// Starts or ends pursuit for attack orders. Orders whose target died or cannot be reached
    /// end; attackers in range stop so they can fire this tick.
    pub(crate) fn pursue_targets(&mut self) {
        let orders: Vec<_> = self
            .attack_orders
            .iter()
            .map(|(&id, &target)| (id, target))
            .collect();
        for (id, target) in orders {
            let unit = &self.units[&id];
            let weapon = self
                .rules
                .unit(unit.kind)
                .weapon
                .expect("attack orders need weapons");
            let Some(goal) = self.units.get(&target).map(|target| target.pos) else {
                self.end_attack(id);
                continue;
            };
            if in_range(unit.pos, goal, &weapon) {
                self.move_orders.remove(&id);
                continue;
            }
            let current = self.move_orders.get(&id).map(MoveOrder::target);
            if current.is_some_and(|current| tile_of(current) == tile_of(goal)) {
                continue;
            }
            match MoveOrder::new(unit.pos, goal, &self.terrain, &self.rules) {
                Some(order) => {
                    self.move_orders.insert(id, order);
                }
                None => self.end_attack(id),
            }
        }
    }

    fn end_attack(&mut self, id: UnitId) {
        self.attack_orders.remove(&id);
        self.move_orders.remove(&id);
    }

    /// Fires every ready, stationary weapon in unit ID order. Casualties apply at once, so a
    /// unit destroyed earlier in the tick no longer fires.
    pub(crate) fn advance_combat(&mut self) {
        let armed: Vec<_> = self
            .units
            .iter()
            .filter(|(_, unit)| self.rules.unit(unit.kind).weapon.is_some())
            .map(|(&id, _)| id)
            .collect();
        for id in armed {
            let Some(unit) = self.units.get_mut(&id) else {
                continue;
            };
            unit.cooldown = unit.cooldown.saturating_sub(1);
            if unit.cooldown > 0 || self.move_orders.contains_key(&id) {
                continue;
            }
            let (pos, owner) = (unit.pos, unit.owner);
            let weapon = self.rules.unit(unit.kind).weapon.expect("armed unit");
            let target = match self.attack_orders.get(&id) {
                Some(target) => self
                    .units
                    .get(target)
                    .is_some_and(|target| in_range(pos, target.pos, &weapon))
                    .then_some(*target),
                None => self.nearest_enemy(pos, owner, &weapon),
            };
            if let Some(target) = target {
                self.fire(id, target, &weapon);
            }
        }
    }

    /// The closest unit of another player within range; ties go to the lowest ID.
    fn nearest_enemy(&self, pos: IVec2, owner: PlayerId, weapon: &Weapon) -> Option<UnitId> {
        self.units
            .iter()
            .filter(|(_, unit)| unit.owner != owner && in_range(pos, unit.pos, weapon))
            .min_by_key(|&(&id, unit)| (distance_squared(pos, unit.pos), id))
            .map(|(&id, _)| id)
    }

    fn fire(&mut self, id: UnitId, target: UnitId, weapon: &Weapon) {
        let shooter = &self.units[&id];
        let shots = shooter.alive();
        let from = shooter.pos;
        let damage = self.rules.shot_damage(weapon, shooter.kills);
        let mut kills = 0;
        let victim = self.units.get_mut(&target).expect("target is alive");
        let definition = self.rules.unit(victim.kind);
        let grid = definition.ranks();
        let half_size = IVec2::splat(definition.radius);
        for _ in 0..shots {
            let nearest = victim
                .soldiers
                .iter()
                .enumerate()
                .filter(|&(_, &health)| health > 0)
                .min_by_key(|&(slot, _)| {
                    let slot = u8::try_from(slot).expect("squads have at most 255 soldiers");
                    distance_squared(from, victim.pos + grid.offset(slot, half_size))
                })
                .map(|(slot, _)| slot);
            let Some(slot) = nearest else {
                break;
            };
            let health = &mut victim.soldiers[slot];
            *health = health.saturating_sub(damage).max(0);
            if *health == 0 {
                kills += 1;
            }
        }
        let destroyed = victim.alive() == 0;
        let shooter = self.units.get_mut(&id).expect("shooter is alive");
        shooter.cooldown = weapon.cooldown;
        let rank = self.rules.rank(shooter.kills);
        shooter.kills = shooter.kills.saturating_add(kills);
        if self.rules.rank(shooter.kills) > rank {
            let max_health = self.rules.unit(shooter.kind).max_health;
            shooter.soldiers.fill(max_health);
        }
        if destroyed {
            self.remove_unit(target);
        }
    }
}
