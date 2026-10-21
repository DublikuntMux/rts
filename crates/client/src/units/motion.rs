//! Interpolate travel separately from visual packing so tiles never change travel speed.

use bevy::prelude::*;

use super::layout::{Placement, PlacementMotion};

pub(super) struct UnitMotion {
    tick: u32,
    previous: Vec2,
    current: Vec2,
    sampled_at: f64,
    packing: PlacementMotion,
}

impl UnitMotion {
    pub fn new(position: Vec2, placement: Placement, tick: u32, sampled_at: f64) -> Self {
        Self {
            tick,
            previous: position,
            current: position,
            sampled_at,
            packing: PlacementMotion::new(Placement {
                position: placement.position - position,
                scale: placement.scale,
            }),
        }
    }

    pub const fn observe(&mut self, position: Vec2, tick: u32, sampled_at: f64) {
        if self.tick != tick {
            self.previous = self.current;
            self.current = position;
            self.tick = tick;
            self.sampled_at = sampled_at;
        }
    }

    pub fn draw(
        &mut self,
        target: Placement,
        now: f64,
        tick_seconds: f64,
        delta_seconds: f32,
    ) -> Placement {
        // The sample time changes only when the simulation advances. A stalled
        // session reaches its last position once, even as the fixed clock keeps ticking.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "fraction is clamped to 0..1"
        )]
        let alpha = ((now - self.sampled_at) / tick_seconds).clamp(0.0, 1.0) as f32;
        let packing = self.packing.advance(
            Placement {
                position: target.position - self.current,
                scale: target.scale,
            },
            delta_seconds,
        );
        Placement {
            position: self.previous.lerp(self.current, alpha) + packing.position,
            scale: packing.scale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_simulation_finishes_the_last_segment_without_rewinding() {
        let initial = Placement {
            position: Vec2::ZERO,
            scale: 1.0,
        };
        let mut motion = UnitMotion::new(Vec2::ZERO, initial, 0, 0.0);
        let target = Placement {
            position: Vec2::new(16.0, 0.0),
            scale: 1.0,
        };
        motion.observe(target.position, 1, 0.05);
        for (now, x) in [
            (0.05, 0.0),
            (0.075, 8.0),
            (0.1, 16.0),
            (0.15, 16.0),
            (0.2, 16.0),
        ] {
            let drawn = motion.draw(target, now, 0.05, 0.025);
            assert!((drawn.position.x - x).abs() < 0.0001);
        }
        let next = Placement {
            position: Vec2::new(32.0, 0.0),
            scale: 1.0,
        };
        motion.observe(next.position, 2, 0.2);
        assert_eq!(motion.draw(next, 0.2, 0.05, 0.0).position, target.position);
        assert!((motion.draw(next, 0.225, 0.05, 0.025).position.x - 24.0).abs() < 0.0001);
    }
}
