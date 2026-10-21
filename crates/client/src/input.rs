//! Mouse and touch selection. Gameplay actions only enqueue simulation commands.

use std::collections::BTreeSet;

use bevy::{
    input::touch::{Touch, Touches},
    prelude::*,
    window::PrimaryWindow,
};
use rts_assets::Graphics;
use rts_base::{Command, IVec2 as SimPos, UnitId};

use crate::{
    camera::{CameraControl, RtsCamera},
    game::{AppState, Match},
    hud::{HudLayout, MoveTargeting, PhoneHud},
    map::TILE_PX,
    menu::Menu,
    pause_menu::{PAUSE_BUTTON_BOUNDS, PauseMenu, world_input_blocked},
    units::{SyncUnits, UnitBounds, UnitVisual},
};

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
    was_blocked: bool,
}

impl PointerState {
    /// Cancel pending gestures on capture, and consume the closing frame too.
    fn capture(&mut self, blocked: bool, touches: &Touches) -> bool {
        let captured = blocked || self.was_blocked;
        self.was_blocked = blocked;
        if captured {
            self.mouse = None;
            self.finger = None;
            self.touch_blocked = touches.iter().next().is_some();
            self.suppress_mouse = true;
        }
        captured
    }

    fn mouse_press(
        &mut self,
        window: &Window,
        mouse: &ButtonInput<MouseButton>,
        view: &ScreenView,
        additive: bool,
    ) -> Option<Press> {
        if mouse.just_pressed(MouseButton::Left) {
            self.mouse = window
                .cursor_position()
                .filter(|&position| view.playable(position))
                .map(|position| Press::new(position, view.world(position), additive));
        }
        let press = self.mouse.as_mut()?;
        if let Some(position) = window.cursor_position() {
            press.update(position, 8.0);
        }
        if mouse.just_released(MouseButton::Left) {
            let completed = window
                .cursor_position()
                .is_some_and(|position| view.playable(position))
                .then_some(*press);
            self.mouse = None;
            return completed;
        }
        if !mouse.pressed(MouseButton::Left) {
            self.mouse = None;
        }
        None
    }

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
    /// Interface scale; the HUD and pause button grow with it.
    ui_scale: f32,
    hud: HudLayout,
}

impl ScreenView {
    fn world(&self, screen: Vec2) -> Vec2 {
        let offset = screen - self.size / 2.0;
        self.center + Vec2::new(offset.x, -offset.y) * self.scale
    }

    fn playable(&self, screen: Vec2) -> bool {
        world_pointer_allowed(screen, self.size, self.ui_scale, &self.hud)
    }
}

/// Pointer positions are in window pixels; UI bounds are in logical pixels before scaling.
pub fn world_pointer_allowed(screen: Vec2, size: Vec2, ui_scale: f32, hud: &HudLayout) -> bool {
    screen.cmpge(Vec2::ZERO).all()
        && screen.x < size.x
        && screen.y < size.y
        && !hud.blocks_point(screen / ui_scale)
        && !PAUSE_BUTTON_BOUNDS.contains(screen / ui_scale)
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
    ui_scale: Res<UiScale>,
    camera: Single<(&Transform, &Projection), With<RtsCamera>>,
    visuals: Query<(&UnitVisual, &Transform)>,
    pause: Option<Res<PauseMenu>>,
    menu: Option<Res<Menu>>,
    mut targeting: Option<ResMut<MoveTargeting>>,
    phone_hud: Option<Res<PhoneHud>>,
) {
    let (transform, Projection::Orthographic(projection)) = camera.into_inner() else {
        return;
    };
    selection.units.retain(|id| {
        game.sim
            .unit(*id)
            .is_some_and(|unit| unit.owner == game.local_player)
    });
    let panel = phone_hud.as_deref().and_then(|hud| hud.panel);
    let hud = HudLayout::new(
        window.size() / ui_scale.0,
        !selection.units.is_empty(),
        panel,
    );
    let blocked =
        world_input_blocked(pause.as_deref(), menu.as_deref()) || (hud.phone && panel.is_some());
    let view = ScreenView {
        center: transform.translation.truncate(),
        scale: projection.scale,
        size: window.size(),
        ui_scale: ui_scale.0,
        hud,
    };
    if pointer.capture(
        blocked || !window.focused || keys.just_pressed(KeyCode::Escape),
        &touches,
    ) {
        if blocked && let Some(targeting) = targeting.as_deref_mut() {
            targeting.0 = false;
        }
        return;
    }
    update_move_targeting(targeting.as_deref_mut(), &game, &selection, &keys);
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
                targeting.as_deref_mut(),
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
    }
    if let Some(press) = pointer.mouse_press(&window, &mouse, &view, additive) {
        finish_press(
            press,
            false,
            &view,
            &visuals,
            &mut selection,
            &mut game,
            &mut feedback,
            time.elapsed_secs_f64(),
            targeting.as_deref_mut(),
        );
    }
    if mouse.just_pressed(MouseButton::Right) {
        if let Some(targeting) = targeting.as_deref_mut() {
            targeting.0 = false;
        }
        if let Some(position) = window
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
}

fn update_move_targeting(
    targeting: Option<&mut MoveTargeting>,
    game: &Match,
    selection: &Selection,
    keys: &ButtonInput<KeyCode>,
) {
    if let Some(targeting) = targeting {
        if !game.accepts_orders || selection.units.is_empty() {
            targeting.0 = false;
        } else if keys.just_pressed(KeyCode::KeyM) {
            targeting.0 = true;
        }
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
    targeting: Option<&mut MoveTargeting>,
) {
    let position = view.world(press.current);
    if let Some(targeting) = targeting.filter(|targeting| targeting.0) {
        targeting.0 = false;
        issue_move(position, selection, game, feedback, now);
        return;
    }
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
    graphics: Res<Graphics>,
    selection: Res<Selection>,
    pointer: Res<PointerState>,
    feedback: Res<OrderFeedback>,
    time: Res<Time>,
    game: Res<Match>,
    camera: Single<(&Transform, &Projection), With<RtsCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    visuals: Query<(&UnitVisual, &Transform)>,
    phone_hud: Option<Res<PhoneHud>>,
) {
    let (transform, Projection::Orthographic(projection)) = camera.into_inner() else {
        return;
    };
    let style = &graphics.selection;
    for (visual, transform) in &visuals {
        if selection.units.contains(&visual.0)
            && let Some(unit) = game.sim.unit(visual.0)
        {
            let definition = game.sim.rules().unit(unit.kind);
            let bounds = UnitBounds::from_definition(definition).world_rect(transform);
            if definition.visual_bounds.is_some() {
                gizmos.rect_2d(
                    Isometry2d::from_translation(bounds.center()),
                    bounds.size() + Vec2::splat(2.0 * style.padding * projection.scale),
                    style.color,
                );
            } else {
                gizmos.circle_2d(
                    bounds.center(),
                    style
                        .padding
                        .mul_add(projection.scale, bounds.width() / 2.0),
                    style.color,
                );
            }
        }
    }
    let view = ScreenView {
        center: transform.translation.truncate(),
        scale: projection.scale,
        size: window.size(),
        ui_scale: ui_scale.0,
        hud: HudLayout::new(
            window.size() / ui_scale.0,
            !selection.units.is_empty(),
            phone_hud.as_deref().and_then(|hud| hud.panel),
        ),
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
            style.color,
        );
    }
    if let Some((position, accepted, started)) = feedback.0 {
        let age = time.elapsed_secs_f64() - started;
        if age < style.marker_duration {
            let color = if accepted {
                style.accepted_order_color
            } else {
                style.blocked_order_color
            };
            gizmos.circle_2d(position, style.marker_radius * projection.scale, color);
            gizmos.cross_2d(position, style.marker_cross_size * projection.scale, color);
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
            ui_scale: 1.0,
            hud: HudLayout::new(Vec2::new(800.0, 600.0), false, None),
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
            .init_resource::<UiScale>()
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
    #[test]
    fn desktop_exclusion_tracks_interface_scale_and_only_the_pause_button() {
        for ui_scale in [1.0, 1.5] {
            let logical_size = Vec2::new(1200.0, 800.0);
            let size = logical_size * ui_scale;
            let hud = HudLayout::new(logical_size, true, None);
            let allowed = |point| world_pointer_allowed(point * ui_scale, size, ui_scale, &hud);
            assert!(!allowed(Vec2::new(20.0, 20.0)));
            assert!(allowed(Vec2::new(100.0, 20.0)));
            assert!(allowed(Vec2::new(5.0, 5.0)));
            assert!(allowed(Vec2::new(600.0, 615.0)));
            assert!(!allowed(Vec2::new(600.0, 616.0)));
            assert!(!allowed(Vec2::new(-1.0, 100.0)));
            assert!(!allowed(Vec2::new(1200.0, 100.0)));
        }
    }

    #[test]
    fn phone_exclusion_uses_visible_controls_selection_and_modal_state_at_each_scale() {
        for logical_size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for ui_scale in [1.0, 1.5] {
                let size = logical_size * ui_scale;
                let hud = HudLayout::new(logical_size, true, None);
                let allowed = |point| world_pointer_allowed(point * ui_scale, size, ui_scale, &hud);
                let bottom_left = Vec2::new(20.0, logical_size.y - 20.0);
                let dock = hud.commands.unwrap().center();
                assert!(allowed(bottom_left));
                assert!(!allowed(dock));
                assert!(!allowed(hud.minimap.center()));
                assert!(!allowed(Vec2::new(20.0, 20.0)));
                assert!(!allowed(Vec2::new(20.0, logical_size.y)));
                let empty = HudLayout::new(logical_size, false, None);
                assert!(world_pointer_allowed(
                    dock * ui_scale,
                    size,
                    ui_scale,
                    &empty
                ));
                for panel in [
                    crate::hud::PhonePanel::Map,
                    crate::hud::PhonePanel::Selection,
                ] {
                    let modal = HudLayout::new(logical_size, true, Some(panel));
                    assert!(!world_pointer_allowed(
                        bottom_left * ui_scale,
                        size,
                        ui_scale,
                        &modal
                    ));
                    assert!(!world_pointer_allowed(size / 2.0, size, ui_scale, &modal));
                }
            }
        }
    }

    fn phone_app(logical_size: Vec2, ui_scale: f32) -> App {
        let mut app = test_app();
        app.insert_resource(UiScale(ui_scale));
        app.insert_resource(PhoneHud::default());
        app.world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>()
            .single_mut(app.world_mut())
            .unwrap()
            .resolution
            .set(logical_size.x * ui_scale, logical_size.y * ui_scale);
        {
            let mut projection = app
                .world_mut()
                .query_filtered::<&mut Projection, With<RtsCamera>>()
                .single_mut(app.world_mut())
                .unwrap();
            let Projection::Orthographic(projection) = projection.as_mut() else {
                panic!("orthographic camera");
            };
            projection.scale = 0.25;
        }
        app.world_mut()
            .resource_mut::<Selection>()
            .units
            .insert(UnitId(0));
        app
    }

    #[test]
    fn phone_controls_consume_taps_and_empty_bottom_left_issues_a_move() {
        for size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for ui_scale in [1.0, 1.5] {
                let mut app = phone_app(size, ui_scale);
                let hud = HudLayout::new(size, true, None);
                tap(&mut app, hud.commands.unwrap().center() * ui_scale);
                tap(&mut app, hud.minimap.center() * ui_scale);
                assert_eq!(app.world().resource::<Match>().pending, []);
                tap(&mut app, Vec2::new(20.0, size.y - 20.0) * ui_scale);
                assert!(
                    matches!(app.world().resource::<Match>().pending.as_slice(), [Command::Move { units, .. }] if units == &[UnitId(0)])
                );
                assert_eq!(app.world().resource::<Match>().sim.tick(), 0);
                app.world_mut().resource_mut::<Match>().pending.clear();
                app.world_mut().resource_mut::<Selection>().units.clear();
                let former_dock = hud.commands.unwrap().center() * ui_scale;
                touch(&mut app, &[(1, TouchPhase::Started, former_dock)]);
                assert!(app.world().resource::<PointerState>().finger.is_some());
                touch(&mut app, &[(1, TouchPhase::Ended, former_dock)]);
                assert_eq!(app.world().resource::<Match>().pending, []);
            }
        }
    }

    #[test]
    fn phone_panels_cancel_pending_presses_and_block_orders_through_close() {
        for size in [Vec2::new(390.0, 844.0), Vec2::new(844.0, 390.0)] {
            for ui_scale in [1.0, 1.5] {
                for panel in [
                    crate::hud::PhonePanel::Map,
                    crate::hud::PhonePanel::Selection,
                ] {
                    let mut app = phone_app(size, ui_scale);
                    let position = Vec2::new(20.0, size.y - 20.0) * ui_scale;
                    mouse_event(&mut app, position, MouseButton::Left, true);
                    app.world_mut().resource_mut::<PhoneHud>().panel = Some(panel);
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(KeyCode::KeyX);
                    mouse_event(&mut app, position, MouseButton::Right, true);
                    touch(&mut app, &[(1, TouchPhase::Started, position)]);
                    assert_eq!(app.world().resource::<Match>().pending, []);
                    assert!(app.world().resource::<PointerState>().mouse.is_none());
                    app.world_mut().resource_mut::<PhoneHud>().panel = None;
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .reset_all();
                    app.update();
                    touch(&mut app, &[(1, TouchPhase::Ended, position)]);
                    mouse_event(&mut app, position, MouseButton::Left, false);
                    mouse_event(&mut app, position, MouseButton::Right, false);
                    assert_eq!(app.world().resource::<Match>().pending, []);
                    tap(&mut app, position);
                    assert_eq!(app.world().resource::<Match>().pending.len(), 1);
                    assert_eq!(
                        app.world().resource::<Selection>().units,
                        BTreeSet::from([UnitId(0)])
                    );
                    assert_eq!(app.world().resource::<Match>().sim.tick(), 0);
                }
            }
        }
    }

    #[test]
    fn escape_keeps_selection_and_cancels_an_active_box() {
        let mut app = test_app();
        click(&mut app, Vec2::new(336.0, 236.0));
        mouse_event(&mut app, Vec2::new(500.0, 300.0), MouseButton::Left, true);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        mouse_event(&mut app, Vec2::new(550.0, 350.0), MouseButton::Left, false);
        assert_eq!(
            app.world().resource::<Selection>().units,
            BTreeSet::from([UnitId(0)])
        );
        assert!(app.world().resource::<PointerState>().mouse.is_none());
    }

    #[test]
    fn pause_and_dialogs_block_orders_and_cancel_mouse_and_touch_through_resume() {
        for dialog in [false, true] {
            let mut app = test_app();
            click(&mut app, Vec2::new(336.0, 236.0));
            mouse_event(&mut app, Vec2::new(500.0, 300.0), MouseButton::Left, true);
            app.insert_resource(PauseMenu { open: !dialog });
            let mut menu = Menu::default();
            if dialog {
                menu.dialog = Some(crate::menu::Dialog::Confirm(
                    crate::menu::Confirm::LeaveMatch,
                ));
            }
            app.insert_resource(menu);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyX);
            mouse_event(&mut app, Vec2::new(400.0, 300.0), MouseButton::Right, true);
            touch(
                &mut app,
                &[(1, TouchPhase::Started, Vec2::new(400.0, 300.0))],
            );
            assert_eq!(app.world().resource::<Match>().pending, []);
            app.world_mut().resource_mut::<PauseMenu>().open = false;
            app.world_mut().resource_mut::<Menu>().dialog = None;
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
            app.update();
            touch(&mut app, &[(1, TouchPhase::Ended, Vec2::new(450.0, 350.0))]);
            mouse_event(&mut app, Vec2::new(500.0, 350.0), MouseButton::Left, false);
            assert_eq!(app.world().resource::<Match>().pending, []);
            assert_eq!(
                app.world().resource::<Selection>().units,
                BTreeSet::from([UnitId(0)])
            );
        }
    }

    #[test]
    fn move_targeting_uses_primary_mouse_and_touch_without_changing_selection() {
        for touch_input in [false, true] {
            let mut app = test_app();
            app.insert_resource(MoveTargeting(false));
            click(&mut app, Vec2::new(336.0, 236.0));
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyM);
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .reset_all();
            assert!(app.world().resource::<MoveTargeting>().0);
            if touch_input {
                tap(&mut app, Vec2::new(400.0, 300.0));
            } else {
                // An ordinary empty-world click would clear selection.
                click(&mut app, Vec2::new(400.0, 300.0));
            }
            assert!(!app.world().resource::<MoveTargeting>().0);
            assert_eq!(
                app.world().resource::<Selection>().units,
                BTreeSet::from([UnitId(0)])
            );
            assert_eq!(
                app.world().resource::<Match>().pending,
                vec![Command::Move {
                    units: vec![UnitId(0)],
                    target: SimPos::splat(2 * rts_base::TILE),
                }]
            );
        }
    }

    #[test]
    fn move_targeting_requires_orders_and_selection_and_is_canceled_by_pause_or_right_click() {
        let mut app = test_app();
        app.insert_resource(MoveTargeting(false));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        assert!(!app.world().resource::<MoveTargeting>().0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        click(&mut app, Vec2::new(336.0, 236.0));
        app.world_mut().resource_mut::<Match>().accepts_orders = false;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        assert!(!app.world().resource::<MoveTargeting>().0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut().resource_mut::<Match>().accepts_orders = true;
        app.world_mut().resource_mut::<MoveTargeting>().0 = true;
        mouse_event(&mut app, Vec2::new(400.0, 300.0), MouseButton::Right, true);
        assert!(!app.world().resource::<MoveTargeting>().0);
        assert_eq!(app.world().resource::<Match>().pending.len(), 1);
        app.world_mut().resource_mut::<MoveTargeting>().0 = true;
        app.insert_resource(PauseMenu { open: true });
        app.update();
        assert!(!app.world().resource::<MoveTargeting>().0);
    }
}
