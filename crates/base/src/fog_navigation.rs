//! Navigation that advances through explored terrain, stopping at the frontier until sight expands.

use std::collections::{BTreeMap, VecDeque};

use crate::{
    IVec2, PlayerId, Sim, TILE, TileVisibility, UnitId,
    movement::{MoveGroupId, MoveOrder},
    tile_of,
    visibility::FogVisibility,
};

const NEIGHBORS: [IVec2; 4] = [IVec2::NEG_Y, IVec2::NEG_X, IVec2::X, IVec2::Y];

#[derive(Debug, Clone, Hash)]
pub struct FogNavigation {
    pub(super) views: Vec<FogVisibility>,
    destinations: BTreeMap<UnitId, (IVec2, Option<MoveGroupId>)>,
}

impl Sim {
    /// Applies fog-of-war rules for combat, exploration, and economic orders.
    ///
    /// Select this when starting a fog-enabled match. Older full recordings keep the
    /// original simulation rules.
    pub fn enforce_fog(&mut self) {
        if self.fog_navigation.is_none() {
            self.fog_navigation = Some(FogNavigation {
                views: vec![FogVisibility::new(self.terrain.size()); self.players.len()],
                destinations: BTreeMap::new(),
            });
            self.refresh_navigation_sight();
        }
    }

    pub(super) fn refresh_navigation_sight(&mut self) {
        if let Some(navigation) = &mut self.fog_navigation {
            for (player, view) in (0_u8..).map(PlayerId).zip(&mut navigation.views) {
                view.update(
                    self.units
                        .values()
                        .filter(|unit| unit.owner == player)
                        .map(|unit| (unit.pos, self.rules.unit(unit.kind).sight)),
                );
            }
        }
    }

    pub(super) fn route_known(
        &self,
        player: PlayerId,
        start: IVec2,
        target: IVec2,
        can_plan: &impl Fn(IVec2) -> bool,
    ) -> Option<MoveOrder> {
        let explored = |tile| {
            self.fog_navigation.as_ref().is_none_or(|fog| {
                fog.views[usize::from(player.0)].tile(tile) != TileVisibility::Unexplored
            })
        };
        if !explored(tile_of(target)) {
            return None;
        }
        MoveOrder::new_avoiding(start, target, &self.terrain, &self.rules, &|tile| {
            explored(tile) && can_plan(tile)
        })
    }

    /// The route runs only through known tiles. Where a full path is unavailable,
    /// go to a known frontier that can reveal another part of the route.
    pub(super) fn plan_exploration(
        &self,
        player: PlayerId,
        start: IVec2,
        destination: IVec2,
    ) -> Option<(MoveOrder, bool)> {
        if let Some(order) = self.route_known(player, start, destination, &|_| true) {
            return Some((order, false));
        }
        let view = &self.fog_navigation.as_ref()?.views[usize::from(player.0)];
        let goal = tile_of(destination);
        let size = self.terrain.size();
        if !inside(goal, size)
            || (view.tile(goal) != TileVisibility::Unexplored && !self.traversable(goal))
        {
            return None;
        }
        let start_tile = tile_of(start);
        let known_frontier = self.closest_frontier(view, start_tile, goal)?;
        let waypoint = known_frontier * TILE + IVec2::splat(TILE / 2);
        self.route_known(player, start, waypoint, &|_| true)
            .map(|order| (order, true))
    }

    fn closest_frontier(&self, view: &FogVisibility, start: IVec2, goal: IVec2) -> Option<IVec2> {
        let size = self.terrain.size();
        let index = |tile: IVec2| usize::try_from(tile.y * size.x + tile.x).expect("inside map");
        let mut seen = vec![false; self.terrain.tiles().len()];
        let mut queue = VecDeque::from([start]);
        let mut best = None;
        seen[index(start)] = true;
        while let Some(tile) = queue.pop_front() {
            let mut frontier = false;
            for neighbor in NEIGHBORS.into_iter().map(|offset| tile + offset) {
                if !inside(neighbor, size) {
                    continue;
                }
                if view.tile(neighbor) == TileVisibility::Unexplored {
                    frontier = true;
                } else if !std::mem::replace(&mut seen[index(neighbor)], true)
                    && self.traversable(neighbor)
                {
                    queue.push_back(neighbor);
                }
            }
            if frontier && tile != start {
                let distance = tile.x.abs_diff(goal.x).pow(2) + tile.y.abs_diff(goal.y).pow(2);
                let key = (distance, tile.y, tile.x);
                if best.is_none_or(|(current, _)| key < current) {
                    best = Some((key, tile));
                }
            }
        }
        best.map(|(_, tile)| tile)
    }

    fn traversable(&self, tile: IVec2) -> bool {
        self.terrain
            .get(tile)
            .is_some_and(|id| self.rules.terrain(id).speed_modifier > 0)
    }

    pub(super) fn remember_destination(
        &mut self,
        unit: UnitId,
        destination: IVec2,
        group: Option<MoveGroupId>,
    ) {
        if let Some(navigation) = &mut self.fog_navigation {
            navigation.destinations.insert(unit, (destination, group));
        }
    }

    pub(super) fn cancel_exploration(&mut self, unit: UnitId) {
        if let Some(navigation) = &mut self.fog_navigation {
            navigation.destinations.remove(&unit);
        }
    }

    pub(super) fn continue_exploration(&mut self) {
        let Some(navigation) = &self.fog_navigation else {
            return;
        };
        let ready: Vec<_> = navigation
            .destinations
            .iter()
            .filter(|&(unit, _)| !self.move_orders.contains_key(unit))
            .map(|(&unit, &(destination, group))| (unit, destination, group))
            .collect();
        for (id, destination, group) in ready {
            let next = self
                .units
                .get(&id)
                .and_then(|unit| self.plan_exploration(unit.owner, unit.pos, destination));
            if let Some((mut order, continuing)) = next {
                order.group = group;
                self.move_orders.insert(id, order);
                if continuing {
                    continue;
                }
            }
            self.cancel_exploration(id);
        }
    }
}

fn inside(tile: IVec2, size: IVec2) -> bool {
    tile.cmpge(IVec2::ZERO).all() && tile.cmplt(size).all()
}
