//! Tile-local presentation bounds; never part of simulation occupancy.

use std::collections::BTreeMap;

use bevy::{platform::collections::HashMap, prelude::*};
use rts_base::{IVec2 as SimPos, UnitDef, UnitId, tile_of};

use crate::map::{TILE_PX, to_world, to_world_distance};

const TILE_PADDING: f32 = 4.0;
const UNIT_GAP: f32 = 4.0;
/// Crowded groups may fill this multiple of a tile's width. Stacked units stay large
/// and spill past the tile edge instead of shrinking into it.
const GROUP_AREA: f32 = 1.5;
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

/// Width of the area a packed tile group may fill, in world pixels.
#[expect(
    clippy::cast_precision_loss,
    reason = "the tile size is a small power of two"
)]
pub(super) fn group_area() -> f32 {
    (TILE_PX as f32).mul_add(GROUP_AREA, -2.0 * TILE_PADDING)
}

/// Pack sprite bounds in ID order, retaining relative unit sizes.
/// A common fit scale keeps the grid inside the group area, which may exceed the
/// tile so even a full tile of units stays close to its unshared size. The grid is
/// centered on the members' mean bounds center, which is the point a group was sent
/// to; it is never pulled toward the tile center, so arriving never shifts the group
/// and a lone unit stands exactly on the ordered point.
#[expect(
    clippy::cast_precision_loss,
    reason = "tile capacity bounds each group to at most u16::MAX units"
)]
pub(super) fn layout_units(
    units: impl Iterator<Item = (UnitId, SimPos, UnitBounds)>,
) -> HashMap<UnitId, Placement> {
    let mut tiles = BTreeMap::<(i32, i32), Vec<_>>::new();
    for (id, position, bounds) in units {
        let tile = tile_of(position);
        tiles
            .entry((tile.x, tile.y))
            .or_default()
            .push((id, to_world(position), bounds));
    }
    let mut placements = HashMap::default();
    for (_, mut group) in tiles {
        group.sort_unstable_by_key(|&(id, _, _)| id);
        let count = group.len();
        let mut columns = count.isqrt() + usize::from(count.isqrt().pow(2) < count);
        let cell = group
            .iter()
            .map(|unit| unit.2.size)
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
        let available = group_area();
        let scale = (available / bounds.max_element()).min(1.0);
        // Sum local offsets so large world coordinates do not accumulate rounding
        // error and pull a crowded group away from its ordered point.
        let origin = group[0].1;
        let center = origin
            + group
                .iter()
                .map(|unit| unit.1 - origin + unit.2.offset * scale)
                .sum::<Vec2>()
                / count as f32;
        let stride = (cell + Vec2::splat(UNIT_GAP)) * scale;
        for (index, (id, _, unit_bounds)) in group.into_iter().enumerate() {
            let row = index / columns;
            let column = index % columns;
            // Center the last row as well when the grid is incomplete.
            let row_columns = columns.min(count - row * columns);
            let offset = Vec2::new(
                column as f32 - (row_columns - 1) as f32 / 2.0,
                (rows - 1) as f32 / 2.0 - row as f32,
            ) * stride;
            placements.insert(
                id,
                Placement {
                    position: center + offset - unit_bounds.offset * scale,
                    scale,
                },
            );
        }
    }
    placements
}

#[cfg(test)]
mod tests {
    use rts_base::TILE;

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
        let min = units
            .iter()
            .fold(Vec2::splat(f32::INFINITY), |min, &(id, _, bounds)| {
                min.min(
                    bounds
                        .world_rect(&crate::units::placement_transform(layout[&id]))
                        .min,
                )
            });
        let max = units
            .iter()
            .fold(Vec2::splat(f32::NEG_INFINITY), |max, &(id, _, bounds)| {
                max.max(
                    bounds
                        .world_rect(&crate::units::placement_transform(layout[&id]))
                        .max,
                )
            });
        assert!((max - min).max_element() <= group_area() + 0.01);
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
    fn crowded_groups_keep_unshared_size_and_spill_past_the_tile() {
        let pos = SimPos::splat(TILE / 2);
        let workers: Vec<_> = (0..4).map(|id| (UnitId(id), pos, circle(38.375))).collect();
        let layout = layout_units(workers.iter().copied());
        let tile = Rect::from_center_size(to_world(pos), Vec2::splat(to_world(SimPos::X * TILE).x));
        let rects: Vec<_> = workers
            .iter()
            .map(|&(id, _, footprint)| {
                let placement = layout[&id];
                assert_eq!(
                    placement.scale, 1.0,
                    "a full tile of workers stays full size"
                );
                footprint.world_rect(&crate::units::placement_transform(placement))
            })
            .collect();
        let left = rects
            .iter()
            .fold(f32::INFINITY, |left, rect| left.min(rect.min.x));
        let right = rects
            .iter()
            .fold(f32::NEG_INFINITY, |right, rect| right.max(rect.max.x));
        assert!(
            right - left > tile.width(),
            "crowded sprites overlap tile edges"
        );
        assert_packed(&workers);

        // Two soldiers at full size would cover more than the tile, so they shrink
        // only slightly instead of down to the old packed size.
        let soldiers: Vec<_> = (0..2).map(|id| (UnitId(id), pos, circle(51.125))).collect();
        assert_packed(&soldiers);
        for placement in layout_units(soldiers.iter().copied()).values() {
            assert!(
                placement.scale > 0.85,
                "two soldiers shrink to {}",
                placement.scale
            );
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
    fn group_offsets_stay_constant_across_centers_and_boundaries() {
        let group = |position| (0..4).map(move |id| (UnitId(id), position, circle(38.375)));
        let start = SimPos::splat(TILE / 2);
        let reference = layout_units(group(start));
        for step in 0..=80 {
            let position = SimPos::new(step * TILE / 20, TILE / 2);
            let placements = layout_units(group(position));
            for id in reference.keys() {
                let expected_offset = reference[id].position - to_world(start);
                let actual_offset = placements[id].position - to_world(position);
                assert!(actual_offset.abs_diff_eq(expected_offset, 0.0001));
                assert_eq!(placements[id].scale, reference[id].scale);
            }
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
    fn stopped_groups_stay_on_the_ordered_point_near_tile_edges() {
        // Four workers ordered to a point near the right edge of their tile.
        let target = SimPos::new(TILE - 1, TILE / 2);
        let units: Vec<_> = (0..4)
            .map(|id| (UnitId(id), target, circle(38.375)))
            .collect();
        let layout = layout_units(units.iter().copied());
        let center = layout
            .values()
            .map(|placement| placement.position)
            .sum::<Vec2>()
            / 4.0;
        assert_eq!(center, to_world(target));
        assert!(
            center.distance(to_world(SimPos::splat(TILE / 2))) > 60.0,
            "the group slid toward the tile center"
        );
        assert_packed(&units);
    }

    #[test]
    fn crowded_group_anchor_does_not_drift_at_large_world_coordinates() {
        let target = SimPos::new(rts_base::MAX_MAP_SIDE * TILE - 575, TILE / 2);
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
