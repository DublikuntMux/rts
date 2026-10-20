//! World coordinates.
//!
//! The simulation uses integer world units so every peer computes identical results.
//! One tile is [`TILE`] units. x grows right and y grows down: tile `(0, 0)` is the top-left.

use glam::IVec2;
use serde::{Deserialize, Deserializer, de::Error};

/// World units per tile.
pub const TILE: i32 = 1024;

/// Converts a distance in tiles, as written in data files, to world units.
///
/// Parsing a decimal and scaling it by a power of two are exact, so every platform gets the same
/// result.
#[expect(
    clippy::float_arithmetic,
    clippy::cast_possible_truncation,
    reason = "exact conversion from data files; the range is checked before the cast"
)]
fn tiles_to_units(tiles: f64) -> Option<i32> {
    let units = (tiles * f64::from(TILE)).round();
    (units.abs() <= f64::from(i32::MAX)).then_some(units as i32)
}

/// Deserializes a distance written in tiles into world units.
pub fn de_tiles<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i32, D::Error> {
    let tiles = f64::deserialize(deserializer)?;
    tiles_to_units(tiles).ok_or_else(|| D::Error::custom(format!("{tiles} tiles is out of range")))
}

/// Deserializes an `(x, y)` position written in tiles into world units.
pub fn de_tile_pos<'de, D: Deserializer<'de>>(deserializer: D) -> Result<IVec2, D::Error> {
    let (x, y) = <(f64, f64)>::deserialize(deserializer)?;
    match (tiles_to_units(x), tiles_to_units(y)) {
        (Some(x), Some(y)) => Ok(IVec2::new(x, y)),
        _ => Err(D::Error::custom(format!(
            "position ({x}, {y}) is out of range"
        ))),
    }
}

/// The tile containing a world position.
#[must_use]
pub const fn tile_of(pos: IVec2) -> IVec2 {
    IVec2::new(pos.x.div_euclid(TILE), pos.y.div_euclid(TILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_tiles_exactly() {
        assert_eq!(tiles_to_units(1.0), Some(TILE));
        assert_eq!(tiles_to_units(0.5), Some(TILE / 2));
        assert_eq!(tiles_to_units(-2.25), Some(-2 * TILE - TILE / 4));
        assert_eq!(tiles_to_units(f64::NAN), None);
        assert_eq!(tiles_to_units(1e12), None);
    }

    #[test]
    fn finds_containing_tile() {
        assert_eq!(tile_of(IVec2::new(0, TILE - 1)), IVec2::new(0, 0));
        assert_eq!(tile_of(IVec2::new(TILE, 3 * TILE + 5)), IVec2::new(1, 3));
        assert_eq!(tile_of(IVec2::new(-1, -1)), IVec2::new(-1, -1));
    }
}
