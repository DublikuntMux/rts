use bevy::{
    input::{
        mouse::{AccumulatedMouseScroll, MouseScrollPixelsPerLine},
        touch::Touches,
    },
    prelude::*,
    window::{CursorMoved, PrimaryWindow},
};

use crate::{
    game::{AppState, Match},
    map::{MapBounds, to_world},
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
    /// Width of the window border that scrolls the camera, in logical pixels.
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
fn focus_on_start(game: Res<Match>, mut camera: Single<&mut Transform, With<RtsCamera>>) {
    if let Some(player) = game.sim.players().get(usize::from(game.local_player.0)) {
        let start = to_world(player.start);
        camera.translation = start.extend(camera.translation.z);
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
    mut previous_touches: Local<Vec<(u64, Vec2)>>,
    window: Single<&Window, With<PrimaryWindow>>,
    bounds: Res<MapBounds>,
    camera: Single<(&RtsCamera, &mut Transform, &mut Projection)>,
) {
    let (settings, mut transform, mut projection) = camera.into_inner();
    let Projection::Orthographic(ortho) = projection.as_mut() else {
        return;
    };
    let window_size = window.size();
    if window_size.min_element() <= 0.0 {
        return;
    }
    let mut view = View {
        center: transform.translation.truncate(),
        scale: ortho.scale,
        window_size,
        min_scale: settings.min_scale,
        max_scale: settings
            .max_scale
            .min((bounds.0.size() / window_size).min_element())
            .max(settings.min_scale),
    };

    // Keyboard and screen-edge scrolling.
    let mut direction = Vec2::ZERO;
    for (keys_for_direction, step) in [
        ([KeyCode::KeyA, KeyCode::ArrowLeft], Vec2::NEG_X),
        ([KeyCode::KeyD, KeyCode::ArrowRight], Vec2::X),
        ([KeyCode::KeyS, KeyCode::ArrowDown], Vec2::NEG_Y),
        ([KeyCode::KeyW, KeyCode::ArrowUp], Vec2::Y),
    ] {
        if keys.any_pressed(keys_for_direction) {
            direction += step;
        }
    }
    if window.focused
        && touches.iter().next().is_none()
        && !touches.any_just_pressed()
        && !touches.any_just_released()
        && !touches.any_just_canceled()
        && let Some(cursor) = window.cursor_position()
    {
        let far = window_size - settings.edge_margin;
        direction.x += f32::from(cursor.x >= far.x) - f32::from(cursor.x <= settings.edge_margin);
        direction.y += f32::from(cursor.y <= settings.edge_margin) - f32::from(cursor.y >= far.y);
    }
    view.pan(direction.normalize_or_zero() * settings.pan_speed * time.delta_secs());

    // Middle-mouse drag.
    let cursor_delta: Vec2 = cursor_moves.read().filter_map(|moved| moved.delta).sum();
    if mouse_buttons.pressed(MouseButton::Middle) {
        view.drag(cursor_delta);
    }

    // Mouse-wheel zoom toward the cursor.
    let wheel_lines = scroll.to_lines(&pixels_per_line).delta.y;
    if wheel_lines != 0.0 {
        let anchor = window.cursor_position().unwrap_or(window_size / 2.0);
        view.zoom_at(anchor, settings.wheel_zoom_step.powf(-wheel_lines));
    }

    // Touch drag and pinch. Positions are tracked here because `Touch::previous_position`
    // is only refreshed on frames that carry touch events.
    let mut current_touches: Vec<_> = touches
        .iter()
        .map(|touch| (touch.id(), touch.position()))
        .collect();
    current_touches.sort_unstable_by_key(|&(id, _)| id);
    if let Some(gesture) = TouchGesture::between(&previous_touches, &current_touches) {
        view.zoom_at(gesture.from, gesture.zoom);
        view.drag(gesture.to - gesture.from);
    }
    *previous_touches = current_touches;

    view.clamp_to(bounds.0);
    transform.translation = view.center.extend(transform.translation.z);
    ortho.scale = view.scale;
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
    min_scale: f32,
    max_scale: f32,
}

impl View {
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

    /// Keeps the visible area inside `bounds`, centering it on axes where it is larger.
    fn clamp_to(&mut self, bounds: Rect) {
        self.scale = self.scale.clamp(self.min_scale, self.max_scale);
        let half_view = (self.window_size / 2.0 * self.scale).min(bounds.half_size());
        self.center = self
            .center
            .clamp(bounds.min + half_view, bounds.max - half_view);
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
    use super::*;

    fn view() -> View {
        View {
            center: Vec2::ZERO,
            scale: 1.0,
            window_size: Vec2::new(800.0, 600.0),
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
}
