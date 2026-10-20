//! Unit rendering from simulation state.

use bevy::{platform::collections::HashMap, prelude::*};
use rts_base::{IVec2 as SimPos, PlayerId, UnitId, UnitKind};

use crate::{
    game::{AppState, Match, PreviousPositions},
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

#[derive(Component)]
pub struct UnitVisual(pub UnitId);

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyncUnits;

pub struct UnitsPlugin;

impl Plugin for UnitsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UnitVisuals>()
            .add_systems(
                Update,
                sync_units
                    .in_set(SyncUnits)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnExit(AppState::InGame), clear_units);
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
    previous: Res<PreviousPositions>,
    fixed: Res<Time<Fixed>>,
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
        let translation = interpolated_position(
            previous.0.get(&id).copied(),
            unit.pos,
            fixed.overstep_fraction(),
        )
        .extend(UNIT_Z);
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
                DespawnOnExit(AppState::InGame),
                UnitVisual(id),
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

fn clear_units(mut visuals: ResMut<UnitVisuals>) {
    *visuals = UnitVisuals::default();
}

fn interpolated_position(previous: Option<SimPos>, current: SimPos, alpha: f32) -> Vec2 {
    to_world(previous.unwrap_or(current)).lerp(to_world(current), alpha.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use rts_base::{Command, MapData, Rules, Sim, TILE};

    use super::*;

    #[test]
    fn interpolates_tick_snapshots_without_changing_simulation() {
        let rules = Rules::from_ron(include_str!("../../../assets/data/core.rules.ron")).unwrap();
        let map = MapData::from_ron(
            r#"(name: "interpolation", players: [(start: (0.5,0.5))],
            tiles: ["...."], units: [(kind:"worker",owner:0,pos:(0.5,0.5))])"#,
        )
        .unwrap();
        let mut sim = Sim::new(rules, &map).unwrap();
        let id = UnitId(0);
        sim.queue_command(
            PlayerId(0),
            Command::Move {
                units: vec![id],
                target: SimPos::new(3 * TILE, TILE / 2),
            },
        );
        sim.step();
        let start = sim.unit(id).unwrap().pos;
        sim.step();
        let end = sim.unit(id).unwrap().pos;
        assert_ne!(start, end);
        let hash = sim.state_hash();
        assert_eq!(
            interpolated_position(Some(start), end, 0.0),
            to_world(start)
        );
        assert_eq!(
            interpolated_position(Some(start), end, 0.5),
            (to_world(start) + to_world(end)) / 2.0
        );
        assert_eq!(interpolated_position(Some(start), end, 1.0), to_world(end));
        assert_eq!(interpolated_position(None, end, 0.0), to_world(end));
        assert_eq!(sim.state_hash(), hash);
    }
}
