use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{
    IVec2, LoadError,
    coords::{de_speed_modifier, de_tile_pos, de_tiles},
};

/// Fixed-point terrain speed multiplier representing normal speed.
pub const SPEED_SCALE: i32 = 1024;

/// Precision of movement effort per world unit at normal speed.
pub const TRAVEL_SCALE: u64 = 1 << 32;

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
    /// Q10 movement multiplier (written as a decimal); zero blocks movement.
    #[serde(deserialize_with = "de_speed_modifier")]
    pub speed_modifier: i32,
    /// Minimap and distant-zoom color as `#rrggbb`. Only the client reads it.
    pub color: String,
    /// Optional square terrain texture in the asset pack. Only the client reads it.
    #[serde(default)]
    pub texture: Option<String>,
}

impl TerrainDef {
    /// Movement effort per world unit, rounded up; nonpositive modifiers block movement.
    pub(crate) fn travel_cost(&self) -> Option<u64> {
        let speed = u64::try_from(self.speed_modifier).ok()?;
        if speed == 0 {
            return None;
        }
        Some((TRAVEL_SCALE * u64::from(SPEED_SCALE.unsigned_abs())).div_ceil(speed))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDef {
    pub id: String,
    /// Tile capacity consumed by this unit.
    pub size: u16,
    pub max_health: i32,
    /// Nominal drawing radius in world units (written in tiles).
    #[serde(deserialize_with = "de_tiles")]
    pub radius: i32,
    /// Optional drawing bounds. When absent, the client uses a centered diameter from `radius`.
    #[serde(default)]
    pub visual_bounds: Option<VisualBounds>,
    /// Movement speed in world units per second (written in tiles per second).
    #[serde(deserialize_with = "de_tiles")]
    pub speed: i32,
}

/// Drawing bounds in world units, independent of simulation movement and tile capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualBounds {
    /// Center offset from the unit position (written as a decimal tile pair).
    #[serde(deserialize_with = "de_tile_pos")]
    pub offset: IVec2,
    /// Positive width and height (written as a decimal tile pair).
    #[serde(deserialize_with = "de_tile_pos")]
    pub size: IVec2,
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
            if let Some(bounds) = unit.visual_bounds
                && (bounds.size.x <= 0 || bounds.size.y <= 0)
            {
                return Err(LoadError::invalid(format!(
                    "unit `{}` visual_bounds size dimensions must be positive",
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

    const TERRAIN: &str = r##"(id: "grass", symbol: 'g', speed_modifier: 1.0, color: "#00ff00")"##;

    fn rules_with_visual_bounds(field: &str) -> Result<Rules, LoadError> {
        Rules::from_ron(&format!(
            r#"(tile_capacity: 4, terrain: [{TERRAIN}], units: [(id: "worker", size: 1, max_health: 40, radius: 0.25, speed: 1.25, {field})])"#
        ))
    }

    #[test]
    fn visual_bounds_default_to_none_and_accept_explicit_none() {
        for field in ["", "visual_bounds: None"] {
            let rules = rules_with_visual_bounds(field).unwrap();
            assert_eq!(rules.unit(UnitKind(0)).visual_bounds, None);
        }
    }

    #[test]
    fn parses_visual_bounds_as_fixed_point_tile_pairs() {
        for (offset, size, expected_offset, expected_size) in [
            (
                "(0.1, -0.2)",
                "(0.75, 0.5)",
                IVec2::new(102, -205),
                IVec2::new(768, 512),
            ),
            ("(0, 0)", "(1, 2)", IVec2::ZERO, IVec2::new(1024, 2048)),
            (
                "(-0.0001, 5e-324)",
                "(0.0009765625, 0.00048828125)",
                IVec2::ZERO,
                IVec2::ONE,
            ),
            (
                "(-2097151.9990234375, 2097151.9990234375)",
                "(2097151.9990234375, 0.0009765625)",
                IVec2::new(-i32::MAX, i32::MAX),
                IVec2::new(i32::MAX, 1),
            ),
        ] {
            let rules = rules_with_visual_bounds(&format!(
                "visual_bounds: Some((offset: {offset}, size: {size}))"
            ))
            .unwrap();
            assert_eq!(
                rules.unit(UnitKind(0)).visual_bounds,
                Some(VisualBounds {
                    offset: expected_offset,
                    size: expected_size,
                }),
                "offset: {offset}, size: {size}"
            );
        }
    }

    #[test]
    fn visual_bounds_size_dimensions_must_be_positive_after_conversion() {
        for value in ["0", "-1", "0.0001", "-0.0001", "5e-324"] {
            for size in [format!("({value}, 1)"), format!("(1, {value})")] {
                let error = rules_with_visual_bounds(&format!(
                    "visual_bounds: Some((offset: (0, 0), size: {size}))"
                ))
                .unwrap_err();
                assert!(matches!(error, LoadError::Invalid(_)), "{size}: {error}");
                let message = error.to_string();
                assert!(message.contains("worker"), "{message}");
                assert!(message.contains("visual_bounds size"), "{message}");
            }
        }
    }

    #[test]
    fn rejects_out_of_range_and_nonfinite_visual_bounds_components() {
        for value in [
            "2097152.0",
            "-2097152.0",
            "1e20",
            "-1e20",
            "NaN",
            "inf",
            "-inf",
        ] {
            for pair in [format!("({value}, 1)"), format!("(1, {value})")] {
                for (offset, size) in [(pair.as_str(), "(1, 1)"), ("(0, 0)", pair.as_str())] {
                    let error = rules_with_visual_bounds(&format!(
                        "visual_bounds: Some((offset: {offset}, size: {size}))"
                    ))
                    .unwrap_err();
                    assert!(
                        matches!(error, LoadError::Parse(_)),
                        "offset: {offset}, size: {size}: {error}"
                    );
                }
            }
        }
    }

    #[test]
    fn visual_bounds_require_both_fields_and_reject_unknown_fields() {
        for (fields, expected) in [
            ("", "offset"),
            ("offset: (0, 0)", "size"),
            ("size: (1, 1)", "offset"),
            ("offset: (0, 0), size: (1, 1), extra: 1", "extra"),
        ] {
            let error =
                rules_with_visual_bounds(&format!("visual_bounds: Some(({fields}))")).unwrap_err();
            assert!(matches!(error, LoadError::Parse(_)), "{error}");
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn rejects_malformed_visual_bounds_pairs() {
        for pair in ["()", "(1,)", "(1, 2, 3)", "(true, 1)"] {
            for (offset, size) in [(pair, "(1, 1)"), ("(0, 0)", pair)] {
                let error = rules_with_visual_bounds(&format!(
                    "visual_bounds: Some((offset: {offset}, size: {size}))"
                ))
                .unwrap_err();
                assert!(
                    matches!(error, LoadError::Parse(_)),
                    "offset: {offset}, size: {size}: {error}"
                );
            }
        }
    }

    #[test]
    fn parses_terrain_speed_modifiers_as_q10() {
        for (modifier, scaled) in [
            ("0", 0),
            ("0.0", 0),
            ("0.5", SPEED_SCALE / 2),
            ("1", SPEED_SCALE),
            ("1.0", SPEED_SCALE),
            ("2", SPEED_SCALE * 2),
            ("2.0", SPEED_SCALE * 2),
            ("0.0009765625", 1),
            ("2097151.9990234375", i32::MAX),
        ] {
            let terrain = TERRAIN.replace("1.0", modifier);
            let rules = Rules::from_ron(&format!(
                "(tile_capacity: 4, terrain: [{terrain}], units: [])"
            ))
            .unwrap();
            assert_eq!(rules.terrain(TerrainId(0)).speed_modifier, scaled);
        }
    }

    #[test]
    fn rejects_invalid_terrain_speed_modifiers() {
        for modifier in [
            "-1.0",
            "-0.0001",
            "-5e-324",
            "0.0001",
            "5e-324",
            "2097152.0",
            "1e20",
            "NaN",
            "inf",
            "-inf",
            "true",
        ] {
            let terrain = TERRAIN.replace("1.0", modifier);
            assert!(
                Rules::from_ron(&format!(
                    "(tile_capacity: 4, terrain: [{terrain}], units: [])"
                ))
                .is_err(),
                "accepted terrain speed_modifier `{modifier}`"
            );
        }
    }

    #[test]
    fn terrain_speed_modifier_is_required_and_legacy_walkable_is_rejected() {
        for field in [
            "",
            "walkable: true,",
            "walkable: false,",
            "speed_modifier: 1.0, walkable: true,",
        ] {
            assert!(
                Rules::from_ron(&format!(
                    r##"(tile_capacity: 4, terrain: [(id: "grass", symbol: 'g', {field} color: "#00ff00")], units: [])"##
                ))
                .is_err(),
                "accepted terrain fields `{field}`"
            );
        }
    }

    #[test]
    fn terrain_travel_cost_is_integer_and_rounds_up() {
        let mut terrain: TerrainDef = ron::from_str(TERRAIN).unwrap();
        for (speed, cost) in [
            (-1, None),
            (0, None),
            (1, Some(TRAVEL_SCALE * 1024)),
            (SPEED_SCALE / 2, Some(TRAVEL_SCALE * 2)),
            (SPEED_SCALE, Some(TRAVEL_SCALE)),
            (SPEED_SCALE * 3 / 2, Some((TRAVEL_SCALE * 2).div_ceil(3))),
            (SPEED_SCALE * 2, Some(TRAVEL_SCALE / 2)),
            (
                i32::MAX,
                Some((TRAVEL_SCALE * 1024).div_ceil(2_147_483_647)),
            ),
        ] {
            terrain.speed_modifier = speed;
            assert_eq!(terrain.travel_cost(), cost, "speed_modifier {speed}");
        }
    }

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
            r##"(tile_capacity: 4, terrain: [{TERRAIN}, (id: "moss", symbol: 'g', speed_modifier: 1.0, color: "#000000")], units: [])"##
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
