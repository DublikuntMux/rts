//! Terrain rendering and conversion from simulation coordinates.

use bevy::{
    asset::{LoadState, RenderAssetUsages},
    image::{ImageLoaderSettings, ImageSampler},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    sprite_render::{
        AlphaMode2d, MeshMaterial2d, TileData, TilemapChunk, TilemapChunkMaterial,
        TilemapChunkTileData,
    },
};
use rts_base::{IVec2 as SimPos, Rules, TILE};

use crate::{
    camera::RtsCamera,
    game::{AppState, Match},
};

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
        app.add_systems(OnEnter(AppState::InGame), spawn_terrain)
            .add_systems(
                Update,
                finish_terrain_textures.run_if(resource_exists::<PendingTerrainTextures>),
            )
            .add_systems(
                PostUpdate,
                update_terrain_detail.run_if(resource_exists::<TerrainTilesets>),
            )
            .add_systems(OnExit(AppState::InGame), clear_map);
    }
}

#[derive(Component)]
struct TerrainChunk;

/// The original palette stays separate from the detailed terrain for distant zoom and minimaps.
#[derive(Resource)]
struct TerrainTilesets {
    colors: Handle<Image>,
    detailed: Option<Handle<Image>>,
}

#[derive(Resource)]
struct PendingTerrainTextures(Vec<Option<Handle<Image>>>);

/// Draws the whole map as one tilemap chunk.
fn spawn_terrain(
    mut commands: Commands,
    game: Res<Match>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    let terrain = game.sim.terrain();
    let tiles = terrain
        .tiles()
        .iter()
        .map(|id| Some(TileData::from_tileset_index(u16::from(id.0))))
        .collect();
    let chunk_size = terrain.size().as_uvec2();
    let bounds = Rect::from_corners(Vec2::ZERO, to_world(terrain.world_size()));
    let colors = images.add(terrain_colors(game.sim.rules()));
    let textures = game
        .sim
        .rules()
        .terrain_types()
        .iter()
        .map(|def| {
            def.texture.as_ref().map(|path| {
                assets
                    .load_builder()
                    .with_settings(|settings: &mut ImageLoaderSettings| {
                        // Keep CPU pixels until they have been assembled into the texture array.
                        settings.asset_usage = RenderAssetUsages::MAIN_WORLD;
                    })
                    .load(path.clone())
            })
        })
        .collect();
    commands.spawn((
        DespawnOnExit(AppState::InGame),
        TerrainChunk,
        TilemapChunk {
            chunk_size,
            tile_display_size: UVec2::splat(TILE_PX),
            tileset: colors.clone(),
            alpha_mode: AlphaMode2d::Opaque,
        },
        TilemapChunkTileData::from_y_down_tiles(chunk_size, tiles),
        Transform::from_translation(bounds.center().extend(0.0)),
    ));
    commands.insert_resource(TerrainTilesets {
        colors,
        detailed: None,
    });
    commands.insert_resource(PendingTerrainTextures(textures));
    commands.insert_resource(MapBounds(bounds));
}

fn clear_map(mut commands: Commands) {
    commands.remove_resource::<MapBounds>();
    commands.remove_resource::<TerrainTilesets>();
    commands.remove_resource::<PendingTerrainTextures>();
}

/// One solid-color 1x1 layer per terrain type, indexed by `TerrainId`.
///
/// Colors go in the texture rather than `TileData::color` because the tilemap shader multiplies
/// the tint's sRGB bytes as if they were linear, which washes colors out.
fn terrain_colors(rules: &Rules) -> Image {
    let pixels: Vec<u8> = rules
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
    array_image(1, pixels)
}

fn array_image(tile_size: u32, mut pixels: Vec<u8>) -> Image {
    let layer_bytes = (tile_size * tile_size * 4) as usize;
    // Array textures need at least two layers.
    if pixels.len() < layer_bytes * 2 {
        pixels.resize(layer_bytes * 2, 0);
    }
    let layers =
        u32::try_from(pixels.len() / layer_bytes).expect("rules allow at most 256 terrain types");
    let mut image = Image::new(
        Extent3d {
            width: tile_size,
            height: tile_size * layers,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image
        .reinterpret_stacked_2d_as_array(layers)
        .expect("stacked square tiles split into equal layers");
    image.sampler = ImageSampler::linear();
    image
}

fn finish_terrain_textures(
    mut commands: Commands,
    pending: Res<PendingTerrainTextures>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut tilesets: ResMut<TerrainTilesets>,
    game: Res<Match>,
) {
    for handle in pending.0.iter().flatten() {
        if let Some(LoadState::Failed(error)) = assets.get_load_state(handle) {
            error!("cannot load terrain texture: {error}");
            commands.remove_resource::<PendingTerrainTextures>();
            return;
        }
        if !images.contains(handle) {
            return;
        }
    }
    let sources: Vec<_> = pending
        .0
        .iter()
        .map(|handle| handle.as_ref().and_then(|handle| images.get(handle)))
        .collect();
    match textured_tileset(game.sim.rules(), &sources) {
        Ok(image) => tilesets.detailed = Some(images.add(image)),
        Err(error) => error!("cannot build terrain tileset: {error}"),
    }
    commands.remove_resource::<PendingTerrainTextures>();
}

/// Assemble equally sized square textures in `TerrainId` order, filling untextured types with color.
fn textured_tileset(rules: &Rules, sources: &[Option<&Image>]) -> Result<Image, String> {
    let tile_size = sources
        .iter()
        .flatten()
        .next()
        .map_or(1, |image| image.width());
    if tile_size == 0 {
        return Err("terrain textures must not be empty".into());
    }
    let colors = terrain_colors(rules);
    let palette = colors.data.as_ref().expect("palette has CPU pixels");
    let mut pixels = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        if let Some(source) = source {
            if source.width() != tile_size
                || source.height() != tile_size
                || source.texture_descriptor.size.depth_or_array_layers != 1
            {
                return Err("terrain textures must be square and have the same size".into());
            }
            let rgba = source
                .convert(TextureFormat::Rgba8UnormSrgb)
                .ok_or("terrain texture cannot be converted to RGBA")?;
            pixels.extend_from_slice(
                rgba.data
                    .as_ref()
                    .ok_or("terrain texture has no CPU pixels")?,
            );
        } else {
            let color = &palette[index * 4..index * 4 + 4];
            for _ in 0..tile_size * tile_size {
                pixels.extend_from_slice(color);
            }
        }
    }
    Ok(array_image(tile_size, pixels))
}

// Hysteresis keeps small wheel/pinch changes from alternating the material at the boundary.
#[expect(
    clippy::cast_precision_loss,
    reason = "tile size is a small power of two"
)]
fn show_detail(scale: f32, was_detailed: bool) -> bool {
    let pixels_per_tile = TILE_PX as f32 / scale;
    pixels_per_tile >= if was_detailed { 40.0 } else { 48.0 }
}

fn update_terrain_detail(
    tilesets: Res<TerrainTilesets>,
    camera: Single<&Projection, With<RtsCamera>>,
    chunks: Query<&MeshMaterial2d<TilemapChunkMaterial>, With<TerrainChunk>>,
    mut materials: ResMut<Assets<TilemapChunkMaterial>>,
) {
    let Projection::Orthographic(projection) = camera.into_inner() else {
        return;
    };
    for handle in &chunks {
        let Some(material) = materials.get(handle) else {
            continue;
        };
        let was_detailed = tilesets.detailed.as_ref() == Some(&material.tileset);
        let desired = if show_detail(projection.scale, was_detailed) {
            tilesets.detailed.as_ref().unwrap_or(&tilesets.colors)
        } else {
            &tilesets.colors
        };
        if material.tileset != *desired {
            materials.get_mut(handle).expect("material exists").tileset = desired.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::image::{CompressedImageFormats, ImageType};

    use super::*;

    fn rules() -> Rules {
        Rules::from_ron(include_str!("../../../assets/data/core.rules.ron")).unwrap()
    }

    #[test]
    fn bundled_textures_form_terrain_id_ordered_array() {
        let rules = rules();
        let images: Vec<_> = rules
            .terrain_types()
            .iter()
            .map(|def| {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../assets")
                    .join(def.texture.as_ref().unwrap());
                Image::from_buffer(
                    &std::fs::read(path).unwrap(),
                    ImageType::Extension("png"),
                    CompressedImageFormats::NONE,
                    true,
                    ImageSampler::linear(),
                    RenderAssetUsages::MAIN_WORLD,
                )
                .unwrap()
            })
            .collect();
        let sources: Vec<_> = images.iter().map(Some).collect();
        let tileset = textured_tileset(&rules, &sources).unwrap();
        assert_eq!(
            tileset.texture_descriptor.size,
            Extent3d {
                width: 128,
                height: 128,
                depth_or_array_layers: 5,
            }
        );
        assert_eq!(
            tileset.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb
        );
        let layer_bytes = 128 * 128 * 4;
        let data = tileset.data.unwrap();
        for (layer, source) in data.chunks_exact(layer_bytes).zip(&images) {
            assert_eq!(
                layer,
                source
                    .convert(TextureFormat::Rgba8UnormSrgb)
                    .unwrap()
                    .data
                    .unwrap()
            );
            assert!(
                layer
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel != &layer[..4])
            );
        }
    }

    #[test]
    fn untextured_terrain_preserves_original_palette() {
        let rules = rules();
        let colors = terrain_colors(&rules);
        assert_eq!(
            colors.data.as_ref().unwrap(),
            &[
                0x4a, 0x7a, 0x3e, 255, 0x8a, 0x73, 0x50, 255, 0xc9, 0xb2, 0x7a, 255, 0x2e, 0x5d,
                0x8c, 255, 0x5a, 0x58, 0x52, 255,
            ]
        );
        let source = Image::new_fill(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[1, 2, 3, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        );
        let tileset = textured_tileset(&rules, &[None, Some(&source), None, None, None]).unwrap();
        let data = tileset.data.unwrap();
        assert_eq!(&data[..16], &[0x4a, 0x7a, 0x3e, 255].repeat(4));
        assert_eq!(&data[16..32], source.data.as_ref().unwrap());
    }

    #[test]
    fn far_zoom_uses_colors_and_threshold_has_hysteresis() {
        assert!(show_detail(1.0, false));
        assert!(!show_detail(4.0, true));
        assert!(show_detail(3.0, true));
        assert!(!show_detail(3.0, false));
        assert!(show_detail(2.5, false));
    }
}
