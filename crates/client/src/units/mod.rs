//! Unit rendering from simulation state.

mod layout;
mod motion;

use bevy::{platform::collections::HashMap, prelude::*, sprite::Anchor};
use rts_assets::Graphics;
use rts_base::{UnitId, UnitKind};

pub use self::layout::UnitBounds;
use self::{
    layout::{Placement, layout_travelling, layout_units},
    motion::UnitMotion,
};

use crate::{
    game::{AppState, Match},
    map::{UpdateZoomDetail, ZoomDetail, to_world},
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
                    .after(UpdateZoomDetail)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnExit(AppState::InGame), clear_units);
    }
}

/// Entities drawing simulation units, plus shared sprite images.
#[derive(Resource, Default)]
struct UnitVisuals {
    entities: HashMap<UnitId, Entity>,
    images: HashMap<UnitKind, Handle<Image>>,
    motion: HashMap<UnitId, UnitMotion>,
}

/// The sprite and distant rectangle share exactly the same size and anchor.
fn unit_scene(id: UnitId, sprite: Sprite, bounds: UnitBounds, placement: Placement) -> impl Scene {
    let visual = UnitVisual(id);
    let transform = placement_transform(placement);
    let anchor = Anchor(-bounds.offset / bounds.size);
    bsn! {
        DespawnOnExit<AppState>(AppState::InGame)
        visual
        Sprite {
            image: {sprite.image},
            color: {sprite.color},
            custom_size: {sprite.custom_size},
        }
        anchor
        transform
    }
}

/// Spawns, moves, and despawns unit entities to match the simulation.
fn sync_units(
    mut commands: Commands,
    game: Res<Match>,
    graphics: Res<Graphics>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    mut visuals: ResMut<UnitVisuals>,
    assets: Res<AssetServer>,
    detail: Res<ZoomDetail>,
    mut sprites: Query<(&mut Transform, &mut Sprite)>,
) {
    let mut current_layout = layout_units(
        game.sim
            .units()
            .filter(|(id, _)| game.sim.move_target(*id).is_none())
            .map(|(id, unit)| {
                (
                    id,
                    unit.pos,
                    UnitBounds::from_definition(game.sim.rules().unit(unit.kind)),
                )
            }),
    );
    current_layout.extend(layout_travelling(
        game.sim
            .units()
            .filter(|(id, _)| game.sim.move_target(*id).is_some())
            .map(|(id, unit)| {
                (
                    id,
                    unit.pos,
                    UnitBounds::from_definition(game.sim.rules().unit(unit.kind)),
                )
            }),
    ));
    let UnitVisuals {
        entities,
        images,
        motion,
    } = &mut *visuals;
    for (id, unit) in game.sim.units() {
        let definition = game.sim.rules().unit(unit.kind);
        let bounds = UnitBounds::from_definition(definition);
        let position = to_world(unit.pos);
        let target = current_layout[&id];
        let motion = motion.entry(id).or_insert_with(|| {
            UnitMotion::new(position, target, game.sim.tick(), fixed.elapsed_secs_f64())
        });
        motion.observe(position, game.sim.tick(), fixed.elapsed_secs_f64());
        let placement = motion.draw(
            target,
            time.elapsed_secs_f64(),
            fixed.timestep().as_secs_f64(),
            time.delta_secs(),
        );
        let image = images
            .entry(unit.kind)
            .or_insert_with(|| assets.load(definition.sprite.clone()));
        let desired_image = if detail.0 {
            image.clone()
        } else {
            Handle::default()
        };
        if let Some(&entity) = entities.get(&id) {
            if let Ok((mut transform, mut sprite)) = sprites.get_mut(entity) {
                *transform = placement_transform(placement);
                if sprite.image != desired_image {
                    sprite.image = desired_image;
                }
            }
            continue;
        }
        let sprite = Sprite {
            image: desired_image,
            color: graphics.player_color(unit.owner.0),
            custom_size: Some(bounds.size),
            ..default()
        };
        let entity = commands
            .spawn_scene(unit_scene(id, sprite, bounds, placement))
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
    motion.retain(|id, _| entities.contains_key(id));
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
    use bevy::{scene::ScenePlugin, state::app::StatesPlugin, time::TimeUpdateStrategy};
    use rts_base::{Command, IVec2 as SimPos, MapData, PlayerId, Rules, Sim, TILE};

    use crate::{camera::RtsCamera, map::update_zoom_detail};

    use super::*;

    fn scene_app(sim: Sim) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            ScenePlugin,
        ))
        .init_resource::<ZoomDetail>()
        .add_systems(Update, update_zoom_detail.in_set(UpdateZoomDetail))
        .init_asset::<Image>()
        .insert_state(AppState::InGame)
        .insert_resource(Match::new(sim, PlayerId(0), true))
        .insert_resource(
            Graphics::from_ron(include_str!("../../../../assets/data/client.graphics.ron"))
                .unwrap(),
        )
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .add_plugins(UnitsPlugin);
        app
    }

    #[test]
    fn required_bundled_sprites_decode_with_transparent_backgrounds() {
        use bevy::{
            asset::RenderAssetUsages,
            image::{CompressedImageFormats, ImageSampler, ImageType},
        };
        let rules =
            Rules::from_ron(include_str!("../../../../assets/data/core.rules.ron")).unwrap();
        for kind in [UnitKind(0), UnitKind(1)] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets")
                .join(&rules.unit(kind).sprite);
            let image = Image::from_buffer(
                &std::fs::read(path).unwrap(),
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::linear(),
                RenderAssetUsages::MAIN_WORLD,
            )
            .unwrap();
            let data = image.data.unwrap();
            assert!(data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 0));
            assert!(data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 255));
        }
    }

    #[test]
    fn rendered_travel_keeps_constant_speed_through_tile_centers() {
        for fps in [30_u16, 60, 120, 144] {
            let rules =
                Rules::from_ron(include_str!("../../../../assets/data/core.rules.ron")).unwrap();
            let map = MapData::from_ron(
                r#"(name: "steady travel", players: [(start: (0.5,0.5))],
                tiles: ["........."], units: [(kind:"worker",owner:0,pos:(0.5,0.5))])"#,
            )
            .unwrap();
            let mut sim = Sim::new(rules, &map).unwrap();
            sim.queue_command(
                PlayerId(0),
                Command::Move {
                    units: vec![UnitId(0)],
                    target: SimPos::new(15 * TILE / 2, TILE / 2),
                },
            );
            let mut app = scene_app(sim);
            app.insert_resource(Time::<Fixed>::from_hz(f64::from(rts_base::TICK_RATE)))
                .insert_resource(TimeUpdateStrategy::ManualDuration(
                    std::time::Duration::from_secs_f64(1.0 / f64::from(fps)),
                ))
                .add_systems(FixedUpdate, |mut game: ResMut<Match>| game.sim.step());
            let mut last = None;
            for _ in 0..fps * 5 / 2 {
                app.update();
                let entity = app.world().resource::<UnitVisuals>().entities[&UnitId(0)];
                let position = app.world().get::<Transform>(entity).unwrap().translation;
                let now = app.world().resource::<Time>().elapsed_secs();
                if let Some(previous) = last
                    && now > 0.3
                {
                    let step: Vec3 = position - previous;
                    let expected = 320.0 / f32::from(fps);
                    assert!(
                        (step.x - expected).abs() < 0.02,
                        "{fps} FPS at {now}s: step {} instead of {expected}",
                        step.x
                    );
                    assert_eq!(step.y, 0.0);
                }
                last = Some(position);
            }
        }
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
            assert!(world.get::<Sprite>(entity).is_some());
            assert_eq!(
                world.get::<Transform>(entity).unwrap().translation,
                to_world(world.resource::<Match>().sim.unit(id).unwrap().pos).extend(UNIT_Z)
            );
        }
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
        app.update();
        let visuals = app.world().resource::<UnitVisuals>();
        assert_eq!(visuals.entities, entities);
        assert_eq!(visuals.images.len(), 1);

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
        assert!(visuals.images.is_empty());
    }

    #[test]
    fn configured_palette_tints_shared_sprites_without_changing_simulation() {
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
        assert_eq!(visuals.images.len(), 1);
        for (id, unit) in world.resource::<Match>().sim.units() {
            let sprite = world.get::<Sprite>(visuals.entities[&id]).unwrap();
            assert_eq!(sprite.image, visuals.images[&unit.kind]);
            assert_eq!(sprite.color, custom_color);
        }
        assert_eq!(world.resource::<Match>().sim.state_hash(), hash);
    }

    #[test]
    fn zoom_switches_sprites_to_exact_bounds_with_hysteresis() {
        let rules = Rules::from_ron(
            &include_str!("../../../../assets/data/core.rules.ron").replace(
                "radius: 0.3,",
                "radius: 0.3, visual_bounds: Some((offset: (0.125, -0.0625), size: (0.75, 0.25))),",
            ),
        )
        .unwrap();
        let map = MapData::from_ron(
            r#"(name: "detail", players: [(start: (0.5,0.5))], tiles: ["."],
            units: [(kind:"worker",owner:0,pos:(0.5,0.5)),
                    (kind:"soldier",owner:0,pos:(0.5,0.5))])"#,
        )
        .unwrap();
        let sim = Sim::new(rules, &map).unwrap();
        let hash = sim.state_hash();
        let bounds = UnitBounds::from_definition(sim.rules().unit(UnitKind(0)));
        let mut app = scene_app(sim);
        let camera = app.world_mut().spawn(RtsCamera::default()).id();
        app.update();
        let entity = app.world().resource::<UnitVisuals>().entities[&UnitId(0)];
        let image = app.world().get::<Sprite>(entity).unwrap().image.clone();
        assert_ne!(image, Handle::default());
        assert_eq!(
            app.world().get::<Sprite>(entity).unwrap().custom_size,
            Some(bounds.size)
        );
        let anchor = app.world().get::<Anchor>(entity).unwrap();
        assert_eq!(-anchor.0 * bounds.size, bounds.offset);
        let original = *app.world().get::<Transform>(entity).unwrap();
        for (pixels, detailed) in [
            (64.0, true),
            (40.0, true),
            (39.0, false),
            (44.0, false),
            (48.0, true),
        ] {
            let mut projection = app.world_mut().get_mut::<Projection>(camera).unwrap();
            let Projection::Orthographic(ortho) = projection.as_mut() else {
                panic!("orthographic camera")
            };
            ortho.scale = to_world(SimPos::new(TILE, 0)).x / pixels;
            app.update();
            let visuals = app.world().resource::<UnitVisuals>();
            for (&id, &unit_entity) in &visuals.entities {
                let kind = app.world().resource::<Match>().sim.unit(id).unwrap().kind;
                let expected = if detailed {
                    visuals.images[&kind].clone()
                } else {
                    Handle::default()
                };
                assert_eq!(
                    app.world().get::<Sprite>(unit_entity).unwrap().image,
                    expected
                );
            }
            assert_eq!(app.world().resource::<ZoomDetail>().0, detailed);
            let sprite = app.world().get::<Sprite>(entity).unwrap();
            assert_eq!(
                sprite.image,
                if detailed {
                    image.clone()
                } else {
                    Handle::default()
                }
            );
            assert_eq!(*app.world().get::<Transform>(entity).unwrap(), original);
            assert_eq!(sprite.custom_size, Some(bounds.size));
        }
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
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
        // Regroup from the last drawn placement, even while the simulation is stationary.
        let before = *app.world().get::<Transform>(entities[&UnitId(0)]).unwrap();
        app.update();
        let changing = *app.world().get::<Transform>(entities[&UnitId(0)]).unwrap();
        assert!(changing.scale.x > before.scale.x && changing.scale.x < 1.0);
        for _ in 0..120 {
            app.update();
        }
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
