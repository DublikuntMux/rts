use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{LoadError, coords::de_tiles};

/// Game data shared by every match: terrain and unit types (`*.rules.ron`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
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
    /// Display color as `#rrggbb`. Only the client reads it.
    pub color: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDef {
    pub id: String,
    pub max_health: i32,
    /// Collision radius in world units (written in tiles).
    #[serde(deserialize_with = "de_tiles")]
    pub radius: i32,
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
            if !ids.insert(&unit.id) {
                return Err(LoadError::invalid(format!("duplicate unit `{}`", unit.id)));
            }
            if unit.max_health <= 0 || unit.radius <= 0 {
                return Err(LoadError::invalid(format!(
                    "unit `{}` needs positive max_health and radius",
                    unit.id
                )));
            }
        }
        Ok(())
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
            r#"(terrain: [{TERRAIN}], units: [(id: "worker", max_health: 40, radius: 0.25)])"#
        ))
        .unwrap();
        let kind = rules.unit_kind("worker").unwrap();
        assert_eq!(rules.unit(kind).radius, crate::TILE / 4);
        assert_eq!(rules.terrain_by_symbol('g'), Some(TerrainId(0)));
    }

    #[test]
    fn rejects_duplicate_symbols() {
        let error = Rules::from_ron(&format!(
            r##"(terrain: [{TERRAIN}, (id: "moss", symbol: 'g', walkable: true, color: "#000000")], units: [])"##
        ))
        .unwrap_err();
        assert!(error.to_string().contains("reuses symbol"), "{error}");
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(Rules::from_ron(&format!("(terrain: [{TERRAIN}], units: [], extra: 1)")).is_err());
    }
}
