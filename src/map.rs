use bevy::prelude::*;

const TILE_SIZE: f32 = 128.0;
const MAP_TILES: UVec2 = UVec2::new(32, 32);

#[derive(Resource)]
pub struct MapBounds(pub Rect);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        let size = MAP_TILES.as_vec2() * TILE_SIZE;
        app.insert_resource(MapBounds(Rect::from_center_size(Vec2::ZERO, size)))
            .add_systems(Startup, spawn_placeholder_tiles);
    }
}

/// Checkerboard for testing
fn spawn_placeholder_tiles(mut commands: Commands, bounds: Res<MapBounds>) {
    let light = Color::srgb(0.26, 0.42, 0.23);
    let dark = Color::srgb(0.22, 0.36, 0.20);
    for y in 0..MAP_TILES.y {
        for x in 0..MAP_TILES.x {
            let color = if (x + y) % 2 == 0 { light } else { dark };
            let center = bounds.0.min + (UVec2::new(x, y).as_vec2() + 0.5) * TILE_SIZE;
            commands.spawn((
                Sprite::from_color(color, Vec2::splat(TILE_SIZE)),
                Transform::from_translation(center.extend(0.0)),
            ));
        }
    }
}
