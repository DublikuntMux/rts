//! Small map images for the menu: terrain colors with player start markers.

use bevy::{
    asset::RenderAssetUsages,
    color::{ColorToPacked, Srgba},
    image::{Image, ImageSampler},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use rts_assets::Graphics;
use rts_base::{Sim, TILE};

/// Preview pixels per map tile.
const SCALE: i32 = 4;
/// Start marker radius, in tiles.
const MARKER_RADIUS: f32 = 1.6;
const MARKER_EDGE: [u8; 4] = [16, 20, 28, 255];

/// Renders a map's terrain and player starts.
#[expect(
    clippy::cast_precision_loss,
    reason = "preview pixel coordinates are far below f32 precision limits"
)]
pub fn map_preview(sim: &Sim, graphics: &Graphics) -> Image {
    let terrain = sim.terrain();
    let size = terrain.size();
    let (width, height) = (size.x * SCALE, size.y * SCALE);
    let colors: Vec<[u8; 4]> = sim
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
            let tile =
                terrain.tiles()[usize::try_from((y / SCALE) * size.x + x / SCALE).unwrap_or(0)];
            pixels.extend_from_slice(&colors[usize::from(tile.0)]);
        }
    }
    let scale = SCALE as f32;
    for (slot, player) in sim.players().iter().enumerate() {
        let center = player.start.as_vec2() / TILE as f32 * scale;
        let color =
            Srgba::from(graphics.player_color(u8::try_from(slot).unwrap_or(0))).to_u8_array();
        let outer = MARKER_RADIUS * scale;
        let inner = outer - 1.5;
        for y in 0..height {
            for x in 0..width {
                let distance = (glam_vec(x, y) - center).length();
                if distance <= outer {
                    let index = usize::try_from((y * width + x) * 4).unwrap_or(0);
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
    fn preview_scales_terrain_and_marks_starts() {
        let sim = MatchSetup {
            rules: include_str!("../../../assets/data/core.rules.ron").into(),
            map: include_str!("../../../assets/maps/proving_grounds.map.ron").into(),
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
        assert_eq!(corner, Srgba::hex("#4a7a3e").unwrap().to_u8_array());
    }
}
