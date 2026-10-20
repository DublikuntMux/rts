use std::collections::VecDeque;

use crate::rules::TRAVEL_SCALE;
use crate::{IVec2, Rules, TICK_RATE, Terrain, path::find_path, tile_of};

/// Every field affects subsequent movement and is included in the simulation hash.
#[derive(Debug, Clone, Hash)]
pub struct MoveOrder {
    target: IVec2,
    waypoints: VecDeque<IVec2>,
    segment_start: IVec2,
    /// Travel effort, including fractional world-unit progress.
    segment_progress: u64,
    speed_remainder: u32,
}

impl MoveOrder {
    pub(crate) fn new(
        start: IVec2,
        target: IVec2,
        terrain: &Terrain,
        rules: &Rules,
    ) -> Option<Self> {
        let waypoints = Self::route(start, target, terrain, rules, &|_| true)?;
        Some(Self {
            target,
            waypoints,
            segment_start: start,
            segment_progress: 0,
            speed_remainder: 0,
        })
    }

    fn route(
        start: IVec2,
        target: IVec2,
        terrain: &Terrain,
        rules: &Rules,
        can_enter: &impl Fn(IVec2) -> bool,
    ) -> Option<VecDeque<IVec2>> {
        let mut waypoints = if tile_of(start) == tile_of(target) {
            VecDeque::new()
        } else {
            // Visit the starting center before crossing a tile edge. Connecting an arbitrary
            // start directly to a neighboring center could cut through an obstructed corner.
            find_path(terrain, rules, tile_of(start), tile_of(target), can_enter)?.into()
        };
        waypoints.push_back(target);
        Some(waypoints)
    }

    pub(crate) const fn target(&self) -> IVec2 {
        self.target
    }

    /// Recompute the route against this tick's living units. Finish an unobstructed segment
    /// already in progress so replanning cannot pull the unit back to its previous center or
    /// discard fractional movement. Only that segment is retained; all later tiles are fresh.
    pub(crate) fn refresh(
        &mut self,
        pos: IVec2,
        terrain: &Terrain,
        rules: &Rules,
        can_enter: &impl Fn(IVec2) -> bool,
    ) -> bool {
        let anchor = self
            .waypoints
            .front()
            .copied()
            .filter(|waypoint| self.segment_progress > 0 && can_enter(tile_of(*waypoint)));
        let Some(mut route) = Self::route(
            anchor.unwrap_or(pos),
            self.target,
            terrain,
            rules,
            can_enter,
        ) else {
            return false;
        };
        if let Some(anchor) = anchor {
            if route.front() != Some(&anchor) {
                route.push_front(anchor);
            }
        } else {
            self.segment_start = pos;
            self.segment_progress = 0;
        }
        self.waypoints = route;
        true
    }

    /// Returns whether the order remains active. The remainder is local to this order, so a new
    /// move or stop discards it. Segment progress prevents slow diagonal movement from stalling.
    pub(crate) fn advance(
        &mut self,
        pos: &mut IVec2,
        speed: i32,
        terrain: &Terrain,
        rules: &Rules,
        can_enter: &impl Fn(IVec2) -> bool,
    ) -> bool {
        let numerator =
            u64::try_from(speed).unwrap() * TRAVEL_SCALE + u64::from(self.speed_remainder);
        let mut budget = numerator / u64::from(TICK_RATE);
        self.speed_remainder = u32::try_from(numerator % u64::from(TICK_RATE)).unwrap();
        while let Some(&waypoint) = self.waypoints.front() {
            // Check every traversed tile, even when a high speed crosses several in one tick.
            if !can_enter(tile_of(waypoint)) {
                break;
            }
            let segment = Segment::new(self.segment_start, waypoint, terrain, rules);
            let cost = segment.cost();
            let spent = budget.min(cost - self.segment_progress);
            self.segment_progress += spent;
            budget -= spent;
            if self.segment_progress == cost {
                *pos = waypoint;
                self.waypoints.pop_front();
                self.segment_start = waypoint;
                self.segment_progress = 0;
            } else {
                *pos = segment.position(self.segment_progress);
                break;
            }
        }
        !self.waypoints.is_empty()
    }
}

/// A route leg stays within a tile, or joins two adjacent tile centers. Split the latter at
/// its boundary so faster terrain cannot carry a unit through slower terrain for free.
struct Segment {
    start: IVec2,
    delta: IVec2,
    length: u64,
    first_length: u64,
    first_cost: u64,
    last_cost: u64,
}

impl Segment {
    fn new(start: IVec2, end: IVec2, terrain: &Terrain, rules: &Rules) -> Self {
        let delta = end - start;
        let squared = i64::from(delta.x).pow(2) + i64::from(delta.y).pow(2);
        let floor = squared.isqrt();
        // Ceil the Euclidean length, as with the original integer movement.
        let length = u64::try_from(floor + i64::from(floor * floor < squared)).unwrap();
        let first_length = if tile_of(start) == tile_of(end) {
            length
        } else {
            debug_assert_eq!(length, u64::try_from(crate::TILE).unwrap());
            length / 2
        };
        let cost = |pos| {
            rules
                .terrain(terrain.get(tile_of(pos)).unwrap())
                .travel_cost()
                .unwrap()
        };
        Self {
            start,
            delta,
            length,
            first_length,
            first_cost: cost(start),
            last_cost: cost(end),
        }
    }

    const fn cost(&self) -> u64 {
        self.first_length * self.first_cost + (self.length - self.first_length) * self.last_cost
    }

    fn position(&self, progress: u64) -> IVec2 {
        let first_effort = self.first_length * self.first_cost;
        let distance = if progress <= first_effort {
            progress / self.first_cost
        } else {
            self.first_length + (progress - first_effort) / self.last_cost
        };
        let distance = i64::try_from(distance).unwrap();
        let length = i64::try_from(self.length).unwrap();
        IVec2::new(
            i32::try_from(i64::from(self.start.x) + i64::from(self.delta.x) * distance / length)
                .unwrap(),
            i32::try_from(i64::from(self.start.y) + i64::from(self.delta.y) * distance / length)
                .unwrap(),
        )
    }
}
