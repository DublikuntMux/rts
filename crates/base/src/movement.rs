use std::collections::VecDeque;

use crate::{IVec2, Rules, TICK_RATE, Terrain, path::find_path, tile_of};

/// Every field affects subsequent movement and is included in the simulation hash.
#[derive(Debug, Clone, Hash)]
pub struct MoveOrder {
    target: IVec2,
    waypoints: VecDeque<IVec2>,
    segment_start: IVec2,
    segment_progress: u32,
    speed_remainder: u32,
}

impl MoveOrder {
    pub(crate) fn new(
        start: IVec2,
        target: IVec2,
        terrain: &Terrain,
        rules: &Rules,
    ) -> Option<Self> {
        let mut waypoints = if tile_of(start) == tile_of(target) {
            VecDeque::new()
        } else {
            // Visit the starting center before crossing a tile edge. Connecting an arbitrary
            // start directly to a neighboring center could cut through an obstructed corner.
            find_path(terrain, rules, tile_of(start), tile_of(target))?.into()
        };
        waypoints.push_back(target);
        Some(Self {
            target,
            waypoints,
            segment_start: start,
            segment_progress: 0,
            speed_remainder: 0,
        })
    }

    pub(crate) const fn target(&self) -> IVec2 {
        self.target
    }

    /// Returns whether the order remains active. The remainder is local to this order, so a new
    /// move or stop discards it. Segment progress prevents slow diagonal movement from stalling.
    pub(crate) fn advance(&mut self, pos: &mut IVec2, speed: i32) -> bool {
        let numerator = u32::try_from(speed).unwrap() + self.speed_remainder;
        let mut budget = numerator / TICK_RATE;
        self.speed_remainder = numerator % TICK_RATE;
        while let Some(&waypoint) = self.waypoints.front() {
            let dx = i64::from(waypoint.x) - i64::from(self.segment_start.x);
            let dy = i64::from(waypoint.y) - i64::from(self.segment_start.y);
            let squared = dx * dx + dy * dy;
            let floor = squared.isqrt();
            // Round length up so a diagonal segment never consumes less than its true length.
            let length = u32::try_from(floor + i64::from(floor * floor < squared)).unwrap();
            let spent = budget.min(length - self.segment_progress);
            self.segment_progress += spent;
            budget -= spent;
            if self.segment_progress == length {
                *pos = waypoint;
                self.waypoints.pop_front();
                self.segment_start = waypoint;
                self.segment_progress = 0;
            } else {
                // Interpolate from the fixed segment origin instead of repeatedly truncating
                // small per-tick deltas. All endpoints are bounded by MAX_MAP_SIDE * TILE.
                let progress = i64::from(self.segment_progress);
                let length = i64::from(length);
                *pos = IVec2::new(
                    i32::try_from(i64::from(self.segment_start.x) + dx * progress / length)
                        .unwrap(),
                    i32::try_from(i64::from(self.segment_start.y) + dy * progress / length)
                        .unwrap(),
                );
                break;
            }
        }
        !self.waypoints.is_empty()
    }
}
