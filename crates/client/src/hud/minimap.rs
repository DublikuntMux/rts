//! The known terrain, redrawn as fog changes, with live unit markers and a camera footprint.

use bevy::{
    asset::RenderAssetUsages,
    color::{ColorToPacked, Srgba},
    image::ImageSampler,
    picking::{
        Pickable,
        events::{PointerCancel, PointerClick, PointerDrag, PointerDragEnd, PointerPress},
        pointer::{PointerButton, PointerId},
    },
    platform::collections::{HashMap, HashSet},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui_widgets::observe,
    window::PrimaryWindow,
};
use rts_assets::Graphics;
use rts_base::{IVec2, PlayerView, TileVisibility, UnitId};

use super::{HUD_HEIGHT, HudLayout, PhoneHud, PhonePanel, Targeting, hud_inset};
use crate::{
    camera::RtsCamera,
    game::Match,
    input::{OrderFeedback, Selection, order_from_minimap},
    map::{MapBounds, to_world},
    menu::Menu,
    menu::widgets::{FIELD, INK, RAIL, SIGNAL, bevel_recessed},
    pause_menu::{PauseMenu, world_input_blocked},
};

#[derive(Component, Default)]
pub(super) struct Minimap {
    dragging: HashSet<PointerId>,
}
#[derive(Component)]
pub(super) struct MapImage(u32);
#[derive(Component)]
pub(super) struct CameraFootprint;
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Marker {
    Unit(UnitId),
    Rally(UnitId),
}

pub(super) fn spawn(commands: &mut Commands, game: &Match, images: &mut Assets<Image>) -> Entity {
    let image = images.add(terrain_image(&game.view));
    let aspect = game.view.size().as_vec2();
    let root = commands
        .spawn((
            Minimap::default(),
            Name::new("Minimap"),
            Node {
                width: px(HUD_HEIGHT),
                height: percent(100),
                flex_shrink: 0.0,
                padding: UiRect::all(px(8)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(RAIL),
            observe(press_map),
            observe(click_map),
            observe(drag_map),
            observe(end_map_drag),
            observe(cancel_map_drag),
        ))
        .id();
    commands.entity(root).with_children(|well| {
        well.spawn((
            MapImage(game.view.terrain_revision()),
            Node {
                width: px(168),
                height: px(168.0 * aspect.y / aspect.x),
                max_height: px(168),
                border: UiRect::all(px(2)),
                overflow: Overflow::clip(),
                ..default()
            },
            ImageNode::new(image),
            bevel_recessed(),
            BackgroundColor(FIELD),
            Pickable::IGNORE,
        ))
        .with_children(|map| {
            map.spawn((
                CameraFootprint,
                Pickable::IGNORE,
                Node {
                    position_type: PositionType::Absolute,
                    border: UiRect::all(px(1)),
                    ..default()
                },
                BorderColor::all(INK),
            ));
        });
    });
    root
}

/// Known terrain colors: black where unexplored and darker where out of sight.
fn terrain_image(view: &PlayerView) -> Image {
    let size = view.size();
    let pixels = terrain_pixels(view);
    let mut image = Image::new(
        Extent3d {
            width: u32::try_from(size.x).unwrap_or(1),
            height: u32::try_from(size.y).unwrap_or(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

fn terrain_pixels(view: &PlayerView) -> Vec<u8> {
    let colors: Vec<_> = view
        .rules()
        .terrain_types()
        .iter()
        .map(|terrain| {
            Srgba::hex(&terrain.color)
                .unwrap_or(Srgba::rgb(1.0, 0.0, 1.0))
                .to_u8_array()
        })
        .collect();
    let width = view.size().x;
    view.terrain_tiles()
        .iter()
        .enumerate()
        .flat_map(|(index, terrain)| {
            let index = i32::try_from(index).expect("bounded map size");
            let tile = IVec2::new(index % width, index / width);
            terrain.map_or([0, 0, 0, 255], |terrain| {
                let [r, g, b, a] = colors[usize::from(terrain.0)];
                if view.visibility(tile) == TileVisibility::Visible {
                    [r, g, b, a]
                } else {
                    [r / 5 * 2, g / 5 * 2, b / 5 * 2, a]
                }
            })
        })
        .collect()
}

/// Redraws the terrain image when discovery or current sight changes, or a new match starts.
pub(super) fn update_minimap_fog(
    game: Res<Match>,
    map: Single<(&ImageNode, &mut MapImage)>,
    mut images: ResMut<Assets<Image>>,
) {
    let revision = game.view.terrain_revision();
    let (map, mut drawn) = map.into_inner();
    if (drawn.0 != revision || game.is_added())
        && let Some(mut image) = images.get_mut(&map.image)
    {
        image.data = Some(terrain_pixels(&game.view));
        drawn.0 = revision;
    }
}

type MapNodes<'a> = (
    &'a mut Node,
    Has<Minimap>,
    Has<MapImage>,
    Has<CameraFootprint>,
);
type MarkerFilter = (
    With<Marker>,
    Without<Minimap>,
    Without<MapImage>,
    Without<CameraFootprint>,
);

// Node percentages don't depend on the UI layout result, so they can update before layout.
pub(super) fn update_minimap(
    mut commands: Commands,
    game: Res<Match>,
    graphics: Res<Graphics>,
    selection: Res<Selection>,
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    phone: Res<PhoneHud>,
    bounds: Res<MapBounds>,
    camera: Single<(&Transform, &Projection), With<RtsCamera>>,
    mut nodes: Query<MapNodes, Without<Marker>>,
    map: Single<Entity, With<MapImage>>,
    root: Single<Entity, With<Minimap>>,
    markers: Query<(Entity, &Marker)>,
    mut marker_styles: Query<(&mut Node, &mut BackgroundColor), MarkerFilter>,
    mut existing: Local<HashMap<Marker, Entity>>,
) {
    let size = window.size() / scale.0;
    let layout = HudLayout::new(size, !selection.units.is_empty(), phone.panel);
    let dimensions = game.view.size().as_vec2();
    let available = layout.minimap.size() - Vec2::splat(16.0);
    let image_size = dimensions * (available / dimensions).min_element();
    commands.entity(*root).insert(GlobalZIndex(
        if layout.phone && phone.panel == Some(PhonePanel::Map) {
            4
        } else {
            0
        },
    ));
    let (camera, projection) = camera.into_inner();
    let rect = if let Projection::Orthographic(projection) = projection {
        footprint(
            camera.translation.truncate()
                + Vec2::Y * hud_inset(size) * scale.0 * projection.scale / 2.0,
            Vec2::new(
                window.width(),
                hud_inset(size).mul_add(-scale.0, window.height()).max(1.0),
            ),
            projection.scale,
            bounds.0,
        )
    } else {
        Rect::default()
    };
    for (mut node, root, image, footprint) in &mut nodes {
        if root {
            node.position_type = if layout.phone {
                PositionType::Absolute
            } else {
                PositionType::Relative
            };
            node.left = if layout.phone {
                px(layout.minimap.min.x)
            } else {
                auto()
            };
            node.top = if layout.phone {
                px(layout.minimap.min.y)
            } else {
                auto()
            };
            node.width = px(layout.minimap.width());
            node.height = px(layout.minimap.height());
            node.display = Display::Flex;
        }
        if image {
            node.width = px(image_size.x);
            node.height = px(image_size.y);
            node.max_height = auto();
        }
        if footprint {
            node.left = percent(rect.min.x * 100.0);
            node.top = percent(rect.min.y * 100.0);
            node.width = percent(rect.width() * 100.0);
            node.height = percent(rect.height() * 100.0);
        }
    }
    existing.clear();
    existing.extend(markers.iter().map(|(entity, marker)| (*marker, entity)));
    let units = game.view.units().map(|(id, unit)| {
        let color = if selection.units.contains(&id) {
            SIGNAL
        } else {
            graphics.player_color(unit.owner.0)
        };
        (Marker::Unit(id), unit.pos, color, 4.0)
    });
    let rallies = selection.units.iter().filter_map(|&id| {
        let point = game.view.rally_point(id)?;
        Some((Marker::Rally(id), point, INK, 6.0))
    });
    for (marker, position, color, size) in units.chain(rallies) {
        let position = map_fraction(to_world(position), bounds.0);
        let node = Node {
            position_type: PositionType::Absolute,
            left: percent(position.x * 100.0),
            top: percent(position.y * 100.0),
            width: px(size),
            height: px(size),
            margin: UiRect::all(px(-size / 2.0)),
            ..default()
        };
        if let Some(entity) = existing.remove(&marker) {
            if let Ok((mut old_node, mut fill)) = marker_styles.get_mut(entity) {
                old_node.set_if_neq(node);
                fill.set_if_neq(BackgroundColor(color));
            }
        } else {
            let marker = commands
                .spawn((marker, Pickable::IGNORE, node, BackgroundColor(color)))
                .id();
            commands.entity(*map).add_child(marker);
        }
    }
    for entity in existing.values() {
        commands.entity(*entity).despawn();
    }
}

fn map_fraction(world: Vec2, bounds: Rect) -> Vec2 {
    Vec2::new(
        (world.x - bounds.min.x) / bounds.width(),
        (bounds.max.y - world.y) / bounds.height(),
    )
    .clamp(Vec2::ZERO, Vec2::ONE)
}

fn footprint(center: Vec2, screen: Vec2, zoom: f32, bounds: Rect) -> Rect {
    let half = screen * zoom / 2.0;
    Rect::from_corners(
        map_fraction(center + Vec2::new(-half.x, half.y), bounds),
        map_fraction(center + Vec2::new(half.x, -half.y), bounds),
    )
}

fn press_map(event: On<PointerPress>, mut map: Single<&mut Minimap>) {
    if event.button == PointerButton::Primary {
        map.dragging.remove(&event.pointer.id);
    }
}

fn end_map_drag(event: On<PointerDragEnd>, mut map: Single<&mut Minimap>) {
    if event.button == PointerButton::Primary {
        map.dragging.remove(&event.pointer.id);
    }
}

fn cancel_map_drag(event: On<PointerCancel>, mut map: Single<&mut Minimap>) {
    map.dragging.remove(&event.pointer.id);
}

fn click_map(
    event: On<PointerClick>,
    node: Single<(&ComputedNode, &UiGlobalTransform), With<MapImage>>,
    window: Single<&Window, With<PrimaryWindow>>,
    bounds: Res<MapBounds>,
    pause: Res<PauseMenu>,
    menu: Option<Res<Menu>>,
    scale: Res<UiScale>,
    mut phone: ResMut<PhoneHud>,
    map: Single<&Minimap>,
    camera: Single<(&mut Transform, &Projection), With<RtsCamera>>,
    mut game: ResMut<Match>,
    selection: Res<Selection>,
    mut feedback: ResMut<OrderFeedback>,
    time: Res<Time>,
    mut targeting: ResMut<Targeting>,
) {
    if event.button != PointerButton::Primary
        || map.dragging.contains(&event.pointer.id)
        || world_input_blocked(Some(&pause), menu.as_deref())
        || phone.panel == Some(PhonePanel::Selection)
    {
        return;
    }
    let Some(target) = map_point(node.into_inner(), event.pointer.position, &window, bounds.0)
    else {
        return;
    };
    if order_from_minimap(
        target,
        &selection,
        &mut game,
        &mut feedback,
        time.elapsed_secs_f64(),
        &mut targeting,
    ) {
        return;
    }
    if super::phone_layout(window.size() / scale.0) && phone.panel != Some(PhonePanel::Map) {
        phone.panel = Some(PhonePanel::Map);
        return;
    }
    recenter(target, &window, bounds.0, scale.0, camera);
}

fn drag_map(
    event: On<PointerDrag>,
    node: Single<(&ComputedNode, &UiGlobalTransform), With<MapImage>>,
    window: Single<&Window, With<PrimaryWindow>>,
    bounds: Res<MapBounds>,
    pause: Res<PauseMenu>,
    menu: Option<Res<Menu>>,
    scale: Res<UiScale>,
    phone: Res<PhoneHud>,
    mut map: Single<&mut Minimap>,
    camera: Single<(&mut Transform, &Projection), With<RtsCamera>>,
) {
    if event.button != PointerButton::Primary
        || world_input_blocked(Some(&pause), menu.as_deref())
        || phone.panel == Some(PhonePanel::Selection)
    {
        return;
    }
    if event.distance.length_squared() >= 64.0 {
        map.dragging.insert(event.pointer.id);
    }
    if map.dragging.contains(&event.pointer.id)
        && (!super::phone_layout(window.size() / scale.0) || phone.panel == Some(PhonePanel::Map))
        && let Some(target) =
            map_point(node.into_inner(), event.pointer.position, &window, bounds.0)
    {
        recenter(target, &window, bounds.0, scale.0, camera);
    }
}

fn map_point(
    (node, transform): (&ComputedNode, &UiGlobalTransform),
    position: Vec2,
    window: &Window,
    bounds: Rect,
) -> Option<Vec2> {
    let point = node.normalize_point(*transform, position * window.scale_factor())?;
    let point = (point + Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::ONE);
    Some(Vec2::new(
        bounds.width().mul_add(point.x, bounds.min.x),
        bounds.height().mul_add(-point.y, bounds.max.y),
    ))
}

fn recenter(
    target: Vec2,
    window: &Window,
    bounds: Rect,
    scale: f32,
    camera: Single<(&mut Transform, &Projection), With<RtsCamera>>,
) {
    let (mut transform, projection) = camera.into_inner();
    let Projection::Orthographic(projection) = projection else {
        return;
    };
    let inset = hud_inset(window.size() / scale) * scale;
    let visible_size = Vec2::new(window.width(), (window.height() - inset).max(1.0));
    let half = (visible_size * projection.scale / 2.0).min(bounds.half_size());
    let target = target.clamp(bounds.min + half, bounds.max - half);
    transform.translation =
        (target - Vec2::Y * inset * projection.scale / 2.0).extend(transform.translation.z);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_footprint_and_unit_markers_share_map_coordinates() {
        let bounds = Rect::from_corners(Vec2::new(0.0, -1000.0), Vec2::new(2000.0, 0.0));
        assert_eq!(map_fraction(Vec2::new(0.0, 0.0), bounds), Vec2::ZERO);
        assert_eq!(map_fraction(Vec2::new(2000.0, -1000.0), bounds), Vec2::ONE);
        let rect = footprint(
            Vec2::new(1000.0, -500.0),
            Vec2::new(800.0, 600.0),
            1.0,
            bounds,
        );
        assert_eq!(rect.min, Vec2::new(0.3, 0.2));
        assert_eq!(rect.max, Vec2::new(0.7, 0.8));
        let rect = footprint(Vec2::ZERO, Vec2::new(800.0, 600.0), 1.0, bounds);
        assert_eq!(rect.min, Vec2::ZERO);
        assert_eq!(rect.max, Vec2::new(0.2, 0.3));
    }
}
