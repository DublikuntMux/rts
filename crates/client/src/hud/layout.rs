//! Shared logical geometry for the HUD, world hit testing, and camera limits.

use bevy::prelude::*;

use super::HUD_HEIGHT;

#[derive(Resource, Default)]
pub struct PhoneHud {
    pub panel: Option<PhonePanel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhonePanel {
    Map,
    Selection,
}

/// All rectangles are in logical UI pixels, measured from the top-left of the window.
pub struct HudLayout {
    pub phone: bool,
    pub landscape: bool,
    pub bottom_inset: f32,
    pub minimap: Rect,
    pub commands: Option<Rect>,
    pub details: Rect,
    modal: bool,
}

pub fn phone_layout(size: Vec2) -> bool {
    size.x < 620.0 || size.y < 520.0
}

pub fn hud_inset(size: Vec2) -> f32 {
    if phone_layout(size) { 0.0 } else { HUD_HEIGHT }
}

impl HudLayout {
    pub fn new(size: Vec2, selected: bool, panel: Option<PhonePanel>) -> Self {
        let phone = phone_layout(size);
        let landscape = size.x > size.y;
        let bottom_inset = hud_inset(size);
        if !phone {
            return Self {
                phone,
                landscape,
                bottom_inset,
                minimap: Rect::from_corners(
                    Vec2::new(0.0, size.y - bottom_inset),
                    Vec2::new(HUD_HEIGHT, size.y),
                ),
                commands: Some(Rect::from_corners(size - Vec2::splat(HUD_HEIGHT), size)),
                details: Rect::from_corners(
                    Vec2::new(HUD_HEIGHT, size.y - bottom_inset),
                    Vec2::new(size.x - HUD_HEIGHT, size.y),
                ),
                modal: false,
            };
        }

        let thumbnail_size = (size.min_element() * 0.3).clamp(72.0, 120.0);
        let minimap = if panel == Some(PhonePanel::Map) {
            Rect::from_corners(Vec2::new(8.0, 72.0), size - Vec2::splat(8.0))
        } else {
            Rect::from_corners(
                Vec2::new(size.x - thumbnail_size - 8.0, 8.0),
                Vec2::new(size.x - 8.0, thumbnail_size + 8.0),
            )
        };
        let commands = selected.then(|| {
            let command_size = if landscape {
                Vec2::new(210.0, 48.0)
            } else {
                Vec2::splat(102.0)
            };
            Rect::from_corners(
                size - command_size - Vec2::splat(8.0),
                size - Vec2::splat(8.0),
            )
        });
        let details_size = Vec2::new((size.x - 32.0).min(460.0), (size.y - 88.0).min(320.0));
        let details_min = Vec2::new((size.x - details_size.x) / 2.0, 72.0);
        Self {
            phone,
            landscape,
            bottom_inset,
            minimap,
            commands,
            details: Rect::from_corners(details_min, details_min + details_size),
            modal: panel.is_some(),
        }
    }

    /// Leave room to move edge units completely past a corner control and its touch area.
    pub fn camera_overscroll(&self) -> f32 {
        if self.phone {
            self.minimap
                .height()
                .max(self.commands.map_or(0.0, |rect| rect.height()))
                + 32.0
        } else {
            0.0
        }
    }

    /// Phone controls consume only their visible rectangles, unless a popup is open.
    pub fn blocks_point(&self, point: Vec2) -> bool {
        if self.phone {
            self.modal
                || self.minimap.contains(point)
                || self.commands.is_some_and(|rect| rect.contains(point))
        } else {
            point.y >= self.minimap.min.y
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_uses_either_small_dimension_and_preserves_the_battlefield() {
        for size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            let hud = HudLayout::new(size, true, None);
            assert!(hud.phone);
            assert_eq!(hud.landscape, size.x > size.y);
            assert_eq!(hud.bottom_inset, 0.0);
            assert!(!hud.blocks_point(Vec2::new(20.0, size.y - 20.0)));
            assert!(hud.blocks_point(hud.minimap.center()));
            let commands = hud.commands.unwrap();
            assert!(hud.blocks_point(commands.center()));
            assert_eq!(commands.max, size - Vec2::splat(8.0));
            assert_eq!(
                commands.size(),
                if hud.landscape {
                    Vec2::new(210.0, 48.0)
                } else {
                    Vec2::splat(102.0)
                }
            );
            let margin = hud.camera_overscroll();
            assert!(!hud.blocks_point(Vec2::new(size.x - margin, margin)));
            assert!(!hud.blocks_point(size - Vec2::splat(margin)));
            let empty = HudLayout::new(size, false, None);
            assert!(empty.commands.is_none());
            assert!(!empty.blocks_point(commands.center()));
        }
        assert!(!phone_layout(Vec2::new(620.0, 520.0)));
    }

    #[test]
    fn phone_popups_share_geometry_and_capture_the_whole_battlefield() {
        for size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for panel in [PhonePanel::Map, PhonePanel::Selection] {
                let hud = HudLayout::new(size, true, Some(panel));
                assert!(hud.blocks_point(Vec2::new(20.0, size.y - 20.0)));
                assert!(hud.blocks_point(size / 2.0));
                assert_eq!(hud.details.min.y, 72.0);
                assert_eq!(hud.details.center().x, size.x / 2.0);
                assert_eq!(hud.details.size().x, 460.0_f32.min(size.x - 32.0));
                assert_eq!(hud.details.size().y, 320.0_f32.min(size.y - 88.0));
                if panel == PhonePanel::Map {
                    assert_eq!(hud.minimap.min, Vec2::new(8.0, 72.0));
                    assert_eq!(hud.minimap.max, size - Vec2::splat(8.0));
                }
            }
        }
    }

    #[test]
    fn desktop_retains_the_bottom_console_even_with_a_stale_phone_panel() {
        let size = Vec2::new(1200.0, 800.0);
        let hud = HudLayout::new(size, false, Some(PhonePanel::Map));
        assert!(!hud.phone);
        assert_eq!(hud.bottom_inset, HUD_HEIGHT);
        assert_eq!(hud.minimap.size(), Vec2::splat(HUD_HEIGHT));
        assert_eq!(hud.commands.unwrap().size(), Vec2::splat(HUD_HEIGHT));
        assert!(!hud.blocks_point(Vec2::new(600.0, 615.0)));
        assert!(hud.blocks_point(Vec2::new(600.0, 616.0)));
    }
}
