//! Terrain rendering and conversion from simulation coordinates.

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    sprite_render::{AlphaMode2d, TileData, TilemapChunk, TilemapChunkTileData},
};
use rts_base::{IVec2 as SimPos, Rules, TILE};

use crate::game::{AppState, Match};

/// Screen pixels per tile at camera scale 1.
pub const TILE_PX: u32 = 128;

#[expect(
    clippy::cast_precision_loss,
    reason = "small powers of two convert exactly"
)]
const PX_PER_UNIT: f32 = TILE_PX as f32 / TILE as f32;

/// Converts a simulation position (world units, y down) to Bevy world space (pixels, y up).
/// The map's top-left corner is at the origin.
pub fn to_world(pos: SimPos) -> Vec2 {
    pos.as_vec2() * Vec2::new(PX_PER_UNIT, -PX_PER_UNIT)
}

/// Converts a simulation distance (world units) to Bevy world pixels.
#[expect(clippy::cast_precision_loss, reason = "distances are far below 2^24")]
pub fn to_world_distance(units: i32) -> f32 {
    units as f32 * PX_PER_UNIT
}

/// Area covered by the map in Bevy world space.
#[derive(Resource)]
pub struct MapBounds(pub Rect);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), spawn_terrain);
    }
}

/// Draws the whole map as one tilemap chunk.
fn spawn_terrain(mut commands: Commands, game: Res<Match>, mut images: ResMut<Assets<Image>>) {
    let terrain = game.sim.terrain();
    let tiles = terrain
        .tiles()
        .iter()
        .map(|id| Some(TileData::from_tileset_index(u16::from(id.0))))
        .collect();
    let chunk_size = terrain.size().as_uvec2();
    let bounds = Rect::from_corners(Vec2::ZERO, to_world(terrain.world_size()));
    commands.spawn((
        TilemapChunk {
            chunk_size,
            tile_display_size: UVec2::splat(TILE_PX),
            tileset: images.add(terrain_tileset(game.sim.rules())),
            alpha_mode: AlphaMode2d::Opaque,
        },
        TilemapChunkTileData::from_y_down_tiles(chunk_size, tiles),
        Transform::from_translation(bounds.center().extend(0.0)),
    ));
    commands.insert_resource(MapBounds(bounds));
}

/// Placeholder tileset with one solid-color 1x1 layer per terrain type, indexed by `TerrainId`.
///
/// Colors go in the texture rather than `TileData::color` because the tilemap shader multiplies
/// the tint's sRGB bytes as if they were linear, which washes colors out.
fn terrain_tileset(rules: &Rules) -> Image {
    let mut pixels: Vec<u8> = rules
        .terrain_types()
        .iter()
        .flat_map(|def| {
            Srgba::hex(&def.color)
                .unwrap_or_else(|error| {
                    warn!(
                        "terrain `{}` has bad color `{}`: {error}",
                        def.id, def.color
                    );
                    Srgba::rgb(1.0, 0.0, 1.0)
                })
                .to_u8_array()
        })
        .collect();
    // Array textures need at least two layers.
    if pixels.len() < 8 {
        pixels.resize(8, 0);
    }
    let layers = u32::try_from(pixels.len() / 4).expect("rules allow at most 256 terrain types");
    let mut image = Image::new(
        Extent3d {
            width: 1,
            height: layers,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image
        .reinterpret_stacked_2d_as_array(layers)
        .expect("a 1xN image splits into N layers");
    image
}
