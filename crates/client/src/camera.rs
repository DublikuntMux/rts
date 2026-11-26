use bevy::{
    input::{
        mouse::{AccumulatedMouseScroll, MouseScrollPixelsPerLine},
        touch::Touches,
    },
    prelude::*,
    window::{CursorMoved, PrimaryWindow},
};
use rts_base::ResourceKind;

use crate::{
    game::{AppState, Match},
    hud::{HudLayout, PhoneHud},
    input::{Selection, world_pointer_allowed},
    map::{MapBounds, to_world, to_world_distance},
    menu::{Menu, MenuActions},
    pause_menu::{PauseMenu, world_input_blocked},
    skirmish,
};

/// Top-down RTS camera.
///
/// - Desktop: pan with WASD/arrows, screen edges, or middle-mouse drag; zoom with the wheel.
/// - Touch: drag with two fingers to pan; pinch to zoom.
pub struct RtsCameraPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraControl;

impl Plugin for RtsCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(AppState::InGame), focus_on_start)
            .add_systems(
                Update,
                control_camera
                    .in_set(CameraControl)
                    .after(MenuActions)
                    .run_if(resource_exists::<MapBounds>),
            );
    }
}

#[derive(Component)]
#[require(Camera2d)]
pub struct RtsCamera {
    /// Smallest projection scale (closest zoom).
    pub min_scale: f32,
    /// Largest projection scale (farthest zoom); also limited so the view never exceeds the map.
    pub max_scale: f32,
    /// Keyboard and edge-scroll speed, in screen pixels per second.
    pub pan_speed: f32,
    /// Width of the window border that scrolls the camera, in logical pixels. Zero disables it.
    pub edge_margin: f32,
    /// Scale multiplier per mouse-wheel line.
    pub wheel_zoom_step: f32,
}

impl Default for RtsCamera {
    fn default() -> Self {
        Self {
            min_scale: 0.25,
            max_scale: 4.0,
            pan_speed: 1200.0,
            edge_margin: 8.0,
            wheel_zoom_step: 1.15,
        }
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(RtsCamera::default());
}

/// Centers the camera on the local player's start position.
fn focus_on_start(
    game: Res<Match>,
    camera: Single<(&RtsCamera, &mut Transform, &mut Projection)>,
    window: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
) {
    let start = to_world(game.view.start());
    let (settings, mut transform, mut projection) = camera.into_inner();
    transform.translation = start.extend(transform.translation.z);
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };
    ortho.scale = 1.0;
    if game.view.is_skirmish()
        && let Ok(window) = window.single()
        && let Some(view) = skirmish_start_view(
            &game,
            start,
            window.size(),
            ui_scale.as_ref().map_or(1.0, |scale| scale.0),
            settings,
        )
    {
        transform.translation = view.center.extend(transform.translation.z);
        ortho.scale = view.scale;
    }
}

fn skirmish_start_view(
    game: &Match,
    start: Vec2,
    window_size: Vec2,
    ui_scale: f32,
    settings: &RtsCamera,
) -> Option<View> {
    let hud = HudLayout::new(window_size / ui_scale, false, None);
    let top = if hud.phone && !hud.landscape {
        let resource_bar = skirmish::resource_bar_position(&hud);
        (resource_bar.y + 42.0).max(hud.minimap.max.y + 12.0)
    } else {
        64.0
    };
    let bottom_inset = hud.bottom_inset * ui_scale;
    let min = Vec2::new(16.0, top) * ui_scale;
    let max = window_size - Vec2::new(16.0 * ui_scale, 16.0_f32.mul_add(ui_scale, bottom_inset));
    if max.cmple(min).any() {
        return None;
    }
    let viewport = Rect::from_corners(min, max);
    let tile = to_world_distance(rts_base::TILE);
    let nearby_units = game
        .view
        .units()
        .filter(|(_, unit)| unit.owner == game.local_player)
        .map(|(_, unit)| to_world(unit.pos))
        .filter(|&pos| pos.distance_squared(start) <= (tile * 4.0).powi(2));
    let deposits = [ResourceKind::Wood, ResourceKind::Gold]
        .into_iter()
        .filter_map(|kind| {
            game.view
                .deposits()
                .filter(|(_, deposit)| deposit.kind == kind)
                .map(|(_, deposit)| to_world(deposit.pos))
                .min_by(|a, b| {
                    a.distance_squared(start)
                        .total_cmp(&b.distance_squared(start))
                })
        });
    let (min, max) = nearby_units
        .chain(deposits)
        .fold((start, start), |(min, max), pos| {
            (min.min(pos), max.max(pos))
        });
    let padding = Vec2::splat(tile * 0.75);
    let region = Rect::from_corners(min - padding, max + padding);
    let bounds = Rect::from_corners(Vec2::ZERO, to_world(game.view.world_size()));
    let playable_size = Vec2::new(window_size.x, (window_size.y - bottom_inset).max(1.0));
    let max_scale = max_projection_scale(settings, bounds, playable_size);
    let scale = (region.size() / viewport.size())
        .max_element()
        .clamp(settings.min_scale, max_scale);
    let offset = (viewport.center() - window_size / 2.0) * Vec2::new(1.0, -1.0) * scale;
    let mut view = View {
        center: region.center() - offset,
        scale,
        window_size,
        bottom_inset,
        overscroll: hud.camera_overscroll() * ui_scale,
        min_scale: settings.min_scale,
        max_scale,
    };
    view.clamp_to(bounds);
    Some(view)
}

fn max_projection_scale(settings: &RtsCamera, bounds: Rect, playable_size: Vec2) -> f32 {
    settings
        .max_scale
        .min((bounds.size() / playable_size).min_element())
        .max(settings.min_scale)
}

#[derive(Default)]
struct CameraGestures {
    previous_touches: Vec<(u64, Vec2)>,
    touch_blocked: bool,
    middle_drag: bool,
    was_blocked: bool,
}

impl CameraGestures {
    // Track positions per frame: Touch::previous_position only refreshes on touch events.
    fn touch_gesture(
        &mut self,
        touches: &Touches,
        pointer_allowed: impl Fn(Vec2) -> bool,
    ) -> Option<TouchGesture> {
        let mut current: Vec<_> = touches
            .iter()
            .map(|touch| (touch.id(), touch.position()))
            .collect();
        current.sort_unstable_by_key(|&(id, _)| id);
        if touches
            .iter()
            .any(|touch| !pointer_allowed(touch.start_position()))
        {
            self.touch_blocked = true;
        }
        let gesture = if self.touch_blocked {
            None
        } else {
            TouchGesture::between(&self.previous_touches, &current)
        };
        if current.is_empty() {
            self.touch_blocked = false;
        }
        self.previous_touches = current;
        gesture
    }
}

fn control_camera(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut cursor_moves: MessageReader<CursorMoved>,
    scroll: Res<AccumulatedMouseScroll>,
    pixels_per_line: Res<MouseScrollPixelsPerLine>,
    touches: Res<Touches>,
    mut gestures: Local<CameraGestures>,
    window: Single<&Window, With<PrimaryWindow>>,
    bounds: Res<MapBounds>,
    camera: Single<(&RtsCamera, &mut Transform, &mut Projection)>,
    pause: Option<Res<PauseMenu>>,
    menu: Option<Res<Menu>>,
    ui_scale: Option<Res<UiScale>>,
    match_view: (Option<Res<Selection>>, Option<Res<Match>>),
    phone_hud: Option<Res<PhoneHud>>,
) {
    let ui_scale = ui_scale.as_ref().map_or(1.0, |scale| scale.0);
    let window_size = window.size();
    let logical_size = window_size / ui_scale;
    let (selection, game) = match_view;
    let panel = phone_hud.as_deref().and_then(|hud| hud.panel);
    let hud = HudLayout::new(
        logical_size,
        selection
            .as_deref()
            .is_some_and(|selection| !selection.units.is_empty()),
        panel,
    );
    let blocked =
        world_input_blocked(pause.as_deref(), menu.as_deref()) || (hud.phone && panel.is_some());
    let was_blocked = gestures.was_blocked;
    gestures.was_blocked = blocked;
    if blocked || was_blocked || !window.focused || keys.just_pressed(KeyCode::Escape) {
        gestures.previous_touches.clear();
        gestures.touch_blocked = touches.iter().next().is_some();
        gestures.middle_drag = false;
        cursor_moves.clear();
        return;
    }
    let (settings, mut transform, mut projection) = camera.into_inner();
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };
    if window_size.min_element() <= 0.0 {
        return;
    }
    let economy = game
        .as_deref()
        .zip(selection.as_deref())
        .and_then(|(game, selection)| {
            skirmish::actions_bounds(game, selection, logical_size, &hud)
        });
    let pointer_allowed = |position| {
        world_pointer_allowed(position, window_size, ui_scale, &hud)
            && !economy.is_some_and(|rect| rect.contains(position / ui_scale))
    };
    let mut view = camera_view(
        transform.translation.truncate(),
        ortho.scale,
        window_size,
        ui_scale,
        &hud,
        economy,
        bounds.0,
        settings,
    );
    let playable_size = view.playable_size();

    // Keyboard and screen-edge scrolling.
    let mut direction = camera_key_direction(&keys);
    if window.focused
        && settings.edge_margin > 0.0
        && touches.iter().next().is_none()
        && !touches.any_just_pressed()
        && !touches.any_just_released()
        && !touches.any_just_canceled()
        && let Some(cursor) = window
            .cursor_position()
            .filter(|&position| pointer_allowed(position))
    {
        let far = playable_size - settings.edge_margin;
        direction.x += f32::from(cursor.x >= far.x) - f32::from(cursor.x <= settings.edge_margin);
        direction.y += f32::from(cursor.y <= settings.edge_margin) - f32::from(cursor.y >= far.y);
    }
    view.pan(direction.normalize_or_zero() * settings.pan_speed * time.delta_secs());

    // Middle-mouse drag.
    let cursor_delta: Vec2 = cursor_moves.read().filter_map(|moved| moved.delta).sum();
    if mouse_buttons.just_pressed(MouseButton::Middle) {
        gestures.middle_drag = window.cursor_position().is_some_and(pointer_allowed);
    }
    if !mouse_buttons.pressed(MouseButton::Middle) {
        gestures.middle_drag = false;
    }
    if gestures.middle_drag {
        view.drag(cursor_delta);
    }

    // Mouse-wheel zoom toward the cursor.
    let wheel_lines = scroll.to_lines(&pixels_per_line).delta.y;
    if wheel_lines != 0.0
        && let Some(anchor) = window
            .cursor_position()
            .filter(|&position| pointer_allowed(position))
    {
        view.zoom_at(anchor, settings.wheel_zoom_step.powf(-wheel_lines));
    }

    if let Some(gesture) = gestures.touch_gesture(&touches, pointer_allowed) {
        view.zoom_at(gesture.from, gesture.zoom);
        view.drag(gesture.to - gesture.from);
    }

    view.clamp_to(bounds.0);
    transform.translation = view.center.extend(transform.translation.z);
    ortho.scale = view.scale;
}

fn camera_key_direction(keys: &ButtonInput<KeyCode>) -> Vec2 {
    let axis = |positive, negative| {
        f32::from(keys.any_pressed(positive)) - f32::from(keys.any_pressed(negative))
    };
    Vec2::new(
        axis(
            [KeyCode::KeyD, KeyCode::ArrowRight],
            [KeyCode::KeyA, KeyCode::ArrowLeft],
        ),
        axis(
            [KeyCode::KeyW, KeyCode::ArrowUp],
            [KeyCode::KeyS, KeyCode::ArrowDown],
        ),
    )
}

fn camera_view(
    center: Vec2,
    scale: f32,
    window_size: Vec2,
    ui_scale: f32,
    hud: &HudLayout,
    economy: Option<Rect>,
    bounds: Rect,
    settings: &RtsCamera,
) -> View {
    let mut overscroll = hud.camera_overscroll();
    if hud.phone
        && let Some(rect) = economy
    {
        overscroll = overscroll.max(window_size.y / ui_scale - rect.min.y + 32.0);
    }
    let mut view = View {
        center,
        scale,
        window_size,
        bottom_inset: hud.bottom_inset * ui_scale,
        overscroll: overscroll * ui_scale,
        min_scale: settings.min_scale,
        max_scale: settings.max_scale,
    };
    view.max_scale = max_projection_scale(settings, bounds, view.playable_size());
    view
}

/// The camera's visible area, manipulated with screen-space input.
///
/// Screen positions are logical pixels from the window's top-left corner, matching cursor and
/// touch positions. Assumes the default `ScalingMode::WindowSize`, where one world unit spans
/// one logical pixel at scale 1.
struct View {
    center: Vec2,
    scale: f32,
    window_size: Vec2,
    /// Screen area covered by the bottom HUD, in window pixels.
    bottom_inset: f32,
    /// Room beyond each map edge, in window pixels, to uncover units behind phone controls.
    overscroll: f32,
    min_scale: f32,
    max_scale: f32,
}

impl View {
    fn playable_size(&self) -> Vec2 {
        Vec2::new(
            self.window_size.x,
            (self.window_size.y - self.bottom_inset).max(1.0),
        )
    }

    /// Moves the camera in screen directions (y up) by a distance in screen pixels.
    fn pan(&mut self, screen_distance: Vec2) {
        self.center += screen_distance * self.scale;
    }

    /// Moves the world along with a screen-space drag (y down).
    fn drag(&mut self, screen_delta: Vec2) {
        self.center -= Vec2::new(screen_delta.x, -screen_delta.y) * self.scale;
    }

    /// Multiplies the scale while keeping the world point under `anchor` fixed on screen.
    fn zoom_at(&mut self, anchor: Vec2, factor: f32) {
        let scale = (self.scale * factor).clamp(self.min_scale, self.max_scale);
        let from_center = anchor - self.window_size / 2.0;
        self.center += Vec2::new(from_center.x, -from_center.y) * (self.scale - scale);
        self.scale = scale;
    }

    /// Bounds the visible area, allowing phone overscroll and centering oversized views.
    fn clamp_to(&mut self, bounds: Rect) {
        self.scale = self.scale.clamp(self.min_scale, self.max_scale);
        // Clamp the visible world above the HUD, retaining the full-window projection used
        // by pointer conversion. Its center is higher than the camera's render center.
        let visible_size = self.playable_size();
        let offset = Vec2::Y * (self.bottom_inset * self.scale / 2.0);
        let half_view = visible_size / 2.0 * self.scale;
        // Keep a small map centered; only scroll past edges on axes larger than the view.
        let extra = Vec2::select(
            half_view.cmplt(bounds.half_size()),
            Vec2::splat(self.overscroll * self.scale),
            Vec2::ZERO,
        );
        let bounds = Rect::from_corners(bounds.min - extra, bounds.max + extra);
        let half_view = half_view.min(bounds.half_size());
        self.center =
            (self.center + offset).clamp(bounds.min + half_view, bounds.max - half_view) - offset;
    }
}

/// Pan and pinch made by fingers that stayed down since the previous frame.
#[derive(Debug, PartialEq)]
struct TouchGesture {
    /// Finger centroid on the previous frame.
    from: Vec2,
    /// Finger centroid on this frame.
    to: Vec2,
    /// Scale multiplier; below 1 when fingers spread apart.
    zoom: f32,
}

impl TouchGesture {
    /// Both slices are `(touch id, position)` sorted by id. Returns `None` when no finger is down
    /// or a finger was added or lifted, so the centroid never jumps.
    fn between(previous: &[(u64, Vec2)], current: &[(u64, Vec2)]) -> Option<Self> {
        let same_fingers = previous.len() == current.len()
            && previous.iter().zip(current).all(|(a, b)| a.0 == b.0);
        if current.len() < 2 || !same_fingers {
            return None;
        }
        let from = centroid(previous);
        let to = centroid(current);
        let spread_before = spread(previous, from);
        let spread_now = spread(current, to);
        let zoom = if spread_before > 0.0 && spread_now > 0.0 {
            spread_before / spread_now
        } else {
            1.0
        };
        Some(Self { from, to, zoom })
    }
}

#[expect(clippy::cast_precision_loss, reason = "a handful of fingers")]
fn centroid(touches: &[(u64, Vec2)]) -> Vec2 {
    touches.iter().map(|&(_, position)| position).sum::<Vec2>() / touches.len() as f32
}

#[expect(clippy::cast_precision_loss, reason = "a handful of fingers")]
fn spread(touches: &[(u64, Vec2)], center: Vec2) -> f32 {
    touches
        .iter()
        .map(|&(_, position)| position.distance(center))
        .sum::<f32>()
        / touches.len() as f32
}

#[cfg(test)]
mod tests {
    #[test]
    fn frontier_starts_with_headquarters_workers_and_both_deposits_visible() {
        let setup = rts_net::MatchSetup {
            rules: include_str!("../../../assets/data/core.rules.ron").into(),
            map: include_str!("../../../assets/maps/frontier.map.ron").into(),
            seed: 0,
            players: 2,
        };
        let game = Match::new(setup.simulation().unwrap(), rts_base::PlayerId(0), true);
        let start = to_world(game.view.start());
        let positions: Vec<_> = game
            .view
            .units()
            .filter(|(_, unit)| unit.owner == game.local_player)
            .map(|(_, unit)| unit.pos)
            .chain(game.view.deposits().take(2).map(|(_, deposit)| deposit.pos))
            .collect();
        for size in [
            Vec2::new(1280.0, 720.0),
            Vec2::new(390.0, 844.0),
            Vec2::new(844.0, 390.0),
            Vec2::new(480.0, 320.0),
        ] {
            for ui_scale in [1.0, 1.5] {
                let view = skirmish_start_view(&game, start, size, ui_scale, &RtsCamera::default())
                    .unwrap();
                let hud = HudLayout::new(size / ui_scale, false, None);
                for &position in &positions {
                    let offset = (to_world(position) - view.center) / view.scale;
                    let screen = size / 2.0 + offset * Vec2::new(1.0, -1.0);
                    assert!(
                        world_pointer_allowed(screen, size, ui_scale, &hud),
                        "{position:?} projects to {screen:?} at {size:?} × {ui_scale}",
                    );
                }
            }
        }
    }
    use super::*;

    fn view() -> View {
        View {
            center: Vec2::ZERO,
            scale: 1.0,
            window_size: Vec2::new(800.0, 600.0),
            bottom_inset: 0.0,
            overscroll: 0.0,
            min_scale: 0.25,
            max_scale: 4.0,
        }
    }

    fn world_at(view: &View, screen: Vec2) -> Vec2 {
        let from_center = screen - view.window_size / 2.0;
        view.center + Vec2::new(from_center.x, -from_center.y) * view.scale
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut view = view();
        let anchor = Vec2::new(700.0, 100.0);
        let before = world_at(&view, anchor);
        view.zoom_at(anchor, 0.5);
        assert_eq!(view.scale, 0.5);
        assert!(world_at(&view, anchor).abs_diff_eq(before, 1e-4));
    }

    #[test]
    fn zoom_respects_limits() {
        let mut view = view();
        view.zoom_at(Vec2::ZERO, 100.0);
        assert_eq!(view.scale, 4.0);
        view.zoom_at(Vec2::ZERO, 0.0001);
        assert_eq!(view.scale, 0.25);
    }

    #[test]
    fn drag_moves_world_with_pointer() {
        let mut view = view();
        let grabbed = world_at(&view, Vec2::new(400.0, 300.0));
        view.drag(Vec2::new(50.0, -20.0));
        assert!(world_at(&view, Vec2::new(450.0, 280.0)).abs_diff_eq(grabbed, 1e-4));
    }

    #[test]
    fn clamp_keeps_view_inside_map() {
        let bounds = Rect::from_center_size(Vec2::ZERO, Vec2::new(1000.0, 500.0));
        let mut view = view();
        view.center = Vec2::new(5000.0, -5000.0);
        view.clamp_to(bounds);
        // Wider map axis: right edge of the view meets the map edge.
        assert_eq!(view.center.x, 100.0);
        // Map shorter than the view: centered.
        assert_eq!(view.center.y, 0.0);
    }

    #[test]
    fn pinch_and_pan() {
        let previous = [(1, Vec2::new(100.0, 100.0)), (2, Vec2::new(200.0, 100.0))];
        let current = [(1, Vec2::new(50.0, 110.0)), (2, Vec2::new(250.0, 110.0))];
        let gesture = TouchGesture::between(&previous, &current).unwrap();
        assert_eq!(gesture.from, Vec2::new(150.0, 100.0));
        assert_eq!(gesture.to, Vec2::new(150.0, 110.0));
        assert_eq!(gesture.zoom, 0.5);
    }

    #[test]
    fn finger_change_is_ignored() {
        let one = [(1, Vec2::ZERO)];
        let two = [(1, Vec2::ZERO), (2, Vec2::ONE)];
        assert_eq!(TouchGesture::between(&one, &two), None);
        assert_eq!(TouchGesture::between(&two, &one), None);
        assert_eq!(TouchGesture::between(&[], &[]), None);
        assert_eq!(TouchGesture::between(&one, &one), None);
    }
    #[test]
    fn clamp_keeps_bottom_map_edge_above_the_hud_without_changing_projection_coordinates() {
        let bounds = Rect::from_corners(Vec2::ZERO, Vec2::splat(2000.0));
        let mut view = view();
        view.bottom_inset = 184.0;
        view.center = Vec2::new(1000.0, -1000.0);
        view.clamp_to(bounds);
        assert_eq!(world_at(&view, Vec2::new(400.0, 416.0)).y, bounds.min.y);
        assert_eq!(world_at(&view, Vec2::new(400.0, 0.0)).y, 416.0);
        assert_eq!(view.center.y, 116.0);
        view.center.y = 3000.0;
        view.clamp_to(bounds);
        assert_eq!(world_at(&view, Vec2::new(400.0, 0.0)).y, bounds.max.y);
        // A short map is centered in the usable area, with its bottom edge still accessible.
        view.clamp_to(Rect::from_center_size(Vec2::ZERO, Vec2::splat(100.0)));
        assert_eq!(world_at(&view, Vec2::new(400.0, 208.0)).y, 0.0);
    }
    fn control_app() -> App {
        use bevy::input::touch::{TouchInput, touch_screen_input_system};
        let mut app = App::new();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs_f32(0.1));
        app.insert_resource(time)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<MouseScrollPixelsPerLine>()
            .init_resource::<Touches>()
            .init_resource::<PauseMenu>()
            .init_resource::<Menu>()
            .insert_resource(MapBounds(Rect::from_center_size(
                Vec2::ZERO,
                Vec2::splat(10000.0),
            )))
            .add_message::<CursorMoved>()
            .add_message::<TouchInput>()
            .add_systems(PreUpdate, touch_screen_input_system)
            .add_systems(Update, control_camera);
        let mut window = Window::default();
        window.resolution.set(800.0, 600.0);
        window.focused = true;
        window.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        app.world_mut().spawn((window, PrimaryWindow));
        app.world_mut().spawn((
            RtsCamera::default(),
            Transform::default(),
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ));
        app
    }

    fn camera_state(app: &mut App) -> (Vec3, f32) {
        let (transform, projection) = app
            .world_mut()
            .query_filtered::<(&Transform, &Projection), With<RtsCamera>>()
            .single(app.world())
            .unwrap();
        let Projection::Orthographic(projection) = projection else {
            panic!("orthographic camera")
        };
        (transform.translation, projection.scale)
    }

    #[test]
    fn modal_blocks_keyboard_wheel_and_drag_and_held_drag_cannot_resume_through_it() {
        let mut app = control_app();
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .unwrap();
        let moved = CursorMoved {
            window,
            position: Vec2::new(410.0, 310.0),
            delta: Some(Vec2::splat(10.0)),
        };
        let before = camera_state(&mut app);
        app.world_mut().resource_mut::<PauseMenu>().open = true;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut()
            .resource_mut::<AccumulatedMouseScroll>()
            .delta
            .y = 1.0;
        app.world_mut().write_message(moved.clone());
        app.update();
        assert_eq!(camera_state(&mut app), before);
        app.world_mut().resource_mut::<PauseMenu>().open = false;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .resource_mut::<AccumulatedMouseScroll>()
            .delta = Vec2::ZERO;
        app.update();
        app.world_mut().write_message(moved.clone());
        app.update();
        assert_eq!(camera_state(&mut app), before);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        app.world_mut().write_message(moved);
        app.update();
        assert_eq!(camera_state(&mut app).0, Vec3::new(-10.0, 10.0, 0.0));
        let before_dialog = camera_state(&mut app);
        app.world_mut().resource_mut::<Menu>().dialog = Some(crate::menu::Dialog::Confirm(
            crate::menu::Confirm::LeaveMatch,
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<AccumulatedMouseScroll>()
            .delta
            .y = 1.0;
        app.update();
        assert_eq!(camera_state(&mut app), before_dialog);
    }

    #[test]
    fn fingers_held_across_pause_cannot_pan_or_pinch_until_they_lift() {
        use bevy::input::touch::{TouchInput, TouchPhase};
        let mut app = control_app();
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .unwrap();
        let fingers = |app: &mut App, phase, positions: [Vec2; 2]| {
            for (id, position) in [1, 2].into_iter().zip(positions) {
                app.world_mut().write_message(TouchInput {
                    id,
                    phase,
                    position,
                    window,
                    force: None,
                });
            }
            app.update();
        };
        let before = camera_state(&mut app);
        app.world_mut().resource_mut::<PauseMenu>().open = true;
        fingers(
            &mut app,
            TouchPhase::Started,
            [Vec2::new(300.0, 200.0), Vec2::new(500.0, 200.0)],
        );
        app.world_mut().resource_mut::<PauseMenu>().open = false;
        app.update();
        fingers(
            &mut app,
            TouchPhase::Moved,
            [Vec2::new(250.0, 220.0), Vec2::new(550.0, 220.0)],
        );
        fingers(
            &mut app,
            TouchPhase::Moved,
            [Vec2::new(200.0, 240.0), Vec2::new(600.0, 240.0)],
        );
        assert_eq!(camera_state(&mut app), before);
        fingers(
            &mut app,
            TouchPhase::Ended,
            [Vec2::new(200.0, 240.0), Vec2::new(600.0, 240.0)],
        );
        fingers(
            &mut app,
            TouchPhase::Started,
            [Vec2::new(300.0, 200.0), Vec2::new(500.0, 200.0)],
        );
        fingers(
            &mut app,
            TouchPhase::Moved,
            [Vec2::new(250.0, 220.0), Vec2::new(550.0, 220.0)],
        );
        assert!(camera_state(&mut app).1 < before.1);
    }
    #[test]
    fn bottom_edge_pan_uses_the_world_boundary_above_the_hud() {
        for ui_scale in [1.0, 1.5] {
            let mut app = control_app();
            app.insert_resource(UiScale(ui_scale));
            let size = Vec2::new(1200.0, 800.0) * ui_scale;
            let bottom = 184.0_f32.mul_add(-ui_scale, size.y);
            {
                let mut window = app
                    .world_mut()
                    .query_filtered::<&mut Window, With<PrimaryWindow>>()
                    .single_mut(app.world_mut())
                    .unwrap();
                window.resolution.set(size.x, size.y);
            }
            app.world_mut()
                .query_filtered::<&mut Window, With<PrimaryWindow>>()
                .single_mut(app.world_mut())
                .unwrap()
                .set_cursor_position(Some(Vec2::new(400.0, bottom - 2.0)));
            app.update();
            let after_pan = camera_state(&mut app).0;
            assert!(after_pan.y < 0.0, "pointer above the HUD must pan down");
            app.world_mut()
                .query_filtered::<&mut Window, With<PrimaryWindow>>()
                .single_mut(app.world_mut())
                .unwrap()
                .set_cursor_position(Some(Vec2::new(400.0, bottom + 2.0)));
            app.update();
            assert_eq!(camera_state(&mut app).0, after_pan);
        }
    }

    fn phone_control_app(size: Vec2, ui_scale: f32) -> App {
        let mut app = control_app();
        app.insert_resource(UiScale(ui_scale))
            .init_resource::<Selection>()
            .init_resource::<PhoneHud>();
        {
            let mut window = app
                .world_mut()
                .query_filtered::<&mut Window, With<PrimaryWindow>>()
                .single_mut(app.world_mut())
                .unwrap();
            window.resolution.set(size.x * ui_scale, size.y * ui_scale);
            window.set_cursor_position(Some(Vec2::new(20.0, size.y - 20.0) * ui_scale));
        }
        app
    }

    #[test]
    fn phone_camera_clamps_full_window_with_room_to_reach_units_beneath_corner_controls() {
        for logical_size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for ui_scale in [1.0, 1.5] {
                for zoom in [0.5, 2.0] {
                    let mut app = phone_control_app(logical_size, ui_scale);
                    let size = logical_size * ui_scale;
                    let bounds = app.world().resource::<MapBounds>().0;
                    let margin = HudLayout::new(logical_size, false, None).camera_overscroll();
                    let overscroll = Vec2::splat(margin * ui_scale * zoom);
                    for direction in [Vec2::NEG_ONE, Vec2::ONE] {
                        {
                            let (mut transform, mut projection) = app.world_mut()
                                .query_filtered::<(&mut Transform, &mut Projection), With<RtsCamera>>()
                                .single_mut(app.world_mut())
                                .unwrap();
                            transform.translation = (direction * 100_000.0).extend(0.0);
                            let Projection::Orthographic(ortho) = projection.as_mut() else {
                                panic!("orthographic camera");
                            };
                            ortho.scale = zoom;
                        }
                        app.update();
                        let (center, actual_zoom) = camera_state(&mut app);
                        assert_eq!(actual_zoom, zoom);
                        let half_view = size / 2.0 * zoom;
                        if direction == Vec2::NEG_ONE {
                            assert!(
                                (center.truncate() - half_view)
                                    .abs_diff_eq(bounds.min - overscroll, 0.001)
                            );
                        } else {
                            assert!(
                                (center.truncate() + half_view)
                                    .abs_diff_eq(bounds.max + overscroll, 0.001)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn phone_camera_zoom_limit_uses_full_height() {
        for logical_size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for ui_scale in [1.0, 1.5] {
                let mut app = phone_control_app(logical_size, ui_scale);
                let bounds = Rect::from_center_size(Vec2::ZERO, Vec2::splat(1000.0));
                app.insert_resource(MapBounds(bounds));
                {
                    let mut projection = app
                        .world_mut()
                        .query_filtered::<&mut Projection, With<RtsCamera>>()
                        .single_mut(app.world_mut())
                        .unwrap();
                    let Projection::Orthographic(ortho) = projection.as_mut() else {
                        panic!("orthographic camera");
                    };
                    ortho.scale = 4.0;
                }
                app.update();
                let expected = (bounds.size() / (logical_size * ui_scale)).min_element();
                assert!((camera_state(&mut app).1 - expected).abs() < 0.0001);
            }
        }
    }

    #[test]
    fn minimum_phone_viewports_center_maps_smaller_than_the_minimum_zoom_without_invalid_clamps() {
        for size in [Vec2::new(320.0, 568.0), Vec2::new(568.0, 320.0)] {
            for ui_scale in [1.0, 1.5] {
                let mut app = phone_control_app(size / ui_scale, ui_scale);
                let bounds = Rect::from_center_size(Vec2::ZERO, Vec2::splat(8.0));
                app.insert_resource(MapBounds(bounds));
                for direction in [Vec2::NEG_ONE, Vec2::ONE] {
                    app.world_mut()
                        .query_filtered::<&mut Transform, With<RtsCamera>>()
                        .single_mut(app.world_mut())
                        .unwrap()
                        .translation = (direction * 100_000.0).extend(0.0);
                    app.update();
                    let (center, zoom) = camera_state(&mut app);
                    assert_eq!(zoom, RtsCamera::default().min_scale);
                    assert_eq!(center.truncate(), bounds.center());
                }
            }
        }
    }

    #[test]
    fn phone_edge_pan_and_wheel_ignore_only_visible_controls() {
        for logical_size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for ui_scale in [1.0, 1.5] {
                let mut app = phone_control_app(logical_size, ui_scale);
                app.world_mut()
                    .resource_mut::<Selection>()
                    .units
                    .insert(rts_base::UnitId(0));
                let hud = HudLayout::new(logical_size, true, None);
                for point in [hud.minimap.center(), hud.commands.unwrap().center()] {
                    app.world_mut()
                        .query_filtered::<&mut Window, With<PrimaryWindow>>()
                        .single_mut(app.world_mut())
                        .unwrap()
                        .set_cursor_position(Some(point * ui_scale));
                    app.world_mut()
                        .resource_mut::<AccumulatedMouseScroll>()
                        .delta
                        .y = 1.0;
                    let before = camera_state(&mut app);
                    app.update();
                    assert_eq!(camera_state(&mut app), before);
                }
                app.world_mut()
                    .resource_mut::<AccumulatedMouseScroll>()
                    .delta = Vec2::ZERO;
                app.world_mut()
                    .query_filtered::<&mut Window, With<PrimaryWindow>>()
                    .single_mut(app.world_mut())
                    .unwrap()
                    .set_cursor_position(Some(Vec2::new(
                        20.0 * ui_scale,
                        logical_size.y.mul_add(ui_scale, -2.0),
                    )));
                app.update();
                assert!(camera_state(&mut app).0.y < 0.0);
            }
        }
    }

    #[test]
    fn phone_panels_capture_keyboard_wheel_middle_drag_and_held_fingers_through_close() {
        use bevy::input::touch::{TouchInput, TouchPhase};
        for logical_size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for ui_scale in [1.0, 1.5] {
                for panel in [
                    crate::hud::PhonePanel::Map,
                    crate::hud::PhonePanel::Selection,
                ] {
                    let mut app = phone_control_app(logical_size, ui_scale);
                    let window = app
                        .world_mut()
                        .query_filtered::<Entity, With<PrimaryWindow>>()
                        .single(app.world())
                        .unwrap();
                    let before = camera_state(&mut app);
                    app.world_mut().resource_mut::<PhoneHud>().panel = Some(panel);
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(KeyCode::KeyW);
                    app.world_mut()
                        .resource_mut::<ButtonInput<MouseButton>>()
                        .press(MouseButton::Middle);
                    app.world_mut()
                        .resource_mut::<AccumulatedMouseScroll>()
                        .delta
                        .y = 1.0;
                    for id in [1, 2] {
                        app.world_mut().write_message(TouchInput {
                            id,
                            phase: TouchPhase::Started,
                            position: Vec2::new(if id == 1 { 100.0 } else { 200.0 }, 200.0)
                                * ui_scale,
                            window,
                            force: None,
                        });
                    }
                    app.update();
                    assert_eq!(camera_state(&mut app), before);
                    app.world_mut().resource_mut::<PhoneHud>().panel = None;
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .reset_all();
                    app.world_mut()
                        .resource_mut::<ButtonInput<MouseButton>>()
                        .clear();
                    app.world_mut()
                        .resource_mut::<AccumulatedMouseScroll>()
                        .delta = Vec2::ZERO;
                    app.update();
                    for id in [1, 2] {
                        app.world_mut().write_message(TouchInput {
                            id,
                            phase: TouchPhase::Moved,
                            position: Vec2::new(if id == 1 { 80.0 } else { 220.0 }, 220.0)
                                * ui_scale,
                            window,
                            force: None,
                        });
                    }
                    app.world_mut().write_message(CursorMoved {
                        window,
                        position: Vec2::new(200.0, 300.0),
                        delta: Some(Vec2::splat(20.0)),
                    });
                    app.update();
                    app.update();
                    assert_eq!(camera_state(&mut app), before);
                    for id in [1, 2] {
                        app.world_mut().write_message(TouchInput {
                            id,
                            phase: TouchPhase::Ended,
                            position: Vec2::new(if id == 1 { 80.0 } else { 220.0 }, 220.0)
                                * ui_scale,
                            window,
                            force: None,
                        });
                    }
                    app.update();
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(KeyCode::KeyW);
                    app.update();
                    assert!(camera_state(&mut app).0.y > before.0.y);
                }
            }
        }
    }
}
