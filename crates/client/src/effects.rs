//! Combat feedback: pixel-art tracers and impacts for the volleys the simulation fired.
//!
//! Each shot of a recorded [`Volley`] becomes a short-lived effect: a muzzle flash at the
//! soldier that fired, a three-block tracer flying to the soldier it hit, and an impact puff
//! there. Endpoints are fixed when the effect spawns, so a tracer ends where its target stood
//! when the shot landed. Effects are presentation only and never touch the simulation.

use bevy::{platform::collections::HashMap, prelude::*};
use rts_base::{TICK_RATE, UnitId, Volley};

use crate::{
    camera::RtsCamera,
    game::{AppState, Match},
    map::to_world,
    units::{SyncUnits, UnitVisual, soldier_point},
};

/// Size of one effect art pixel in world pixels, matching the terrain's art grid.
const ART_PIXEL: f32 = 4.0;
/// Effect pixels never shrink below this many screen pixels when zoomed out.
const MIN_SCREEN_PIXELS: f32 = 2.0;
/// Above unit roots (1.0) and their soldiers.
const EFFECT_Z: f32 = 2.0;

const TRACER_SPEED: f32 = 2400.0;
const MIN_FLIGHT: f32 = 0.04;
const MAX_FLIGHT: f32 = 0.12;
const MUZZLE_SECONDS: f32 = 0.05;
const IMPACT_SECONDS: f32 = 0.12;
/// Soldiers of a squad fire in a quick ripple rather than all in one frame.
const SHOT_STAGGER: f32 = 0.018;
/// Volleys older than this when drawn are skipped; their effects would already be over.
const RECENT_TICKS: u32 = 4;
/// Upper bound on live shot effects, so heavy fights and catch-up stay cheap.
const MAX_SHOTS: usize = 192;

/// Neutral warm tones that read on every terrain and do not resemble any player color.
const FLASH: Color = Color::srgb(1.0, 0.95, 0.78);
const TRACER_HEAD: Color = Color::srgb(1.0, 0.86, 0.55);
const TRACER_TAIL: Color = Color::srgb(0.85, 0.55, 0.28);
const IMPACT: Color = Color::srgb(1.0, 0.72, 0.38);
const IMPACT_FADE: Color = Color::srgb(0.62, 0.45, 0.32);

/// Blocks per shot: a muzzle flash plus a three-block tracer, reused for the five-block impact.
const BLOCKS: usize = 5;

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        // Spawn before units sync, so a target destroyed by this volley still has its visual.
        app.add_systems(
            Update,
            (spawn_shots.before(SyncUnits), animate_shots)
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// One shot in flight or landing, in client world space.
#[derive(Component, Debug)]
pub struct ShotEffect {
    from: Vec2,
    to: Vec2,
    /// Seconds of virtual time when the soldier fired.
    fired: f32,
    flight: f32,
}

impl ShotEffect {
    fn new(from: Vec2, to: Vec2, fired: f32) -> Self {
        let flight = (from.distance(to) / TRACER_SPEED).clamp(MIN_FLIGHT, MAX_FLIGHT);
        Self {
            from,
            to,
            fired,
            flight,
        }
    }

    const fn duration(&self) -> f32 {
        self.flight + IMPACT_SECONDS
    }

    /// Blocks to draw `age` seconds after firing, as centers and colors on the art grid.
    fn blocks(&self, age: f32, block: f32) -> Vec<(Vec2, Color)> {
        let mut blocks = Vec::with_capacity(BLOCKS);
        if age < 0.0 || age >= self.duration() {
            return blocks;
        }
        if age < self.flight {
            if age < MUZZLE_SECONDS {
                blocks.push((snap(self.from, block), FLASH));
            }
            let direction = (self.to - self.from).normalize_or_zero();
            let head = self.from.lerp(self.to, age / self.flight);
            for (index, color) in [TRACER_HEAD, TRACER_TAIL, TRACER_TAIL]
                .into_iter()
                .enumerate()
            {
                #[expect(clippy::cast_precision_loss, reason = "index is below 3")]
                let back = direction * block * index as f32;
                // The tail never trails behind the muzzle.
                if back.length_squared() <= (head - self.from).length_squared() {
                    blocks.push((snap(head - back, block), color));
                }
            }
        } else if age < self.flight + IMPACT_SECONDS / 2.0 {
            let center = snap(self.to, block);
            blocks.push((center, FLASH));
            for side in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
                blocks.push((center + side * block, IMPACT));
            }
        } else {
            blocks.push((snap(self.to, block), IMPACT_FADE));
        }
        blocks
    }
}

/// Aligns a point to the effect pixel grid, so blocks land on whole pixels like the art.
fn snap(point: Vec2, block: f32) -> Vec2 {
    (point / block).round() * block
}

/// Effect pixel size for the camera's zoom: one art pixel, or larger when zoomed out.
fn block_size(scale: f32) -> f32 {
    ART_PIXEL.max(MIN_SCREEN_PIXELS * scale)
}

/// Turns volleys recorded since the last frame into shot effects.
fn spawn_shots(
    mut commands: Commands,
    mut game: ResMut<Match>,
    time: Res<Time>,
    roots: Query<(&UnitVisual, &Transform)>,
    live: Query<(), With<ShotEffect>>,
) {
    if game.volleys.is_empty() {
        return;
    }
    let volleys = std::mem::take(&mut game.volleys);
    let roots: HashMap<UnitId, &Transform> = roots
        .iter()
        .map(|(visual, transform)| (visual.0, transform))
        .collect();
    let current = game.view.tick();
    let now = time.elapsed_secs();
    let mut room = MAX_SHOTS.saturating_sub(live.iter().count());
    // Newest volleys first, so catch-up keeps the shots that are still visible.
    for (tick, volley) in volleys.iter().rev() {
        let behind = current.saturating_sub(*tick);
        if behind >= RECENT_TICKS {
            break;
        }
        #[expect(clippy::cast_precision_loss, reason = "behind is below RECENT_TICKS")]
        let fired = now - behind as f32 / TICK_RATE as f32;
        for (index, (from, to)) in shot_lines(&game, volley, &roots).enumerate() {
            if room == 0 {
                return;
            }
            room -= 1;
            #[expect(
                clippy::cast_precision_loss,
                reason = "squads have at most 255 soldiers"
            )]
            let effect = ShotEffect::new(from, to, (index as f32).mul_add(SHOT_STAGGER, fired));
            commands.spawn((
                effect,
                Transform::default(),
                Visibility::default(),
                DespawnOnExit(AppState::InGame),
                Children::spawn(SpawnIter((0..BLOCKS).map(|_| {
                    (
                        Sprite::from_color(Color::NONE, Vec2::ONE),
                        Transform::default(),
                        Visibility::Hidden,
                    )
                }))),
            ));
        }
    }
}

/// Start and end of each shot: the firing soldier as drawn, to the soldier it hit.
fn shot_lines<'a>(
    game: &'a Match,
    volley: &'a Volley,
    roots: &'a HashMap<UnitId, &Transform>,
) -> impl Iterator<Item = (Vec2, Vec2)> + 'a {
    let rules = game.view.rules();
    let shooter = rules.unit(volley.shooter_kind);
    let target = rules.unit(volley.target_kind);
    // A unit without a visual yet stands exactly on its simulation position.
    let root = |id: UnitId, pos| {
        roots.get(&id).map_or_else(
            || Transform::from_translation(to_world(pos).extend(0.0)),
            |t| **t,
        )
    };
    let (from, to) = (
        root(volley.shooter, volley.from),
        root(volley.target, volley.to),
    );
    volley.shots.iter().map(move |shot| {
        (
            soldier_point(&from, shooter, shot.soldier),
            soldier_point(&to, target, shot.hit),
        )
    })
}

/// Moves each shot's blocks along its path and removes finished shots.
fn animate_shots(
    mut commands: Commands,
    time: Res<Time>,
    camera: Query<&Projection, With<RtsCamera>>,
    shots: Query<(Entity, &ShotEffect, &Children)>,
    mut blocks: Query<(&mut Transform, &mut Sprite, &mut Visibility)>,
) {
    let scale = match camera.single() {
        Ok(Projection::Orthographic(projection)) => projection.scale,
        _ => 1.0,
    };
    let block = block_size(scale);
    let now = time.elapsed_secs();
    for (entity, shot, children) in &shots {
        let age = now - shot.fired;
        if age >= shot.duration() {
            commands.entity(entity).despawn();
            continue;
        }
        let drawn = shot.blocks(age, block);
        for (index, child) in children.iter().enumerate() {
            let Ok((mut transform, mut sprite, mut visibility)) = blocks.get_mut(child) else {
                continue;
            };
            match drawn.get(index) {
                Some(&(center, color)) => {
                    transform.translation = center.extend(EFFECT_Z);
                    sprite.color = color;
                    sprite.custom_size = Some(Vec2::splat(block));
                    visibility.set_if_neq(Visibility::Inherited);
                }
                None => {
                    visibility.set_if_neq(Visibility::Hidden);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::{scene::ScenePlugin, state::app::StatesPlugin, time::TimeUpdateStrategy};
    use rts_assets::Graphics;
    use rts_base::{MapData, PlayerId, Rules, Sim};

    use super::*;
    use crate::{
        map::{UpdateZoomDetail, ZoomDetail},
        units::UnitsPlugin,
    };

    /// Two facing squads of two soldiers that fire every tick.
    fn duel() -> Sim {
        let rules = Rules::from_ron(
            r##"(tile_capacity: 4, terrain: [
                (id: "grass", symbol: '.', speed_modifier: 1, color: "#00ff00"),
            ], units: [
                (id: "rifles", sprite: "r.png", size: 2, squad: 2, max_health: 1000, radius: 0.4,
                 speed: 2, weapon: Some((range: 2, damage: 1, cooldown: 0.05))),
            ])"##,
        )
        .unwrap();
        let map = MapData::from_ron(
            r#"(name: "duel", players: [(start: (0.5, 0.5)), (start: (2.5, 0.5))],
                tiles: ["...."], units: [
                    (kind: "rifles", owner: 0, pos: (0.5, 0.5)),
                    (kind: "rifles", owner: 1, pos: (2.5, 0.5)),
                ])"#,
        )
        .unwrap();
        Sim::new(rules, &map).unwrap()
    }

    fn app(sim: Sim) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            StatesPlugin,
            ScenePlugin,
        ))
        .init_resource::<ZoomDetail>()
        .configure_sets(Update, UpdateZoomDetail)
        .init_asset::<Image>()
        .insert_state(AppState::InGame)
        .insert_resource(Match::new(sim, PlayerId(0), true))
        .insert_resource(
            Graphics::from_ron(include_str!("../../../assets/data/client.graphics.ron")).unwrap(),
        )
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )))
        .add_plugins((UnitsPlugin, EffectsPlugin));
        app
    }

    fn step(app: &mut App, ticks: u32) {
        let mut game = app.world_mut().resource_mut::<Match>();
        for _ in 0..ticks {
            game.step();
        }
    }

    fn live_shots(app: &mut App) -> Vec<(Vec2, Vec2, f32)> {
        app.world_mut()
            .query::<&ShotEffect>()
            .iter(app.world())
            .map(|shot| (shot.from, shot.to, shot.fired))
            .collect()
    }

    #[test]
    fn volleys_spawn_shots_between_drawn_soldiers_and_expire() {
        let mut app = app(duel());
        app.update();
        step(&mut app, 1);
        app.update();
        let shots = live_shots(&mut app);
        // Both squads fire both soldiers; each shot leaves its own soldier.
        assert_eq!(shots.len(), 4);
        assert_eq!(app.world().resource::<Match>().volleys.len(), 0);
        // Lone units stand on their simulation positions, so soldiers are drawn from there.
        let rules = app.world().resource::<Match>().view.rules().clone();
        let definition = rules.unit(rts_base::UnitKind(0));
        let soldiers: Vec<Vec2> = [64.0, 320.0]
            .into_iter()
            .flat_map(|x| {
                let root = Transform::from_xyz(x, -64.0, 1.0);
                (0..2).map(move |slot| soldier_point(&root, definition, slot))
            })
            .collect();
        let sorted = |mut points: Vec<Vec2>| {
            points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
            points
        };
        assert_eq!(sorted(shots.iter().map(|shot| shot.0).collect()), soldiers);
        assert!(shots.iter().all(|shot| soldiers.contains(&shot.1)));
        // Every tracer crosses to the other squad.
        assert!(
            shots
                .iter()
                .all(|(from, to, _)| (from.x < 192.0) != (to.x < 192.0))
        );
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(live_shots(&mut app).len(), 0);
    }

    #[test]
    fn catch_up_draws_only_recent_volleys_already_aged() {
        let mut app = app(duel());
        app.update();
        step(&mut app, 10);
        app.update();
        let shots = live_shots(&mut app);
        assert_eq!(shots.len(), 4 * RECENT_TICKS as usize);
        let now = app.world().resource::<Time>().elapsed_secs();
        let oldest = shots.iter().map(|shot| shot.2).fold(f32::MAX, f32::min);
        #[expect(clippy::cast_precision_loss, reason = "small tick counts")]
        let behind = (RECENT_TICKS - 1) as f32 / TICK_RATE as f32;
        assert!((now - oldest - behind).abs() < 1e-4);
        assert_eq!(app.world().resource::<Match>().volleys.len(), 0);
    }

    #[test]
    fn tracer_flies_from_shooter_to_target_then_lands_as_an_impact() {
        let shot = ShotEffect::new(Vec2::ZERO, Vec2::new(240.0, 0.0), 0.0);
        assert!((shot.flight - 0.1).abs() < 1e-6);
        assert_eq!(shot.blocks(-0.01, ART_PIXEL), Vec::new());
        let start = shot.blocks(0.0, ART_PIXEL);
        assert_eq!(start, [(Vec2::ZERO, FLASH), (Vec2::ZERO, TRACER_HEAD)]);
        let middle = shot.blocks(0.06, ART_PIXEL);
        assert_eq!(middle[0], (Vec2::new(144.0, 0.0), TRACER_HEAD));
        assert_eq!(middle[1], (Vec2::new(140.0, 0.0), TRACER_TAIL));
        let impact = shot.blocks(0.11, ART_PIXEL);
        assert_eq!(impact.len(), 5);
        assert_eq!(impact[0], (Vec2::new(240.0, 0.0), FLASH));
        assert_eq!(
            shot.blocks(0.2, ART_PIXEL),
            [(Vec2::new(240.0, 0.0), IMPACT_FADE)]
        );
        assert_eq!(shot.blocks(shot.duration(), ART_PIXEL), Vec::new());
    }

    #[test]
    fn blocks_stay_on_the_pixel_grid_and_visible_when_zoomed_out() {
        assert_eq!(block_size(0.5), ART_PIXEL);
        assert_eq!(block_size(8.0), 16.0);
        let shot = ShotEffect::new(Vec2::new(3.0, -5.0), Vec2::new(101.0, 77.0), 0.0);
        for age in [0.0, 0.02, 0.05, 0.08, 0.13] {
            for block in [ART_PIXEL, 16.0] {
                for (center, _) in shot.blocks(age, block) {
                    assert_eq!(center, snap(center, block));
                }
            }
        }
    }
}
