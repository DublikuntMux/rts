//! Tile-local presentation bounds; never part of simulation occupancy.

use bevy::{platform::collections::HashMap, prelude::*};
use rts_base::{IVec2 as SimPos, TILE, UnitDef, UnitId, tile_of};

use crate::map::{TILE_PX, to_world, to_world_distance};

const TILE_PADDING: f32 = 4.0;
const UNIT_GAP: f32 = 4.0;
// Ease only packing changes; simulation travel is interpolated separately.
const MOTION_RESPONSE_SECONDS: f32 = 0.1;
const POSITION_SETTLE_DISTANCE: f32 = 0.01;
const SCALE_SETTLE_DISTANCE: f32 = 0.0001;

/// Axis-aligned sprite footprint, relative to the visual root in client world space.
#[derive(Clone, Copy, Debug)]
pub struct UnitBounds {
    pub offset: Vec2,
    pub size: Vec2,
}

impl UnitBounds {
    pub fn from_definition(definition: &UnitDef) -> Self {
        definition.visual_bounds.map_or_else(
            || Self {
                offset: Vec2::ZERO,
                size: Vec2::splat(to_world_distance(definition.radius) * 2.0),
            },
            |bounds| Self {
                offset: to_world(bounds.offset),
                size: to_world(bounds.size).abs(),
            },
        )
    }

    /// Largest size with the image's aspect ratio that fits the bounds, centered on
    /// them. Images that have not loaded yet fill the bounds.
    pub fn sprite_size(self, image: Option<Vec2>) -> Vec2 {
        match image {
            Some(image) if image.min_element() > 0.0 => image * (self.size / image).min_element(),
            _ => self.size,
        }
    }

    pub fn world_rect(self, transform: &Transform) -> Rect {
        let scale = transform.scale.truncate();
        Rect::from_center_size(
            transform.translation.truncate() + self.offset * scale,
            self.size * scale,
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Placement {
    pub position: Vec2,
    pub scale: f32,
}

impl Placement {
    pub fn interpolate(self, current: Self, alpha: f32) -> Self {
        let alpha = alpha.clamp(0.0, 1.0);
        Self {
            position: self.position.lerp(current.position, alpha),
            scale: self.scale.lerp(current.scale, alpha),
        }
    }
}

/// Persistent presentation state for one visual, independent of the fixed clock.
///
/// Smooth packing offsets and scale, independently of interpolated simulation travel.
/// Changing group membership continues from the last drawn offset. The exponential
/// response has no overshoot and uses elapsed seconds rather than a frame count.
#[derive(Clone, Copy, Debug)]
pub(super) struct PlacementMotion {
    placement: Placement,
}

impl PlacementMotion {
    /// New visuals appear at their packed position rather than arriving from zero.
    pub const fn new(placement: Placement) -> Self {
        Self { placement }
    }

    pub fn advance(&mut self, target: Placement, delta_seconds: f32) -> Placement {
        if delta_seconds <= 0.0 {
            return self.placement;
        }
        let alpha = -(-delta_seconds / MOTION_RESPONSE_SECONDS).exp_m1();
        self.placement = self.placement.interpolate(target, alpha);
        // Finish exactly so an unchanged final layout remains packed and stable.
        if self.placement.position.distance_squared(target.position)
            <= POSITION_SETTLE_DISTANCE * POSITION_SETTLE_DISTANCE
        {
            self.placement.position = target.position;
        }
        if (self.placement.scale - target.scale).abs() <= SCALE_SETTLE_DISTANCE {
            self.placement.scale = target.scale;
        }
        self.placement
    }
}

/// Width of a tile's interior in world pixels.
#[expect(
    clippy::cast_precision_loss,
    reason = "the tile size is a small power of two"
)]
pub(super) fn group_area() -> f32 {
    2.0_f32.mul_add(-TILE_PADDING, TILE_PX as f32)
}

/// Reusable storage for packing units. Sorting a flat buffer avoids rebuilding a tree
/// and allocating a fresh vector for every occupied tile on every rendered frame.
#[derive(Default)]
pub(super) struct UnitLayout {
    units: Vec<LayoutUnit>,
    placements: HashMap<UnitId, Placement>,
}

struct LayoutUnit {
    tile: (i32, i32),
    id: UnitId,
    position: Vec2,
    bounds: UnitBounds,
}

impl UnitLayout {
    pub fn placement(&self, id: UnitId) -> Placement {
        self.placements[&id]
    }

    /// All occupants pack in stable ID order within each tile.
    #[expect(
        clippy::cast_precision_loss,
        reason = "tile capacity bounds each group to at most u16::MAX units"
    )]
    pub fn update(&mut self, units: impl Iterator<Item = (UnitId, SimPos, UnitBounds)>) {
        self.units.clear();
        self.units.extend(units.map(|(id, position, bounds)| {
            let tile = tile_of(position);
            LayoutUnit {
                tile: (tile.x, tile.y),
                id,
                position: to_world(position),
                bounds,
            }
        }));
        self.units.sort_unstable_by_key(|unit| (unit.tile, unit.id));
        self.placements.clear();
        for group in self.units.chunk_by(|a, b| a.tile == b.tile) {
            // The group occupies a contiguous portion of the retained scratch buffer.
            let count = group.len();
            let mut columns = count.isqrt() + usize::from(count.isqrt().pow(2) < count);
            let cell = group
                .iter()
                .map(|unit| unit.bounds.size)
                .fold(Vec2::ZERO, Vec2::max);
            let grid_bounds = |columns: usize| {
                let size = Vec2::new(columns as f32, count.div_ceil(columns) as f32);
                size * cell + (size - Vec2::ONE) * UNIT_GAP
            };
            let mut bounds = grid_bounds(columns);
            // Rectangular sprites may fit better in a vertical stack than a square grid.
            for candidate in 1..=count {
                let candidate_bounds = grid_bounds(candidate);
                if candidate_bounds.max_element() < bounds.max_element() {
                    columns = candidate;
                    bounds = candidate_bounds;
                }
            }
            let rows = count.div_ceil(columns);
            let available = if count == 1 {
                TILE_PX as f32
            } else {
                group_area()
            };
            let scale = (available / bounds.max_element()).min(1.0);
            // Sum local offsets so large world coordinates do not accumulate rounding
            // error and pull a crowded group away from its ordered point.
            let origin = group[0].position;
            let preferred = origin
                + group
                    .iter()
                    .map(|unit| unit.position - origin + unit.bounds.offset * scale)
                    .sum::<Vec2>()
                    / count as f32;
            let center = if count == 1 {
                preferred
            } else {
                let (x, y) = group[0].tile;
                let tile_center = to_world(SimPos::new(x * TILE + TILE / 2, y * TILE + TILE / 2));
                let travel = ((Vec2::splat(available) - bounds * scale) / 2.0).max(Vec2::ZERO);
                preferred.clamp(tile_center - travel, tile_center + travel)
            };
            let stride = (cell + Vec2::splat(UNIT_GAP)) * scale;
            for (index, unit) in group.iter().enumerate() {
                let row = index / columns;
                let column = index % columns;
                // Center the last row as well when the grid is incomplete.
                let row_columns = columns.min(count - row * columns);
                let offset = Vec2::new(
                    column as f32 - (row_columns - 1) as f32 / 2.0,
                    (rows - 1) as f32 / 2.0 - row as f32,
                ) * stride;
                self.placements.insert(
                    unit.id,
                    Placement {
                        position: center + offset - unit.bounds.offset * scale,
                        scale,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
fn layout_units(
    units: impl Iterator<Item = (UnitId, SimPos, UnitBounds)>,
) -> HashMap<UnitId, Placement> {
    let mut layout = UnitLayout::default();
    layout.update(units);
    layout.placements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(radius: f32) -> UnitBounds {
        UnitBounds {
            offset: Vec2::ZERO,
            size: Vec2::splat(radius * 2.0),
        }
    }

    fn assert_packed(units: &[(UnitId, SimPos, UnitBounds)]) {
        let layout = layout_units(units.iter().copied());
        for &(id, _, footprint) in units {
            let placement = layout[&id];
            let bounds = footprint.world_rect(&crate::units::placement_transform(placement));
            assert!(placement.scale > 0.0 && placement.scale <= 1.0);
            for &(other, _, other_footprint) in units.iter().filter(|unit| unit.0 > id) {
                let other =
                    other_footprint.world_rect(&crate::units::placement_transform(layout[&other]));
                assert!(
                    bounds.max.x <= other.min.x
                        || other.max.x <= bounds.min.x
                        || bounds.max.y <= other.min.y
                        || other.max.y <= bounds.min.y
                );
            }
        }
        if units.len() > 1 {
            let tile = tile_of(units[0].1);
            let center = to_world(tile * TILE + SimPos::splat(TILE / 2));
            let inset = Rect::from_center_size(center, Vec2::splat(group_area() + 0.01));
            for &(id, _, footprint) in units {
                let bounds = footprint.world_rect(&crate::units::placement_transform(layout[&id]));
                assert!(
                    inset.contains(bounds.min) && inset.contains(bounds.max),
                    "{bounds:?} outside {inset:?}"
                );
            }
        }
    }

    #[test]
    fn retained_layout_reuses_storage_and_packs_occupants_together() {
        let position = SimPos::splat(TILE / 2);
        let bounds = circle(38.375);
        let units = [
            (UnitId(0), position, bounds),
            (UnitId(1), position, bounds),
            (UnitId(2), position, bounds),
            (UnitId(3), position, bounds),
            (UnitId(4), position + SimPos::X * TILE, bounds),
        ];
        let mut layout = UnitLayout::default();
        layout.update(units.into_iter());
        let expected = layout.placements.clone();
        assert_packed(&units[..4]);
        assert_eq!(layout.placement(UnitId(4)).position, to_world(units[4].1));
        let scratch = layout.units.as_ptr();
        let capacity = layout.placements.capacity();
        layout.update(units.into_iter().rev());
        assert_eq!(layout.units.as_ptr(), scratch);
        assert_eq!(layout.placements.capacity(), capacity);
        for (id, placement) in expected {
            assert_eq!(layout.placement(id).position, placement.position);
            assert_eq!(layout.placement(id).scale, placement.scale);
        }
        layout.update(units[..1].iter().copied());
        assert_eq!(layout.placements.len(), 1);
        assert_eq!(layout.placement(UnitId(0)).position, to_world(position));
        layout.update(std::iter::empty());
        assert!(layout.placements.is_empty());
    }

    #[test]
    fn crowded_and_mixed_size_groups_fit_at_centers_and_tile_edges() {
        for count in 1..=12 {
            for pos in [
                SimPos::splat(TILE / 2),
                SimPos::ZERO,
                SimPos::splat(TILE - 1),
                SimPos::new(3 * TILE, 2 * TILE - 1),
            ] {
                let units: Vec<_> = (0..count)
                    .map(|id| {
                        (
                            UnitId(id),
                            pos,
                            circle(if id % 2 == 0 { 38.375 } else { 51.125 }),
                        )
                    })
                    .collect();
                assert_packed(&units);
            }
        }
    }

    #[test]
    fn full_tiles_pack_four_workers_or_two_squads() {
        let pos = SimPos::splat(TILE / 2);
        for (count, radius) in [(4, 38.375), (2, 57.625)] {
            let units: Vec<_> = (0..count)
                .map(|id| (UnitId(id), pos, circle(radius)))
                .collect();
            assert_packed(&units);
            for placement in layout_units(units.iter().copied()).values() {
                assert!(placement.scale < 1.0);
            }
        }
    }

    #[test]
    fn layout_is_stable_by_id_and_keeps_tiles_independent() {
        let units = [
            (UnitId(0), SimPos::splat(TILE / 2), circle(38.375)),
            (UnitId(1), SimPos::splat(TILE / 2), circle(38.375)),
            (
                UnitId(2),
                SimPos::new(3 * TILE / 2, TILE / 2),
                circle(51.125),
            ),
        ];
        let forward = layout_units(units.into_iter());
        let reverse = layout_units(units.into_iter().rev());
        for (id, _, _) in units {
            assert_eq!(forward[&id].position, reverse[&id].position);
            assert_eq!(forward[&id].scale, reverse[&id].scale);
        }
        assert_eq!(forward[&UnitId(2)].position, to_world(units[2].1));
        assert_eq!(forward[&UnitId(2)].scale, 1.0);
        let without_neighbor = layout_units(units[..2].iter().copied());
        assert_eq!(
            forward[&UnitId(0)].position,
            without_neighbor[&UnitId(0)].position
        );
    }

    #[test]
    fn oversized_singleton_fits_custom_rules() {
        let units = [(UnitId(0), SimPos::ZERO, circle(256.0))];
        assert_packed(&units);
        assert!(layout_units(units.into_iter())[&UnitId(0)].scale < 1.0);
    }

    #[test]
    fn groups_retain_spacing_when_they_cross_tile_boundaries() {
        let group = |position| (0..4).map(move |id| (UnitId(id), position, circle(38.375)));
        let start = SimPos::splat(TILE / 2);
        let reference = layout_units(group(start));
        for step in 0..=80 {
            let units: Vec<_> = group(SimPos::new(step * TILE / 20, TILE / 2)).collect();
            let placements = layout_units(units.iter().copied());
            let anchor = UnitId(0);
            for id in reference.keys() {
                let expected_offset = reference[id].position - reference[&anchor].position;
                let actual_offset = placements[id].position - placements[&anchor].position;
                assert!(actual_offset.abs_diff_eq(expected_offset, 0.0001));
                assert_eq!(placements[id].scale, reference[id].scale);
            }
            assert_packed(&units);
        }
    }

    #[test]
    fn a_lone_unit_stands_exactly_on_the_ordered_point_across_tile_edges() {
        for position in [
            SimPos::ZERO,
            SimPos::new(1, TILE - 1),
            SimPos::new(TILE - 1, TILE / 2),
            SimPos::new(TILE + 1, TILE / 2),
            SimPos::new(3 * TILE - 1, 2 * TILE),
        ] {
            let placement =
                layout_units([(UnitId(0), position, circle(51.125))].into_iter())[&UnitId(0)];
            assert_eq!(placement.position, to_world(position));
            assert_eq!(placement.scale, 1.0);
        }
    }

    #[test]
    fn crowded_group_stays_as_close_to_the_order_as_its_footprint_allows() {
        let pos = SimPos::new(TILE - 1, TILE / 2);
        let units = [
            (UnitId(0), pos, circle(10.0)),
            (UnitId(1), pos, circle(10.0)),
        ];
        let layout = layout_units(units.into_iter());
        let center = (layout[&UnitId(0)].position + layout[&UnitId(1)].position) / 2.0;
        assert_eq!(center.x, 128.0 - TILE_PADDING - 22.0);
        assert_eq!(center.y, to_world(pos).y);
        assert_packed(&units);
    }

    #[test]
    fn crowded_group_anchor_does_not_drift_at_large_world_coordinates() {
        let target = SimPos::new(rts_base::MAX_MAP_SIDE * TILE - TILE / 2, TILE / 2);
        let footprint = circle(38.375);
        // Custom rules may allow up to u16::MAX size-one units in a tile.
        for count in [1024, u32::from(u16::MAX)] {
            let layout = layout_units((0..count).map(|id| (UnitId(id), target, footprint)));
            let min = layout
                .values()
                .fold(Vec2::splat(f32::INFINITY), |min, &placement| {
                    min.min(
                        footprint
                            .world_rect(&crate::units::placement_transform(placement))
                            .min,
                    )
                });
            let max = layout
                .values()
                .fold(Vec2::splat(f32::NEG_INFINITY), |max, &placement| {
                    max.max(
                        footprint
                            .world_rect(&crate::units::placement_transform(placement))
                            .max,
                    )
                });
            assert!(((min + max) / 2.0).abs_diff_eq(to_world(target), 0.01));
        }
    }

    #[test]
    fn group_changes_retarget_position_and_scale_from_the_drawn_placement() {
        let id = UnitId(0);
        let crowded: Vec<_> = (0..4)
            .map(|id| (UnitId(id), SimPos::splat(TILE / 2), circle(51.125)))
            .collect();
        let packed = layout_units(crowded.iter().copied())[&id];
        let alone =
            layout_units([(id, SimPos::new(3 * TILE / 2, TILE / 2), circle(51.125))].into_iter())
                [&id];
        assert!(packed.scale < alone.scale);
        let mut motion = PlacementMotion::new(packed);
        let leaving = motion.advance(alone, 1.0 / 60.0);
        assert!(leaving.position.x > packed.position.x);
        assert!(leaving.position.x < alone.position.x);
        assert!(leaving.scale > packed.scale && leaving.scale < alone.scale);

        // Rejoin before the old transition completes. Neither channel snaps to
        // the old endpoint or restarts from the previous simulation snapshot.
        let retargeted = motion.advance(packed, 0.0);
        assert_eq!(retargeted.position, leaving.position);
        assert_eq!(retargeted.scale, leaving.scale);
        let joining = motion.advance(packed, 1.0 / 60.0);
        assert!(joining.position.x > packed.position.x);
        assert!(joining.position.x < leaving.position.x);
        assert!(joining.scale > packed.scale && joining.scale < leaving.scale);

        for _ in 0..240 {
            motion.advance(packed, 1.0 / 120.0);
        }
        let settled = motion.advance(packed, 1.0 / 120.0);
        assert_eq!(settled.position, packed.position);
        assert_eq!(settled.scale, packed.scale);
        assert_packed(&crowded);
    }

    #[test]
    fn motion_response_depends_on_elapsed_time_instead_of_frame_count() {
        let start = Placement {
            position: Vec2::new(-40.0, 30.0),
            scale: 0.4,
        };
        let target = Placement {
            position: Vec2::new(150.0, -80.0),
            scale: 1.0,
        };
        let expected = start.interpolate(target, 1.0 - (-5.0_f32).exp());
        for frames_per_second in [30_u16, 60, 120, 144] {
            let mut motion = PlacementMotion::new(start);
            for _ in 0..frames_per_second / 2 {
                motion.advance(target, 1.0 / f32::from(frames_per_second));
            }
            let drawn = motion.advance(target, 0.0);
            assert!(drawn.position.distance(expected.position) < 0.0001);
            assert!((drawn.scale - expected.scale).abs() < 0.00001);
        }
        // A delayed frame produces the same response as many short frames.
        let mut motion = PlacementMotion::new(start);
        let drawn = motion.advance(target, 0.5);
        assert!(drawn.position.distance(expected.position) < 0.0001);
    }

    #[test]
    fn stopped_motion_settles_exactly_and_remains_at_rest() {
        let target = Placement {
            position: Vec2::new(128.0, -128.0),
            scale: 0.75,
        };
        let mut motion = PlacementMotion::new(Placement {
            position: Vec2::ZERO,
            scale: 1.0,
        });
        for _ in 0..120 {
            let before = motion.advance(target, 0.0);
            let drawn = motion.advance(target, 1.0 / 60.0);
            assert!(
                drawn.position.distance(target.position)
                    <= before.position.distance(target.position)
            );
            assert!(drawn.scale >= target.scale && drawn.scale <= before.scale);
        }
        for delta in [0.0, 1.0 / 144.0, 1.0 / 30.0, 1.0] {
            let drawn = motion.advance(target, delta);
            assert_eq!(drawn.position, target.position);
            assert_eq!(drawn.scale, target.scale);
        }
    }

    #[test]
    fn interpolation_preserves_separation_for_unchanged_groups() {
        let start = [
            (UnitId(0), SimPos::ZERO, circle(38.375)),
            (UnitId(1), SimPos::ZERO, circle(51.125)),
        ];
        let end = start.map(|(id, _, radius)| (id, SimPos::splat(TILE - 1), radius));
        let previous = layout_units(start.into_iter());
        let current = layout_units(end.into_iter());
        for alpha in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let a = previous[&UnitId(0)].interpolate(current[&UnitId(0)], alpha);
            let b = previous[&UnitId(1)].interpolate(current[&UnitId(1)], alpha);
            assert!(
                (a.position.x - b.position.x).abs()
                    >= 51.125_f32.mul_add(b.scale, 38.375 * a.scale)
            );
        }
    }
    #[test]
    fn rectangular_bounds_with_different_offsets_pack_at_tile_edges() {
        for pos in [
            SimPos::ZERO,
            SimPos::splat(TILE / 2),
            SimPos::splat(TILE - 1),
        ] {
            let units = [
                (
                    UnitId(0),
                    pos,
                    UnitBounds {
                        offset: Vec2::new(28.0, -15.0),
                        size: Vec2::new(96.0, 32.0),
                    },
                ),
                (
                    UnitId(1),
                    pos,
                    UnitBounds {
                        offset: Vec2::new(-12.0, 30.0),
                        size: Vec2::new(32.0, 64.0),
                    },
                ),
                (
                    UnitId(2),
                    pos,
                    UnitBounds {
                        offset: Vec2::new(40.0, -70.0),
                        size: Vec2::new(48.0, 48.0),
                    },
                ),
            ];
            assert_packed(&units);
        }
    }

    #[test]
    fn wide_footprints_stack_without_unnecessary_shrinking() {
        let pos = SimPos::splat(TILE / 2);
        let footprint = UnitBounds {
            offset: Vec2::new(8.0, -4.0),
            size: Vec2::new(100.0, 24.0),
        };
        let units = [(UnitId(0), pos, footprint), (UnitId(1), pos, footprint)];
        let layout = layout_units(units.into_iter());
        assert_packed(&units);
        assert_eq!(layout[&UnitId(0)].scale, 1.0);
        assert_eq!(layout[&UnitId(1)].scale, 1.0);
        assert_eq!(layout[&UnitId(0)].position.x, layout[&UnitId(1)].position.x);
        assert_ne!(layout[&UnitId(0)].position.y, layout[&UnitId(1)].position.y);
    }

    #[test]
    fn offset_moves_the_footprint_without_moving_an_unconstrained_root() {
        let pos = SimPos::splat(TILE / 2);
        let footprint = UnitBounds {
            offset: Vec2::new(8.0, 12.0),
            size: Vec2::new(48.0, 24.0),
        };
        let layout = layout_units([(UnitId(0), pos, footprint)].into_iter());
        let placement = layout[&UnitId(0)];
        assert_eq!(placement.position, to_world(pos));
        assert_eq!(
            footprint
                .world_rect(&crate::units::placement_transform(placement))
                .center(),
            to_world(pos) + footprint.offset
        );
    }
}
