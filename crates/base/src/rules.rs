use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{LoadError, coords::de_tiles};

/// Game data shared by every match: terrain and unit types (`*.rules.ron`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    tile_capacity: u16,
    terrain: Vec<TerrainDef>,
    units: Vec<UnitDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainDef {
    pub id: String,
    /// Character that stands for this terrain in map files.
    pub symbol: char,
    pub walkable: bool,
    /// Minimap and distant-zoom color as `#rrggbb`. Only the client reads it.
    pub color: String,
    /// Optional square terrain texture in the asset pack. Only the client reads it.
    #[serde(default)]
    pub texture: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDef {
    pub id: String,
    /// Tile capacity consumed by this unit.
    pub size: u16,
    pub max_health: i32,
    /// Collision radius in world units (written in tiles).
    #[serde(deserialize_with = "de_tiles")]
    pub radius: i32,
    /// Movement speed in world units per second (written in tiles per second).
    #[serde(deserialize_with = "de_tiles")]
    pub speed: i32,
}

/// Index of a terrain type in [`Rules`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerrainId(pub u8);

/// Index of a unit type in [`Rules`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitKind(pub u16);

impl Rules {
    /// Parses and validates a rules file.
    ///
    /// # Errors
    ///
    /// Returns an error if the text is not valid RON for this format or the data is inconsistent.
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        let rules: Self = ron::from_str(text)?;
        rules.validate()?;
        Ok(rules)
    }

    fn validate(&self) -> Result<(), LoadError> {
        if self.tile_capacity == 0 {
            return Err(LoadError::invalid("tile_capacity must be positive"));
        }
        if self.terrain.is_empty() {
            return Err(LoadError::invalid("rules define no terrain"));
        }
        if self.terrain.len() > usize::from(u8::MAX) + 1 {
            return Err(LoadError::invalid(
                "rules define more than 256 terrain types",
            ));
        }
        if self.units.len() > usize::from(u16::MAX) + 1 {
            return Err(LoadError::invalid("rules define too many unit types"));
        }
        let mut ids = BTreeSet::new();
        let mut symbols = BTreeSet::new();
        for terrain in &self.terrain {
            if !ids.insert(&terrain.id) {
                return Err(LoadError::invalid(format!(
                    "duplicate terrain `{}`",
                    terrain.id
                )));
            }
            if !symbols.insert(terrain.symbol) {
                return Err(LoadError::invalid(format!(
                    "terrain `{}` reuses symbol `{}`",
                    terrain.id, terrain.symbol
                )));
            }
        }
        let mut ids = BTreeSet::new();
        for unit in &self.units {
            if unit.size == 0 || unit.size > self.tile_capacity {
                return Err(LoadError::invalid(format!(
                    "unit `{}` size must be positive and at most tile_capacity ({})",
                    unit.id, self.tile_capacity
                )));
            }
            if !ids.insert(&unit.id) {
                return Err(LoadError::invalid(format!("duplicate unit `{}`", unit.id)));
            }
            if unit.max_health <= 0 || unit.radius <= 0 || unit.speed <= 0 {
                return Err(LoadError::invalid(format!(
                    "unit `{}` needs positive max_health, radius, and speed",
                    unit.id
                )));
            }
        }
        Ok(())
    }

    /// Maximum combined unit size in one tile.
    #[must_use]
    pub const fn tile_capacity(&self) -> u16 {
        self.tile_capacity
    }

    /// # Panics
    ///
    /// Panics if `id` does not come from these rules.
    #[must_use]
    pub fn terrain(&self, id: TerrainId) -> &TerrainDef {
        &self.terrain[usize::from(id.0)]
    }

    /// Terrain types in [`TerrainId`] order.
    #[must_use]
    pub fn terrain_types(&self) -> &[TerrainDef] {
        &self.terrain
    }

    /// # Panics
    ///
    /// Panics if `kind` does not come from these rules.
    #[must_use]
    pub fn unit(&self, kind: UnitKind) -> &UnitDef {
        &self.units[usize::from(kind.0)]
    }

    pub(crate) fn terrain_by_symbol(&self, symbol: char) -> Option<TerrainId> {
        let index = self.terrain.iter().position(|t| t.symbol == symbol)?;
        u8::try_from(index).ok().map(TerrainId)
    }

    pub(crate) fn unit_kind(&self, id: &str) -> Option<UnitKind> {
        let index = self.units.iter().position(|u| u.id == id)?;
        u16::try_from(index).ok().map(UnitKind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TERRAIN: &str = r##"(id: "grass", symbol: 'g', walkable: true, color: "#00ff00")"##;

    #[test]
    fn parses_tile_distances() {
        let rules = Rules::from_ron(&format!(
            r#"(tile_capacity: 4, terrain: [{TERRAIN}], units: [(id: "worker", size: 1, max_health: 40, radius: 0.25, speed: 1.25)])"#
        ))
        .unwrap();
        let kind = rules.unit_kind("worker").unwrap();
        assert_eq!(rules.unit(kind).radius, crate::TILE / 4);
        assert_eq!(rules.unit(kind).speed, crate::TILE * 5 / 4);
        assert_eq!(rules.terrain_by_symbol('g'), Some(TerrainId(0)));
    }

    #[test]
    fn rejects_duplicate_symbols() {
        let error = Rules::from_ron(&format!(
            r##"(tile_capacity: 4, terrain: [{TERRAIN}, (id: "moss", symbol: 'g', walkable: true, color: "#000000")], units: [])"##
        ))
        .unwrap_err();
        assert!(error.to_string().contains("reuses symbol"), "{error}");
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(
            Rules::from_ron(&format!(
                "(tile_capacity: 4, terrain: [{TERRAIN}], units: [], extra: 1)"
            ))
            .is_err()
        );
    }

    #[test]
    fn speed_is_required_positive_and_in_range() {
        for speed in [
            "",
            ", speed: 0.0",
            ", speed: -1.0",
            ", speed: 0.0001",
            ", speed: 1e20",
            ", speed: NaN",
        ] {
            assert!(Rules::from_ron(&format!(
                r#"(tile_capacity: 4, terrain: [{TERRAIN}], units: [(id: "worker", size: 1, max_health: 40, radius: 0.25{speed})])"#
            )).is_err(), "accepted speed field `{speed}`");
        }
    }

    #[test]
    fn rejects_zero_tile_capacity() {
        let error = Rules::from_ron(&format!(
            "(tile_capacity: 0, terrain: [{TERRAIN}], units: [])"
        ))
        .unwrap_err();
        assert!(matches!(error, LoadError::Invalid(_)), "{error}");
        assert!(error.to_string().contains("tile_capacity"), "{error}");
    }

    #[test]
    fn rejects_zero_and_oversized_units() {
        for size in [0, 5] {
            let error = Rules::from_ron(&format!(
                r#"(tile_capacity: 4, terrain: [{TERRAIN}], units: [(id: "worker", size: {size}, max_health: 40, radius: 0.25, speed: 1.25)])"#
            ))
            .unwrap_err();
            assert!(matches!(error, LoadError::Invalid(_)), "{error}");
            assert!(error.to_string().contains("worker` size"), "{error}");
        }
    }

    #[test]
    fn tile_capacity_and_unit_size_are_required() {
        for (capacity, size, missing) in [
            ("", "size: 1,", "tile_capacity"),
            ("tile_capacity: 4,", "", "size"),
        ] {
            let error = Rules::from_ron(&format!(
                r#"({capacity} terrain: [{TERRAIN}], units: [(id: "worker", {size} max_health: 40, radius: 0.25, speed: 1.25)])"#
            ))
            .unwrap_err();
            assert!(matches!(error, LoadError::Parse(_)), "{error}");
            assert!(error.to_string().contains(missing), "{error}");
        }
    }

    #[test]
    fn rejects_tile_capacity_and_unit_size_outside_u16_range() {
        for (capacity, size) in [("-1", "1"), ("65536", "1"), ("4", "-1"), ("4", "65536")] {
            let error = Rules::from_ron(&format!(
                r#"(tile_capacity: {capacity}, terrain: [{TERRAIN}], units: [(id: "worker", size: {size}, max_health: 40, radius: 0.25, speed: 1.25)])"#
            ))
            .unwrap_err();
            assert!(matches!(error, LoadError::Parse(_)), "{error}");
        }
    }

    #[test]
    fn accepts_unit_sizes_up_to_tile_capacity() {
        for (capacity, size) in [(1, 1), (4, 1), (4, 2), (4, 4), (u16::MAX, u16::MAX)] {
            let result = Rules::from_ron(&format!(
                r#"(tile_capacity: {capacity}, terrain: [{TERRAIN}], units: [(id: "worker", size: {size}, max_health: 40, radius: 0.25, speed: 1.25)])"#
            ));
            assert!(result.is_ok(), "{result:?}");
        }
    }
}
