//! Small map images for the menu from the local player's opening view.

use bevy::{
    asset::RenderAssetUsages,
    color::{ColorToPacked, Srgba},
    image::{Image, ImageSampler},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use rts_assets::Graphics;
use rts_base::{PlayerId, PlayerView, Sim, TILE};

/// Preview pixels per map tile.
const SCALE: i32 = 4;
/// Start marker radius, in tiles.
const MARKER_RADIUS: f32 = 1.6;
#[expect(
    clippy::cast_precision_loss,
    reason = "preview scale is exactly representable"
)]
const MARKER_OUTER: f32 = MARKER_RADIUS * SCALE as f32;
const MARKER_EDGE: [u8; 4] = [16, 20, 28, 255];

/// Renders explored terrain and the local start, as they appear before the first turn.
pub fn map_preview(sim: &Sim, graphics: &Graphics) -> Image {
    let (_, view) = PlayerView::observe(sim, PlayerId(0));
    opening_preview(&view, graphics)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "preview pixel coordinates are far below f32 precision limits"
)]
fn opening_preview(view: &PlayerView, graphics: &Graphics) -> Image {
    let size = view.size();
    let (width, height) = (size.x * SCALE, size.y * SCALE);
    let colors: Vec<[u8; 4]> = view
        .rules()
        .terrain_types()
        .iter()
        .map(|def| {
            Srgba::hex(&def.color)
                .unwrap_or(Srgba::rgb(1.0, 0.0, 1.0))
                .to_u8_array()
        })
        .collect();
    let mut pixels = Vec::with_capacity(usize::try_from(width * height * 4).unwrap_or(0));
    for y in 0..height {
        for x in 0..width {
            let tile = bevy::math::IVec2::new(x / SCALE, y / SCALE);
            let color = view
                .terrain(tile)
                .map_or([0, 0, 0, 255], |terrain| colors[usize::from(terrain.0)]);
            pixels.extend_from_slice(&color);
        }
    }
    let scale = SCALE as f32;
    let center = view.start().as_vec2() / TILE as f32 * scale;
    let color = Srgba::from(graphics.player_color(view.player().0)).to_u8_array();
    paint_start(&mut pixels, width, height, center, color);
    let mut image = Image::new(
        Extent3d {
            width: u32::try_from(width).unwrap_or(1),
            height: u32::try_from(height).unwrap_or(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Keep terrain boundaries and start markers crisp when the menu enlarges this image.
    image.sampler = ImageSampler::nearest();
    image
}

/// Visit only the marker's clipped bounds, regardless of the map's dimensions.
fn paint_start(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    center: bevy::math::Vec2,
    color: [u8; 4],
) {
    let outer = MARKER_OUTER;
    let inner = outer - 1.5;
    let radius = bevy::math::Vec2::splat(outer);
    let min = (center - radius)
        .floor()
        .as_ivec2()
        .max(bevy::math::IVec2::ZERO);
    let max = (center + radius)
        .ceil()
        .as_ivec2()
        .min(bevy::math::IVec2::new(width, height));
    for y in min.y..max.y {
        for x in min.x..max.x {
            let distance = (glam_vec(x, y) - center).length();
            if distance <= outer {
                let index =
                    usize::try_from((y * width + x) * 4).expect("pixel is inside the image");
                let paint = if distance <= inner {
                    color
                } else {
                    MARKER_EDGE
                };
                pixels[index..index + 4].copy_from_slice(&paint);
            }
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "preview pixel coordinates are far below f32 precision limits"
)]
fn glam_vec(x: i32, y: i32) -> bevy::math::Vec2 {
    bevy::math::Vec2::new(x as f32 + 0.5, y as f32 + 0.5)
}

#[cfg(test)]
mod tests {
    use rts_net::MatchSetup;

    use super::*;

    #[test]
    fn clipped_markers_match_full_image_scan_at_edges_and_fractional_starts() {
        let (width, height) = (96, 64);
        let mut clipped = vec![0; usize::try_from(width * height * 4).unwrap()];
        let mut reference = clipped.clone();
        let color = [70, 130, 240, 255];
        for center in [
            bevy::math::Vec2::ZERO,
            bevy::math::Vec2::new(0.125, 32.5),
            bevy::math::Vec2::new(48.25, 0.75),
            bevy::math::Vec2::new(48.375, 32.625),
            bevy::math::Vec2::new(95.875, 63.875),
            bevy::math::Vec2::new(95.0, 0.0),
            bevy::math::Vec2::new(0.0, 63.0),
        ] {
            paint_start(&mut clipped, width, height, center, color);
            let outer = MARKER_OUTER;
            for y in 0..height {
                for x in 0..width {
                    let distance = (glam_vec(x, y) - center).length();
                    if distance <= outer {
                        let index = usize::try_from((y * width + x) * 4).unwrap();
                        let paint = if distance <= outer - 1.5 {
                            color
                        } else {
                            MARKER_EDGE
                        };
                        reference[index..index + 4].copy_from_slice(&paint);
                    }
                }
            }
            assert_eq!(clipped, reference, "marker at {center}");
        }
    }

    #[test]
    fn preview_scales_known_terrain_and_marks_only_the_local_start() {
        let sim = MatchSetup {
            rules: include_str!("../../../assets/data/core.rules.ron").into(),
            map: include_str!("../../../assets/maps/frontier.map.ron").into(),
            seed: 0,
            players: 2,
        }
        .simulation()
        .unwrap();
        let graphics =
            Graphics::from_ron(include_str!("../../../assets/data/client.graphics.ron")).unwrap();
        let image = map_preview(&sim, &graphics);
        let size = sim.terrain().size();
        assert_eq!(image.width(), u32::try_from(size.x * SCALE).unwrap());
        assert_eq!(image.height(), u32::try_from(size.y * SCALE).unwrap());
        let start = sim.players()[0].start / TILE * SCALE;
        let pixel = image
            .pixel_bytes(bevy::math::UVec3::new(
                u32::try_from(start.x).unwrap(),
                u32::try_from(start.y).unwrap(),
                0,
            ))
            .unwrap();
        assert_eq!(pixel, Srgba::from(graphics.player_color(0)).to_u8_array());
        let corner = image.pixel_bytes(bevy::math::UVec3::ZERO).unwrap();
        assert_eq!(corner, [0, 0, 0, 255]);
        let enemy = ((sim.players()[1].start / TILE) * SCALE).as_uvec2();
        assert_eq!(image.pixel_bytes(enemy.extend(0)).unwrap(), [0, 0, 0, 255]);
        let tile = sim.deposits().next().unwrap().1.pos / TILE;
        let wood = (tile * SCALE).as_uvec2();
        let terrain = sim.terrain().get(tile).unwrap();
        let color = Srgba::hex(&sim.rules().terrain(terrain).color)
            .unwrap()
            .to_u8_array();
        assert_eq!(image.pixel_bytes(wood.extend(0)).unwrap(), color);
    }
}
