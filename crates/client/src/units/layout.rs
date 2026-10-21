//! Tile-local presentation bounds; never part of simulation occupancy.

use std::collections::BTreeMap;

use bevy::{platform::collections::HashMap, prelude::*};
use rts_base::{IVec2 as SimPos, TILE, UnitDef, UnitId, tile_of};

use crate::map::{TILE_PX, to_world, to_world_distance};

const TILE_PADDING: f32 = 4.0;
const UNIT_GAP: f32 = 4.0;

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

/// Pack sprite bounds in ID order, retaining relative unit sizes.
/// A common fit scale allows even custom rules with oversized radii to fit.
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
    for ((x, y), mut group) in tiles {
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
        let available = 2.0_f32.mul_add(-TILE_PADDING, TILE_PX as f32);
        let scale = (available / bounds.max_element()).min(1.0);
        let half_bounds = bounds * scale / 2.0;
        let tile_center = to_world(SimPos::new(x, y) * TILE + SimPos::splat(TILE / 2));
        let slack = (Vec2::splat(available / 2.0) - half_bounds).max(Vec2::ZERO);
        let desired = group
            .iter()
            .map(|unit| unit.1 + unit.2.offset * scale)
            .sum::<Vec2>()
            / count as f32;
        let center = desired.clamp(tile_center - slack, tile_center + slack);
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
    use super::*;

    fn circle(radius: f32) -> UnitBounds {
        UnitBounds {
            offset: Vec2::ZERO,
            size: Vec2::splat(radius * 2.0),
        }
    }

    fn assert_packed(units: &[(UnitId, SimPos, UnitBounds)]) {
        let layout = layout_units(units.iter().copied());
        for &(id, pos, footprint) in units {
            let placement = layout[&id];
            let bounds = footprint.world_rect(&crate::units::placement_transform(placement));
            let center = to_world(tile_of(pos) * TILE + SimPos::splat(TILE / 2));
            let tile = Rect::from_center_size(center, Vec2::splat(to_world(SimPos::X * TILE).x));
            assert!(tile.contains(bounds.min), "{id:?} spills out of tile");
            assert!(tile.contains(bounds.max), "{id:?} spills out of tile");
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
        assert_packed(&[(UnitId(0), SimPos::ZERO, circle(256.0))]);
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
