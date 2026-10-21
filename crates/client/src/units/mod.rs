//! Unit rendering from simulation state.

mod layout;

use bevy::{platform::collections::HashMap, prelude::*};
use rts_assets::Graphics;
use rts_base::{PlayerId, UnitId, UnitKind};

pub use self::layout::UnitBounds;
use self::layout::{Placement, layout_units};

use crate::{
    game::{AppState, Match, PreviousPositions},
    map::to_world_distance,
};

const UNIT_Z: f32 = 1.0;

#[derive(Component, Clone)]
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

/// Presentation for one simulation unit; extend this scene with visual children as needed.
fn unit_scene(
    id: UnitId,
    mesh: Handle<Mesh>,
    material: Handle<ColorMaterial>,
    placement: Placement,
) -> impl Scene {
    let visual = UnitVisual(id);
    let transform = placement_transform(placement);
    bsn! {
        DespawnOnExit<AppState>(AppState::InGame)
        visual
        Mesh2d(mesh)
        MeshMaterial2d<ColorMaterial>(material)
        transform
    }
}

/// Spawns, moves, and despawns unit entities to match the simulation.
fn sync_units(
    mut commands: Commands,
    game: Res<Match>,
    graphics: Res<Graphics>,
    previous: Res<PreviousPositions>,
    fixed: Res<Time<Fixed>>,
    mut visuals: ResMut<UnitVisuals>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut transforms: Query<&mut Transform>,
) {
    let current_layout = layout_units(game.sim.units().map(|(id, unit)| {
        (
            id,
            unit.pos,
            UnitBounds::from_definition(game.sim.rules().unit(unit.kind)),
        )
    }));
    let previous_layout = layout_units(game.sim.units().map(|(id, unit)| {
        (
            id,
            previous.0.get(&id).copied().unwrap_or(unit.pos),
            UnitBounds::from_definition(game.sim.rules().unit(unit.kind)),
        )
    }));
    let UnitVisuals {
        entities,
        meshes: mesh_cache,
        materials: material_cache,
    } = &mut *visuals;
    for (id, unit) in game.sim.units() {
        let current = current_layout[&id];
        let placement = if previous.0.contains_key(&id) {
            previous_layout[&id].interpolate(current, fixed.overstep_fraction())
        } else {
            current
        };
        if let Some(&entity) = entities.get(&id) {
            if let Ok(mut transform) = transforms.get_mut(entity) {
                *transform = placement_transform(placement);
            }
            continue;
        }
        let mesh = mesh_cache.entry(unit.kind).or_insert_with(|| {
            let radius = game.sim.rules().unit(unit.kind).radius;
            meshes.add(Circle::new(to_world_distance(radius)))
        });
        let material = material_cache
            .entry(unit.owner)
            .or_insert_with(|| materials.add(graphics.player_color(unit.owner.0)));
        let entity = commands
            .spawn_scene(unit_scene(id, mesh.clone(), material.clone(), placement))
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

const fn placement_transform(placement: Placement) -> Transform {
    Transform::from_translation(placement.position.extend(UNIT_Z)).with_scale(Vec3::new(
        placement.scale,
        placement.scale,
        1.0,
    ))
}

#[cfg(test)]
mod tests {
    use bevy::{scene::ScenePlugin, state::app::StatesPlugin};
    use rts_base::{Command, IVec2 as SimPos, MapData, Rules, Sim, TILE};

    use crate::map::to_world;

    use super::*;

    fn scene_app(sim: Sim) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            ScenePlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<ColorMaterial>()
        .insert_state(AppState::InGame)
        .insert_resource(Match::new(sim, PlayerId(0), true))
        .insert_resource(
            Graphics::from_ron(include_str!("../../../../assets/data/client.graphics.ron"))
                .unwrap(),
        )
        .init_resource::<PreviousPositions>()
        .add_plugins(UnitsPlugin);
        app
    }

    #[test]
    fn scene_visuals_follow_unit_and_match_lifetimes() {
        let rules =
            Rules::from_ron(include_str!("../../../../assets/data/core.rules.ron")).unwrap();
        let map = MapData::from_ron(
            r#"(name: "scenes", players: [(start: (0.5,0.5))],
            tiles: [".."], units: [
                (kind:"worker",owner:0,pos:(0.5,0.5)),
                (kind:"worker",owner:0,pos:(1.5,0.5))])"#,
        )
        .unwrap();
        let sim = Sim::new(rules, &map).unwrap();
        let hash = sim.state_hash();
        let mut app = scene_app(sim);
        app.update();

        let entities = app.world().resource::<UnitVisuals>().entities.clone();
        assert_eq!(entities.len(), 2);
        for (&id, &entity) in &entities {
            let world = app.world();
            assert_eq!(world.get::<UnitVisual>(entity).unwrap().0, id);
            assert!(world.get::<Mesh2d>(entity).is_some());
            assert!(world.get::<MeshMaterial2d<ColorMaterial>>(entity).is_some());
            assert_eq!(
                world.get::<Transform>(entity).unwrap().translation,
                to_world(world.resource::<Match>().sim.unit(id).unwrap().pos).extend(UNIT_Z)
            );
        }
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
        app.update();
        let visuals = app.world().resource::<UnitVisuals>();
        assert_eq!(visuals.entities, entities);
        assert_eq!(visuals.meshes.len(), 1);
        assert_eq!(visuals.materials.len(), 1);

        app.world_mut()
            .resource_mut::<Match>()
            .sim
            .remove_unit(UnitId(0));
        app.update();
        assert!(app.world().get_entity(entities[&UnitId(0)]).is_err());
        assert_eq!(app.world().resource::<UnitVisuals>().entities.len(), 1);

        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::Menu);
        app.update();
        assert!(app.world().get_entity(entities[&UnitId(1)]).is_err());
        let visuals = app.world().resource::<UnitVisuals>();
        assert!(visuals.entities.is_empty());
        assert!(visuals.meshes.is_empty());
        assert!(visuals.materials.is_empty());
    }

    #[test]
    fn configured_palette_drives_shared_materials_without_changing_simulation() {
        let rules =
            Rules::from_ron(include_str!("../../../../assets/data/core.rules.ron")).unwrap();
        let map = MapData::from_ron(
            r#"(name: "palette", players: [(start: (0.5,0.5)), (start: (1.5,0.5))],
            tiles: [".."], units: [
                (kind:"worker",owner:0,pos:(0.5,0.5)),
                (kind:"worker",owner:0,pos:(0.5,0.5)),
                (kind:"worker",owner:1,pos:(1.5,0.5))])"#,
        )
        .unwrap();
        let sim = Sim::new(rules, &map).unwrap();
        let hash = sim.state_hash();
        let mut app = scene_app(sim);
        let mut graphics =
            Graphics::from_ron(include_str!("../../../../assets/data/client.graphics.ron"))
                .unwrap();
        let custom_color = Color::srgb(0.1, 0.2, 0.3);
        graphics.player_colors = vec![custom_color];
        app.insert_resource(graphics);
        app.update();

        let world = app.world();
        let visuals = world.resource::<UnitVisuals>();
        let materials = world.resource::<Assets<ColorMaterial>>();
        assert_eq!(visuals.materials.len(), 2);
        for (id, unit) in world.resource::<Match>().sim.units() {
            let material = world
                .get::<MeshMaterial2d<ColorMaterial>>(visuals.entities[&id])
                .unwrap();
            assert_eq!(material.0, visuals.materials[&unit.owner]);
            assert_eq!(materials.get(&material.0).unwrap().color, custom_color);
        }
        assert_eq!(world.resource::<Match>().sim.state_hash(), hash);
    }

    #[test]
    fn grouped_visuals_use_offsets_and_scale_without_changing_occupancy() {
        let rules =
            Rules::from_ron(include_str!("../../../../assets/data/core.rules.ron")).unwrap();
        let map = MapData::from_ron(
            r#"(name: "group", players: [(start: (0.5,0.5)), (start: (1.5,0.5))],
            tiles: [".."], units: [
                (kind:"worker",owner:0,pos:(0.5,0.5)),
                (kind:"worker",owner:1,pos:(0.5,0.5)),
                (kind:"soldier",owner:0,pos:(0.5,0.5))])"#,
        )
        .unwrap();
        let sim = Sim::new(rules, &map).unwrap();
        let hash = sim.state_hash();
        let mut app = scene_app(sim);
        app.update();
        let entities = app.world().resource::<UnitVisuals>().entities.clone();
        let transforms: Vec<_> = entities
            .values()
            .map(|&entity| *app.world().get::<Transform>(entity).unwrap())
            .collect();
        assert!(transforms.iter().all(|transform| transform.scale.x < 1.0));
        for (index, transform) in transforms.iter().enumerate() {
            assert!(
                transforms[index + 1..]
                    .iter()
                    .all(|other| other.translation != transform.translation)
            );
        }
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);

        for id in [UnitId(1), UnitId(2)] {
            app.world_mut().resource_mut::<Match>().sim.remove_unit(id);
        }
        // A cleared snapshot (including a stalled session) draws the current layout directly.
        app.update();
        let transform = app.world().get::<Transform>(entities[&UnitId(0)]).unwrap();
        assert_eq!(transform.scale, Vec3::ONE);
        assert_eq!(
            transform.translation,
            to_world(SimPos::splat(TILE / 2)).extend(UNIT_Z)
        );
    }

    #[test]
    fn configured_bounds_drive_scene_placement_and_scale() {
        let rules = Rules::from_ron(&include_str!("../../../../assets/data/core.rules.ron").replace(
            "radius: 0.3,",
            "radius: 0.125, visual_bounds: Some((offset: (0.125, -0.25), size: (0.875, 0.25))),",
        )).unwrap();
        let map = MapData::from_ron(
            r#"(name: "configured bounds", players: [(start: (0.5,0.5))], tiles: ["."],
            units: [(kind:"worker",owner:0,pos:(0.5,0.5)),
                    (kind:"worker",owner:0,pos:(0.5,0.5))])"#,
        )
        .unwrap();
        let sim = Sim::new(rules, &map).unwrap();
        let hash = sim.state_hash();
        let bounds = UnitBounds::from_definition(sim.rules().unit(UnitKind(0)));
        assert_eq!(bounds.offset, Vec2::new(16.0, 32.0));
        assert_eq!(bounds.size, Vec2::new(112.0, 32.0));
        let mut app = scene_app(sim);
        app.update();
        let entities = &app.world().resource::<UnitVisuals>().entities;
        let rectangles: Vec<_> = entities
            .values()
            .map(|&entity| {
                let transform = app.world().get::<Transform>(entity).unwrap();
                assert_eq!(transform.scale, Vec3::ONE);
                let rect = bounds.world_rect(transform);
                assert_eq!(
                    rect.center(),
                    transform.translation.truncate() + bounds.offset
                );
                assert!(rect.min.cmpge(Vec2::new(0.0, -128.0)).all());
                assert!(rect.max.cmple(Vec2::new(128.0, 0.0)).all());
                rect
            })
            .collect();
        assert_eq!(rectangles[0].center().x, rectangles[1].center().x);
        assert!(
            rectangles[0].max.y <= rectangles[1].min.y
                || rectangles[1].max.y <= rectangles[0].min.y
        );
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
    }

    #[test]
    fn interpolates_tick_snapshots_without_changing_simulation() {
        let rules =
            Rules::from_ron(include_str!("../../../../assets/data/core.rules.ron")).unwrap();
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
        let previous = Placement {
            position: to_world(start),
            scale: 1.0,
        };
        let current = Placement {
            position: to_world(end),
            scale: 0.75,
        };
        assert_eq!(previous.interpolate(current, 0.0).position, to_world(start));
        let middle = previous.interpolate(current, 0.5);
        assert_eq!(middle.position, (to_world(start) + to_world(end)) / 2.0);
        assert_eq!(middle.scale, 0.875);
        assert_eq!(previous.interpolate(current, 1.0).position, to_world(end));
        assert_eq!(
            previous.interpolate(current, -1.0).position,
            to_world(start)
        );
        assert_eq!(previous.interpolate(current, 2.0).position, to_world(end));
        assert_eq!(sim.state_hash(), hash);
    }
}
