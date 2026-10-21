//! Local presentation settings, independent of simulation rules and match setup.

use bevy::prelude::*;
use serde::{Deserialize, Deserializer, de::Error};

/// A validated `*.graphics.ron` asset, installed as a resource before the lobby opens.
#[derive(Asset, TypePath, Resource, Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graphics {
    /// sRGB colors by player slot; slots wrap when the palette is shorter than the player list.
    #[serde(deserialize_with = "colors")]
    pub player_colors: Vec<Color>,
    pub selection: SelectionStyle,
    pub terrain_detail: TerrainDetail,
}

/// Selection and move feedback. Sizes are screen pixels; duration is in seconds.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionStyle {
    #[serde(deserialize_with = "color")]
    pub color: Color,
    pub padding: f32,
    #[serde(deserialize_with = "color")]
    pub accepted_order_color: Color,
    #[serde(deserialize_with = "color")]
    pub blocked_order_color: Color,
    pub marker_radius: f32,
    pub marker_cross_size: f32,
    pub marker_duration: f64,
}

/// Hysteresis thresholds in screen pixels per tile.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainDetail {
    pub colors_below: f32,
    pub textures_at: f32,
}

impl Graphics {
    /// Parses and validates presentation settings.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed RON, invalid colors, an empty palette, nonfinite sizes,
    /// negative padding, nonpositive marker sizes/duration, or unordered detail thresholds.
    pub fn from_ron(text: &str) -> Result<Self, BevyError> {
        let graphics: Self = ron::from_str(text)?;
        if graphics.player_colors.is_empty() {
            return Err("player_colors must not be empty".into());
        }
        let selection = &graphics.selection;
        if !selection.padding.is_finite() || selection.padding < 0.0 {
            return Err("selection.padding must be finite and nonnegative".into());
        }
        for (field, value) in [
            (
                "selection.marker_radius",
                f64::from(selection.marker_radius),
            ),
            (
                "selection.marker_cross_size",
                f64::from(selection.marker_cross_size),
            ),
            ("selection.marker_duration", selection.marker_duration),
            (
                "terrain_detail.colors_below",
                f64::from(graphics.terrain_detail.colors_below),
            ),
            (
                "terrain_detail.textures_at",
                f64::from(graphics.terrain_detail.textures_at),
            ),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("{field} must be finite and positive").into());
            }
        }
        if graphics.terrain_detail.colors_below >= graphics.terrain_detail.textures_at {
            return Err("terrain_detail.colors_below must be less than textures_at".into());
        }
        Ok(graphics)
    }

    /// Looks up the color for a player slot.
    #[must_use]
    pub fn player_color(&self, slot: u8) -> Color {
        self.player_colors[usize::from(slot) % self.player_colors.len()]
    }
}

fn srgb<E: Error>((r, g, b): (f32, f32, f32)) -> Result<Color, E> {
    if [r, g, b].iter().all(|value| (0.0..=1.0).contains(value)) {
        Ok(Color::srgb(r, g, b))
    } else {
        Err(E::custom(
            "sRGB components must be finite and between 0 and 1",
        ))
    }
}

fn color<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    srgb(<(f32, f32, f32)>::deserialize(deserializer)?)
}

fn colors<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Color>, D::Error> {
    Vec::<(f32, f32, f32)>::deserialize(deserializer)?
        .into_iter()
        .map(srgb)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &str = include_str!("../../../assets/data/client.graphics.ron");

    #[test]
    fn bundled_graphics_preserve_palette_and_allow_slot_wrapping() {
        let graphics = Graphics::from_ron(BUNDLED).unwrap();
        assert_eq!(graphics.player_colors.len(), 8);
        assert_eq!(graphics.player_color(0), Color::srgb(0.20, 0.45, 0.90));
        assert_eq!(graphics.player_color(7), Color::srgb(0.90, 0.90, 0.90));
        assert_eq!(graphics.player_color(8), graphics.player_color(0));
    }

    #[test]
    fn rejects_invalid_graphics() {
        for (from, to) in [
            ("(0.20, 0.45, 0.90)", "(NaN, 0.45, 0.90)"),
            ("(0.20, 0.45, 0.90)", "(inf, 0.45, 0.90)"),
            ("(0.20, 0.45, 0.90)", "(-0.1, 0.45, 0.90)"),
            ("(0.20, 0.45, 0.90)", "(1.1, 0.45, 0.90)"),
            ("color: (0.95, 0.83, 0.45)", "color: (0.95, NaN, 0.45)"),
            ("padding: 4.0", "padding: -1.0"),
            ("padding: 4.0", "padding: NaN"),
            ("marker_radius: 12.0", "marker_radius: 0.0"),
            ("marker_cross_size: 5.0", "marker_cross_size: inf"),
            ("marker_duration: 0.6", "marker_duration: -0.6"),
            ("marker_duration: 0.6", "marker_duration: NaN"),
            ("colors_below: 40.0", "colors_below: 0.0"),
            ("textures_at: 48.0", "textures_at: NaN"),
            ("textures_at: 48.0", "textures_at: 40.0"),
            ("textures_at: 48.0", "textures_at: 39.0"),
            ("padding: 4.0", "paddding: 4.0"),
        ] {
            assert!(BUNDLED.contains(from));
            assert!(
                Graphics::from_ron(&BUNDLED.replace(from, to)).is_err(),
                "{to}"
            );
        }
        let start = BUNDLED.find('[').unwrap();
        let end = BUNDLED.find(']').unwrap();
        let empty_palette = format!("{}[]{}", &BUNDLED[..start], &BUNDLED[end + 1..]);
        assert!(
            Graphics::from_ron(&empty_palette)
                .unwrap_err()
                .to_string()
                .contains("player_colors must not be empty")
        );
        assert!(Graphics::from_ron("()").is_err());
    }
}
