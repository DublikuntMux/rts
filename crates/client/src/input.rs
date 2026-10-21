//! Mouse and touch selection. Gameplay actions only enqueue simulation commands.

use std::collections::BTreeSet;

use bevy::{
    input::touch::{Touch, Touches},
    prelude::*,
    window::PrimaryWindow,
};
use rts_base::{Command, IVec2 as SimPos, UnitId};

use crate::{
    camera::{CameraControl, RtsCamera},
    game::{AppState, Match},
    hud::HUD_HEIGHT,
    lobby::SESSION_BAR_HEIGHT,
    map::TILE_PX,
    units::{SyncUnits, UnitBounds, UnitVisual},
};

const SELECTION_COLOR: Color = Color::srgb(0.95, 0.83, 0.45);

#[derive(Resource, Default)]
pub struct Selection {
    pub units: BTreeSet<UnitId>,
    pub touch_mode: bool,
}

impl Selection {
    pub fn stop(&self, game: &mut Match) {
        if !self.units.is_empty() {
            game.queue_command(Command::Stop {
                units: self.units.iter().copied().collect(),
            });
        }
    }
}

pub struct SelectionPlugin;

impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<PointerState>()
            .init_resource::<OrderFeedback>()
            .add_systems(
                Update,
                control_units
                    .after(CameraControl)
                    .after(SyncUnits)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                PostUpdate,
                draw_selection.run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnExit(AppState::InGame), reset_pointer);
    }
}

fn reset_pointer(mut pointer: ResMut<PointerState>, mut feedback: ResMut<OrderFeedback>) {
    *pointer = PointerState::default();
    feedback.0 = None;
}

#[derive(Clone, Copy)]
struct Press {
    screen_start: Vec2,
    world_start: Vec2,
    current: Vec2,
    dragged: bool,
    additive: bool,
}

impl Press {
    const fn new(screen: Vec2, world: Vec2, additive: bool) -> Self {
        Self {
            screen_start: screen,
            world_start: world,
            current: screen,
            dragged: false,
            additive,
        }
    }

    fn update(&mut self, position: Vec2, threshold: f32) {
        self.current = position;
        self.dragged |= self.screen_start.distance_squared(position) >= threshold * threshold;
    }
}

#[derive(Resource, Default)]
struct PointerState {
    mouse: Option<Press>,
    finger: Option<(u64, Press)>,
    touch_blocked: bool,
    suppress_mouse: bool,
}

impl PointerState {
    fn touch_press(&mut self, touches: &Touches, view: &ScreenView) -> Option<Press> {
        self.suppress_mouse = true;
        self.mouse = None;
        // Include released fingers so a rapid second finger cannot become a tap.
        let fingers: BTreeSet<_> = touches
            .iter()
            .chain(touches.iter_just_pressed())
            .chain(touches.iter_just_released())
            .map(Touch::id)
            .collect();
        if fingers.len() > 1 || touches.any_just_canceled() {
            self.touch_blocked = true;
            self.finger = None;
        }
        let mut completed = None;
        if !self.touch_blocked {
            if self.finger.is_none()
                && let Some(touch) = touches.iter_just_pressed().next()
                && view.playable(touch.start_position())
            {
                self.finger = Some((
                    touch.id(),
                    Press::new(
                        touch.start_position(),
                        view.world(touch.start_position()),
                        false,
                    ),
                ));
            }
            if let Some((id, press)) = &mut self.finger {
                if let Some(touch) = touches.get_pressed(*id) {
                    press.update(touch.position(), 14.0);
                }
                if let Some(touch) = touches.get_released(*id) {
                    press.update(touch.position(), 14.0);
                    completed = view.playable(press.current).then_some(*press);
                    self.finger = None;
                }
            }
        }
        if touches.iter().next().is_none() {
            self.touch_blocked = false;
            self.finger = None;
        }
        completed
    }
}

#[derive(Resource, Default)]
struct OrderFeedback(Option<(Vec2, bool, f64)>);

/// Screen conversion uses the camera's current transform, including this frame's pan/zoom.
/// This avoids reading a `GlobalTransform` that is only propagated later in `PostUpdate`.
struct ScreenView {
    center: Vec2,
    scale: f32,
    size: Vec2,
}

impl ScreenView {
    fn world(&self, screen: Vec2) -> Vec2 {
        let offset = screen - self.size / 2.0;
        self.center + Vec2::new(offset.x, -offset.y) * self.scale
    }

    fn playable(&self, screen: Vec2) -> bool {
        screen.cmpge(Vec2::new(0.0, SESSION_BAR_HEIGHT)).all()
            && screen.x < self.size.x
            && screen.y < self.size.y - HUD_HEIGHT
    }
}

fn control_units(
    mut game: ResMut<Match>,
    mut selection: ResMut<Selection>,
    mut pointer: ResMut<PointerState>,
    mut feedback: ResMut<OrderFeedback>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    touches: Res<Touches>,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Transform, &Projection), With<RtsCamera>>,
    visuals: Query<(&UnitVisual, &Transform)>,
) {
    let (transform, Projection::Orthographic(projection)) = camera.into_inner() else {
        return;
    };
    let view = ScreenView {
        center: transform.translation.truncate(),
        scale: projection.scale,
        size: window.size(),
    };
    selection.units.retain(|id| {
        game.sim
            .unit(*id)
            .is_some_and(|unit| unit.owner == game.local_player)
    });
    if !window.focused || keys.just_pressed(KeyCode::Escape) {
        pointer.mouse = None;
        pointer.finger = None;
        pointer.touch_blocked = touches.iter().next().is_some();
        if keys.just_pressed(KeyCode::Escape) {
            selection.units.clear();
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        selection.stop(&mut game);
    }

    let touching = touches.iter().next().is_some()
        || touches.any_just_pressed()
        || touches.any_just_released()
        || touches.any_just_canceled();
    if touching {
        selection.touch_mode = true;
        if let Some(press) = pointer.touch_press(&touches, &view) {
            finish_press(
                press,
                true,
                &view,
                &visuals,
                &mut selection,
                &mut game,
                &mut feedback,
                time.elapsed_secs_f64(),
            );
        }
        return;
    }
    if pointer.suppress_mouse {
        if !mouse.any_pressed([MouseButton::Left, MouseButton::Right]) {
            pointer.suppress_mouse = false;
        }
        return;
    }

    let additive = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if mouse.just_pressed(MouseButton::Left) {
        selection.touch_mode = false;
        pointer.mouse = window
            .cursor_position()
            .filter(|&position| view.playable(position))
            .map(|position| Press::new(position, view.world(position), additive));
    }
    if let Some(press) = &mut pointer.mouse {
        if let Some(position) = window.cursor_position() {
            press.update(position, 8.0);
        }
        if mouse.just_released(MouseButton::Left) {
            let press = *press;
            pointer.mouse = None;
            if window
                .cursor_position()
                .is_some_and(|position| view.playable(position))
            {
                finish_press(
                    press,
                    false,
                    &view,
                    &visuals,
                    &mut selection,
                    &mut game,
                    &mut feedback,
                    time.elapsed_secs_f64(),
                );
            }
        } else if !mouse.pressed(MouseButton::Left) {
            pointer.mouse = None;
        }
    }
    if mouse.just_pressed(MouseButton::Right)
        && let Some(position) = window
            .cursor_position()
            .filter(|&position| view.playable(position))
    {
        selection.touch_mode = false;
        issue_move(
            view.world(position),
            &selection,
            &mut game,
            &mut feedback,
            time.elapsed_secs_f64(),
        );
    }
}

fn finish_press(
    press: Press,
    touch: bool,
    view: &ScreenView,
    visuals: &Query<(&UnitVisual, &Transform)>,
    selection: &mut Selection,
    game: &mut Match,
    feedback: &mut OrderFeedback,
    now: f64,
) {
    let position = view.world(press.current);
    if press.dragged {
        let rect = Rect::from_corners(press.world_start, position);
        if !press.additive {
            selection.units.clear();
        }
        selection
            .units
            .extend(visuals.iter().filter_map(|(visual, transform)| {
                game.sim
                    .unit(visual.0)
                    .filter(|unit| {
                        if unit.owner != game.local_player {
                            return false;
                        }
                        let definition = game.sim.rules().unit(unit.kind);
                        let bounds = UnitBounds::from_definition(definition).world_rect(transform);
                        if definition.visual_bounds.is_some() {
                            rect.min.cmple(bounds.max).all() && rect.max.cmpge(bounds.min).all()
                        } else {
                            rect.contains(bounds.center())
                        }
                    })
                    .map(|_| visual.0)
            }));
    } else {
        let hit = visuals
            .iter()
            .filter_map(|(visual, transform)| {
                let unit = game.sim.unit(visual.0)?;
                if unit.owner != game.local_player {
                    return None;
                }
                let definition = game.sim.rules().unit(unit.kind);
                let bounds = UnitBounds::from_definition(definition).world_rect(transform);
                let distance = position.distance_squared(bounds.center());
                let minimum = if touch { 24.0 } else { 8.0 } * view.scale;
                let hit = if definition.visual_bounds.is_some() {
                    Rect::from_center_half_size(
                        bounds.center(),
                        bounds.half_size().max(Vec2::splat(minimum)),
                    )
                    .contains(position)
                } else {
                    let radius = (bounds.width() / 2.0).max(minimum);
                    distance <= radius * radius
                };
                hit.then_some((visual.0, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
            .map(|hit| hit.0);
        if let Some(id) = hit {
            if !press.additive {
                selection.units.clear();
            }
            if press.additive && selection.units.contains(&id) {
                selection.units.remove(&id);
            } else {
                selection.units.insert(id);
            }
        } else if touch && !selection.units.is_empty() {
            issue_move(position, selection, game, feedback, now);
        } else if !press.additive {
            selection.units.clear();
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "validated small world coordinates are rounded into sim units"
)]
fn issue_move(
    position: Vec2,
    selection: &Selection,
    game: &mut Match,
    feedback: &mut OrderFeedback,
    now: f64,
) {
    if selection.units.is_empty() {
        return;
    }
    let target =
        (position * Vec2::new(1.0, -1.0) * (rts_base::TILE as f32 / TILE_PX as f32)).round();
    let target = SimPos::new(target.x as i32, target.y as i32);
    let traversable = game.sim.terrain().speed_modifier(target, game.sim.rules()) > 0;
    feedback.0 = Some((position, traversable, now));
    if traversable {
        game.queue_command(Command::Move {
            units: selection.units.iter().copied().collect(),
            target,
        });
    }
}

fn draw_selection(
    mut gizmos: Gizmos,
    selection: Res<Selection>,
    pointer: Res<PointerState>,
    feedback: Res<OrderFeedback>,
    time: Res<Time>,
    game: Res<Match>,
    camera: Single<(&Transform, &Projection), With<RtsCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    visuals: Query<(&UnitVisual, &Transform)>,
) {
    let (transform, Projection::Orthographic(projection)) = camera.into_inner() else {
        return;
    };
    for (visual, transform) in &visuals {
        if selection.units.contains(&visual.0)
            && let Some(unit) = game.sim.unit(visual.0)
        {
            let definition = game.sim.rules().unit(unit.kind);
            let bounds = UnitBounds::from_definition(definition).world_rect(transform);
            if definition.visual_bounds.is_some() {
                gizmos.rect_2d(
                    Isometry2d::from_translation(bounds.center()),
                    bounds.size() + Vec2::splat(8.0 * projection.scale),
                    SELECTION_COLOR,
                );
            } else {
                gizmos.circle_2d(
                    bounds.center(),
                    4.0_f32.mul_add(projection.scale, bounds.width() / 2.0),
                    SELECTION_COLOR,
                );
            }
        }
    }
    let view = ScreenView {
        center: transform.translation.truncate(),
        scale: projection.scale,
        size: window.size(),
    };
    if let Some(press) = pointer
        .mouse
        .or_else(|| pointer.finger.map(|(_, press)| press))
        .filter(|press| press.dragged)
    {
        let rect = Rect::from_corners(press.world_start, view.world(press.current));
        gizmos.rect_2d(
            Isometry2d::from_translation(rect.center()),
            rect.size(),
            SELECTION_COLOR,
        );
    }
    if let Some((position, accepted, started)) = feedback.0 {
        let age = time.elapsed_secs_f64() - started;
        if age < 0.6 {
            let color = if accepted {
                Color::srgb(0.60, 0.90, 0.65)
            } else {
                Color::srgb(1.0, 0.35, 0.25)
            };
            gizmos.circle_2d(position, 12.0 * projection.scale, color);
            gizmos.cross_2d(position, 5.0 * projection.scale, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::touch::{TouchInput, TouchPhase, touch_screen_input_system};
    use rts_base::{MapData, PlayerId, Rules, Sim};

    use super::*;

    #[test]
    fn screen_coordinates_follow_camera_pan_zoom_and_y_direction() {
        let view = ScreenView {
            center: Vec2::new(128.0, -256.0),
            scale: 2.0,
            size: Vec2::new(800.0, 600.0),
        };
        assert_eq!(view.world(Vec2::new(400.0, 300.0)), view.center);
        assert_eq!(view.world(Vec2::new(500.0, 200.0)), Vec2::new(328.0, -56.0));
        assert!(!view.playable(Vec2::new(400.0, 590.0)));
        assert!(!view.playable(Vec2::new(-1.0, 100.0)));
    }

    #[test]
    fn drag_stays_a_drag_after_returning_to_start() {
        let mut press = Press::new(Vec2::ZERO, Vec2::ZERO, false);
        press.update(Vec2::new(4.0, 0.0), 8.0);
        assert!(!press.dragged);
        press.update(Vec2::new(10.0, 0.0), 8.0);
        press.update(Vec2::ZERO, 8.0);
        assert!(press.dragged);
    }

    fn test_app() -> App {
        let rules = Rules::from_ron(include_str!("../../../assets/data/core.rules.ron")).unwrap();
        let map = MapData::from_ron(
            r#"(
            name: "input", players: [(start: (0.5,0.5)), (start: (3.5,3.5))],
            tiles: ["....","....","....","...."],
            units: [(kind: "worker", owner: 0, pos: (1.5,1.5)),
                (kind: "soldier", owner: 1, pos: (2.5,1.5))],
        )"#,
        )
        .unwrap();
        let sim = Sim::new(rules, &map).unwrap();
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .insert_resource(Match::new(sim, PlayerId(0), true))
            .init_resource::<Selection>()
            .init_resource::<PointerState>()
            .init_resource::<OrderFeedback>()
            .init_resource::<Touches>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<TouchInput>()
            .add_systems(PreUpdate, touch_screen_input_system)
            .add_systems(Update, control_units);
        let mut window = Window::default();
        window.resolution.set(800.0, 600.0);
        window.focused = true;
        app.world_mut().spawn((window, PrimaryWindow));
        app.world_mut().spawn((
            RtsCamera::default(),
            Transform::from_xyz(256.0, -256.0, 0.0),
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ));
        let units: Vec<_> = app
            .world()
            .resource::<Match>()
            .sim
            .units()
            .map(|(id, unit)| {
                (
                    UnitVisual(id),
                    Transform::from_translation(crate::map::to_world(unit.pos).extend(1.0)),
                )
            })
            .collect();
        for visual in units {
            app.world_mut().spawn(visual);
        }
        app
    }

    fn touch(app: &mut App, events: &[(u64, TouchPhase, Vec2)]) {
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .unwrap();
        for &(id, phase, position) in events {
            app.world_mut().write_message(TouchInput {
                id,
                phase,
                position,
                window,
                force: None,
            });
        }
        app.update();
    }

    fn tap(app: &mut App, position: Vec2) {
        touch(app, &[(1, TouchPhase::Started, position)]);
        touch(app, &[(1, TouchPhase::Ended, position)]);
    }

    fn mouse_event(app: &mut App, position: Vec2, button: MouseButton, pressed: bool) {
        let mut window = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>()
            .single_mut(app.world_mut())
            .unwrap();
        window.set_cursor_position(Some(position));
        let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        mouse.clear();
        if pressed {
            mouse.press(button);
        } else {
            mouse.release(button);
        }
        app.update();
    }

    fn click(app: &mut App, position: Vec2) {
        mouse_event(app, position, MouseButton::Left, true);
        mouse_event(app, position, MouseButton::Left, false);
    }

    #[test]
    fn mouse_selects_boxes_local_units_and_shift_toggles() {
        let mut app = test_app();
        let own = Vec2::new(336.0, 236.0);
        click(&mut app, own);
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        click(&mut app, own);
        assert!(app.world().resource::<Selection>().units.is_empty());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        mouse_event(&mut app, Vec2::new(500.0, 300.0), MouseButton::Left, true);
        mouse_event(&mut app, Vec2::new(300.0, 200.0), MouseButton::Left, false);
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        click(&mut app, Vec2::new(464.0, 236.0));
        assert!(app.world().resource::<Selection>().units.is_empty());
    }

    #[test]
    fn selection_uses_displayed_offset_and_scaled_radius() {
        let mut app = test_app();
        let hash = app.world().resource::<Match>().sim.state_hash();
        for (visual, mut transform) in app
            .world_mut()
            .query::<(&UnitVisual, &mut Transform)>()
            .iter_mut(app.world_mut())
        {
            if visual.0 == UnitId(0) {
                transform.translation = Vec3::new(128.0, -128.0, 1.0);
                transform.scale = Vec3::new(0.5, 0.5, 1.0);
            }
        }
        // The displayed center is (272, 172); the original radius must no longer hit.
        click(&mut app, Vec2::new(302.0, 172.0));
        assert!(app.world().resource::<Selection>().units.is_empty());
        click(&mut app, Vec2::new(280.0, 172.0));
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        // Its simulation center is no longer clickable or included by a drag there.
        mouse_event(&mut app, Vec2::new(320.0, 220.0), MouseButton::Left, true);
        mouse_event(&mut app, Vec2::new(350.0, 250.0), MouseButton::Left, false);
        assert!(app.world().resource::<Selection>().units.is_empty());
        mouse_event(&mut app, Vec2::new(255.0, 155.0), MouseButton::Left, true);
        mouse_event(&mut app, Vec2::new(290.0, 190.0), MouseButton::Left, false);
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
    }

    #[test]
    fn custom_bounds_use_offset_and_rectangle_for_click_and_box_selection() {
        let mut app = test_app();
        let rules = Rules::from_ron(
            &include_str!("../../../assets/data/core.rules.ron").replace(
                "radius: 0.3,",
                "radius: 0.3, visual_bounds: Some((offset: (0.25, -0.125), size: (0.75, 0.125))),",
            ),
        )
        .unwrap();
        let map = MapData::from_ron(
            r#"(name: "bounds", players: [(start: (0.5,0.5))], tiles: ["....","...."],
            units: [(kind:"worker",owner:0,pos:(1.5,1.5))])"#,
        )
        .unwrap();
        app.world_mut().resource_mut::<Match>().sim = Sim::new(rules, &map).unwrap();
        let hash = app.world().resource::<Match>().sim.state_hash();
        // Root: (336,236), footprint center: (368,220), half-size: (48,8).
        click(&mut app, Vec2::new(411.0, 220.0));
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        click(&mut app, Vec2::new(350.0, 240.0));
        assert!(app.world().resource::<Selection>().units.is_empty());
        // A box intersecting only the footprint edge still selects this sprite.
        mouse_event(&mut app, Vec2::new(410.0, 210.0), MouseButton::Left, true);
        mouse_event(&mut app, Vec2::new(425.0, 230.0), MouseButton::Left, false);
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
    }

    #[test]
    fn right_click_and_x_queue_orders_without_changing_simulation() {
        let mut app = test_app();
        click(&mut app, Vec2::new(336.0, 236.0));
        mouse_event(&mut app, Vec2::new(400.0, 300.0), MouseButton::Right, true);
        {
            let mut game = app.world_mut().resource_mut::<Match>();
            assert_eq!(game.sim.tick(), 0);
            assert_eq!(
                game.pending,
                vec![Command::Move {
                    units: vec![UnitId(0)],
                    target: SimPos::splat(2 * rts_base::TILE)
                }]
            );
            game.pending.clear();
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyX);
        app.update();
        let game = app.world().resource::<Match>();
        assert_eq!(
            game.pending,
            vec![Command::Stop {
                units: vec![UnitId(0)]
            }]
        );
        assert_eq!(game.sim.move_target(UnitId(0)), None);
    }

    #[test]
    fn touch_tap_selects_then_issues_delayed_move_and_hud_consumes_taps() {
        let mut app = test_app();
        tap(&mut app, Vec2::new(336.0, 236.0));
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        tap(&mut app, Vec2::new(400.0, 300.0));
        {
            let game = app.world().resource::<Match>();
            assert_eq!(game.sim.move_target(UnitId(0)), None);
            assert_eq!(
                game.pending,
                vec![Command::Move {
                    units: vec![UnitId(0)],
                    target: SimPos::splat(2 * rts_base::TILE)
                }]
            );
        }
        let hash = app.world().resource::<Match>().sim.state_hash();
        tap(&mut app, Vec2::new(700.0, 570.0));
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
    }

    #[test]
    fn pinch_cancels_selection_and_remaining_finger_cannot_issue_an_order() {
        let mut app = test_app();
        tap(&mut app, Vec2::new(336.0, 236.0));
        let hash = app.world().resource::<Match>().sim.state_hash();
        touch(
            &mut app,
            &[(1, TouchPhase::Started, Vec2::new(400.0, 300.0))],
        );
        touch(
            &mut app,
            &[(2, TouchPhase::Started, Vec2::new(500.0, 300.0))],
        );
        touch(&mut app, &[(2, TouchPhase::Ended, Vec2::new(500.0, 300.0))]);
        touch(&mut app, &[(1, TouchPhase::Ended, Vec2::new(450.0, 350.0))]);
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
    }

    #[test]
    fn one_finger_box_selects_local_units_only_and_cancel_never_moves() {
        let mut app = test_app();
        touch(
            &mut app,
            &[(1, TouchPhase::Started, Vec2::new(500.0, 300.0))],
        );
        touch(&mut app, &[(1, TouchPhase::Moved, Vec2::new(300.0, 200.0))]);
        touch(&mut app, &[(1, TouchPhase::Ended, Vec2::new(300.0, 200.0))]);
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        let hash = app.world().resource::<Match>().sim.state_hash();
        touch(
            &mut app,
            &[(1, TouchPhase::Started, Vec2::new(400.0, 300.0))],
        );
        touch(
            &mut app,
            &[(1, TouchPhase::Canceled, Vec2::new(400.0, 300.0))],
        );
        assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
    }
}
