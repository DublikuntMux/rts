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
use rts_assets::{Graphics, TerrainDetail};
use rts_base::{IVec2 as SimPos, PlayerView, Rules, TILE, TileVisibility};

use crate::{
    camera::{CameraControl, RtsCamera},
    game::{AppState, Match},
    server_assets::ServerAssets,
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
        app.init_resource::<ZoomDetail>()
            .add_systems(OnEnter(AppState::InGame), spawn_terrain)
            .add_systems(
                Update,
                update_zoom_detail
                    .after(CameraControl)
                    .in_set(UpdateZoomDetail)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(Update, update_fog.run_if(in_state(AppState::InGame)))
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

/// One zoom decision shared by terrain and all unit visuals.
#[derive(Resource)]
pub struct ZoomDetail(pub bool);

impl Default for ZoomDetail {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UpdateZoomDetail;

pub fn update_zoom_detail(
    graphics: Res<Graphics>,
    camera: Query<&Projection, With<RtsCamera>>,
    mut detail: ResMut<ZoomDetail>,
) {
    if let Ok(Projection::Orthographic(projection)) = camera.single() {
        detail.0 = show_detail(projection.scale, detail.0, &graphics.terrain_detail);
    }
}

#[derive(Component)]
struct TerrainChunk(u32);

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
    server_assets: Option<Res<ServerAssets>>,
    mut images: ResMut<Assets<Image>>,
) {
    let tiles = fog_tiles(&game.view);
    let chunk_size = game.view.size().as_uvec2();
    let bounds = Rect::from_corners(Vec2::ZERO, to_world(game.view.world_size()));
    let colors = images.add(terrain_colors(game.view.rules()));
    let textures = game
        .view
        .rules()
        .terrain_types()
        .iter()
        .map(|def| {
            def.texture.as_ref().map(|path| {
                if let Some(decoded) = server_assets
                    .as_deref()
                    .and_then(|synced| synced.terrain_image(path))
                {
                    match decoded {
                        Ok(image) => return images.add(image),
                        Err(error) => warn!("{error}; falling back to bundled texture"),
                    }
                }
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
        TerrainChunk(game.view.terrain_revision()),
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

/// Tint of explored terrain out of sight.
const REMEMBERED_TINT: Color = Color::srgb(0.42, 0.42, 0.48);

/// Terrain as the local player knows it: unexplored tiles are black, remembered tiles are dim.
pub fn fog_tiles(view: &PlayerView) -> Vec<Option<TileData>> {
    let size = view.size();
    view.terrain_tiles()
        .iter()
        .enumerate()
        .map(|(index, terrain)| {
            let index = i32::try_from(index).expect("bounded map size");
            let tile = SimPos::new(index % size.x, index / size.x);
            Some(terrain.map_or_else(
                || TileData {
                    color: Color::BLACK,
                    ..TileData::from_tileset_index(0)
                },
                |terrain| TileData {
                    color: if view.visibility(tile) == TileVisibility::Visible {
                        Color::WHITE
                    } else {
                        REMEMBERED_TINT
                    },
                    ..TileData::from_tileset_index(u16::from(terrain.0))
                },
            ))
        })
        .collect()
}

/// Redraws only when discovery or current sight changes, or a new match starts.
fn update_fog(game: Res<Match>, mut chunks: Query<(&mut TerrainChunk, &mut TilemapChunkTileData)>) {
    let revision = game.view.terrain_revision();
    let size = game.view.size().as_uvec2();
    for (mut drawn, mut data) in &mut chunks {
        if drawn.0 != revision || game.is_added() {
            *data = TilemapChunkTileData::from_y_down_tiles(size, fog_tiles(&game.view));
            drawn.0 = revision;
        }
    }
}

fn clear_map(mut commands: Commands, mut detail: ResMut<ZoomDetail>) {
    *detail = ZoomDetail::default();
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
    match textured_tileset(game.view.rules(), &sources) {
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
fn show_detail(scale: f32, was_detailed: bool, detail: &TerrainDetail) -> bool {
    let pixels_per_tile = TILE_PX as f32 / scale;
    pixels_per_tile
        >= if was_detailed {
            detail.colors_below
        } else {
            detail.textures_at
        }
}

fn update_terrain_detail(
    tilesets: Res<TerrainTilesets>,
    detail: Res<ZoomDetail>,
    chunks: Query<&MeshMaterial2d<TilemapChunkMaterial>, With<TerrainChunk>>,
    mut materials: ResMut<Assets<TilemapChunkMaterial>>,
) {
    for handle in &chunks {
        let Some(material) = materials.get(handle) else {
            continue;
        };
        let desired = if detail.0 {
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
    fn fog_paints_unexplored_black_and_remembered_terrain_dim() {
        let row = ".".repeat(30);
        let map = rts_base::MapData::from_ron(&format!(
            r#"(name: "fog", players: [(start: (1.5, 0.5))], tiles: ["{row}"],
            units: [(kind: "worker", owner: 0, pos: (1.5, 0.5))])"#
        ))
        .unwrap();
        let mut sim = rts_base::Sim::new(rules(), &map).unwrap();
        let (mut knowledge, mut view) = PlayerView::observe(&sim, rts_base::PlayerId(0));
        let tiles = fog_tiles(&view);
        assert_eq!(tiles.len(), 30);
        assert_eq!(tiles[1].unwrap().color, Color::WHITE);
        assert_eq!(tiles[29].unwrap().color, Color::BLACK);
        sim.step_with_commands([(
            rts_base::PlayerId(0),
            rts_base::Command::Move {
                units: vec![rts_base::UnitId(0)],
                target: SimPos::new(28 * TILE + TILE / 2, TILE / 2),
            },
        )]);
        view.apply(knowledge.observe(&sim)).unwrap();
        for _ in 0..200 {
            sim.step();
            view.apply(knowledge.observe(&sim)).unwrap();
        }
        let tiles = fog_tiles(&view);
        assert_eq!(tiles[1].unwrap().color, REMEMBERED_TINT);
        assert_eq!(tiles[29].unwrap().color, Color::WHITE);
    }

    #[test]
    fn terrain_redraws_only_when_the_terrain_revision_changes() {
        let row = ".".repeat(30);
        let map = rts_base::MapData::from_ron(&format!(
            r#"(name: "fog", players: [(start: (1.5, 0.5))], tiles: ["{row}"],
            units: [(kind: "worker", owner: 0, pos: (1.5, 0.5))])"#
        ))
        .unwrap();
        let sim = rts_base::Sim::new(rules(), &map).unwrap();
        let game = crate::game::Match::new(sim, rts_base::PlayerId(0), true);
        let revision = game.view.terrain_revision();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(game)
            .add_systems(Update, update_fog);
        let blank = || TilemapChunkTileData(vec![None; 30]);
        let chunk = app
            .world_mut()
            .spawn((TerrainChunk(revision), blank()))
            .id();
        let drawn = |app: &App| {
            app.world()
                .get::<TilemapChunkTileData>(chunk)
                .unwrap()
                .iter()
                .all(Option::is_some)
        };
        app.update();
        assert!(drawn(&app), "a new match draws its terrain");

        app.world_mut().entity_mut(chunk).insert(blank());
        app.world_mut().resource_mut::<crate::game::Match>().step();
        app.update();
        assert_eq!(
            app.world()
                .resource::<crate::game::Match>()
                .view
                .terrain_revision(),
            revision
        );
        assert!(!drawn(&app), "quiet ticks keep the drawn terrain");

        let mut game = app.world_mut().resource_mut::<crate::game::Match>();
        game.step_with([(
            rts_base::PlayerId(0),
            rts_base::Command::Move {
                units: vec![rts_base::UnitId(0)],
                target: SimPos::new(28 * TILE + TILE / 2, TILE / 2),
            },
        )]);
        for _ in 0..200 {
            if game.view.terrain_revision() != revision {
                break;
            }
            game.step();
        }
        assert_ne!(
            game.view.terrain_revision(),
            revision,
            "the worker discovers terrain"
        );
        app.update();
        assert!(drawn(&app), "discovery redraws the terrain");
    }

    #[test]
    fn far_zoom_uses_colors_and_threshold_has_hysteresis() {
        let graphics =
            Graphics::from_ron(include_str!("../../../assets/data/client.graphics.ron")).unwrap();
        let detail = &graphics.terrain_detail;
        assert!(show_detail(1.0, false, detail));
        assert!(show_detail(4.0, true, detail));
        assert!(!show_detail(12.0, true, detail));
        assert!(show_detail(9.0, true, detail));
        assert!(!show_detail(9.0, false, detail));
        assert!(show_detail(8.0, false, detail));

        let detail = TerrainDetail {
            colors_below: 64.0,
            textures_at: 128.0,
        };
        assert!(show_detail(2.0, true, &detail));
        assert!(!show_detail(2.01, true, &detail));
        assert!(!show_detail(2.0, false, &detail));
        assert!(show_detail(1.0, false, &detail));
        assert!(!show_detail(1.01, false, &detail));
    }
}
