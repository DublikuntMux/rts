//! Unit rendering from simulation state.

use bevy::{platform::collections::HashMap, prelude::*};
use rts_base::{PlayerId, UnitId, UnitKind};

use crate::{
    game::{AppState, Match},
    map::{to_world, to_world_distance},
};

/// Player colors by slot.
const PLAYER_COLORS: [Color; 8] = [
    Color::srgb(0.20, 0.45, 0.90),
    Color::srgb(0.85, 0.20, 0.20),
    Color::srgb(0.95, 0.80, 0.20),
    Color::srgb(0.30, 0.75, 0.35),
    Color::srgb(0.65, 0.35, 0.85),
    Color::srgb(0.95, 0.55, 0.15),
    Color::srgb(0.25, 0.80, 0.85),
    Color::srgb(0.90, 0.90, 0.90),
];

const UNIT_Z: f32 = 1.0;

pub struct UnitsPlugin;

impl Plugin for UnitsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UnitVisuals>()
            .add_systems(Update, sync_units.run_if(in_state(AppState::InGame)));
    }
}

/// Entities drawing simulation units, plus shared render assets.
#[derive(Resource, Default)]
struct UnitVisuals {
    entities: HashMap<UnitId, Entity>,
    meshes: HashMap<UnitKind, Handle<Mesh>>,
    materials: HashMap<PlayerId, Handle<ColorMaterial>>,
}

/// Spawns, moves, and despawns unit entities to match the simulation.
fn sync_units(
    mut commands: Commands,
    game: Res<Match>,
    mut visuals: ResMut<UnitVisuals>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut transforms: Query<&mut Transform>,
) {
    let UnitVisuals {
        entities,
        meshes: mesh_cache,
        materials: material_cache,
    } = &mut *visuals;
    for (id, unit) in game.sim.units() {
        let translation = to_world(unit.pos).extend(UNIT_Z);
        if let Some(&entity) = entities.get(&id) {
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.translation = translation;
            }
            continue;
        }
        let mesh = mesh_cache.entry(unit.kind).or_insert_with(|| {
            let radius = game.sim.rules().unit(unit.kind).radius;
            meshes.add(Circle::new(to_world_distance(radius)))
        });
        let material = material_cache.entry(unit.owner).or_insert_with(|| {
            materials.add(PLAYER_COLORS[usize::from(unit.owner.0) % PLAYER_COLORS.len()])
        });
        let entity = commands
            .spawn((
                Mesh2d(mesh.clone()),
                MeshMaterial2d(material.clone()),
                Transform::from_translation(translation),
            ))
            .id();
        entities.insert(id, entity);
    }
    entities.retain(|&id, &mut entity| {
        let alive = game.sim.unit(id).is_some();
        if !alive {
            commands.entity(entity).despawn();
        }
        alive
    });
}
