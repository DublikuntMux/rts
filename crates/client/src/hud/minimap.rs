//! A terrain image built once, with live unit markers and a camera footprint.

use std::collections::BTreeMap;

use bevy::{
    asset::RenderAssetUsages,
    color::{ColorToPacked, Srgba},
    image::ImageSampler,
    picking::{
        Pickable,
        events::{PointerClick, PointerDrag},
        pointer::PointerButton,
    },
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui_widgets::observe,
    window::PrimaryWindow,
};
use rts_assets::Graphics;
use rts_base::{Sim, UnitId};

use super::{HUD_HEIGHT, HudLayout, PhoneHud, PhonePanel, hud_inset};
use crate::{
    camera::RtsCamera,
    game::Match,
    input::Selection,
    map::{MapBounds, to_world},
    menu::widgets::{EDGE, FIELD, INK, RAIL, SIGNAL},
    pause_menu::PauseMenu,
};

#[derive(Component)]
pub(super) struct Minimap;
#[derive(Component)]
pub(super) struct MapImage;
#[derive(Component)]
pub(super) struct CameraFootprint;
#[derive(Component)]
pub(super) struct UnitMarker(UnitId);

pub(super) fn spawn(commands: &mut Commands, game: &Match, images: &mut Assets<Image>) -> Entity {
    let image = images.add(terrain_image(&game.sim));
    let aspect = game.sim.terrain().size().as_vec2();
    let root = commands
        .spawn((
            Minimap,
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
        ))
        .id();
    commands.entity(root).with_children(|well| {
        well.spawn((
            MapImage,
            Node {
                width: px(168),
                height: px(168.0 * aspect.y / aspect.x),
                max_height: px(168),
                border: UiRect::all(px(1)),
                overflow: Overflow::clip(),
                ..default()
            },
            ImageNode::new(image),
            BorderColor::all(EDGE),
            BackgroundColor(FIELD),
            observe(click_map),
            observe(drag_map),
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

fn terrain_image(sim: &Sim) -> Image {
    let size = sim.terrain().size();
    let colors: Vec<_> = sim
        .rules()
        .terrain_types()
        .iter()
        .map(|terrain| {
            Srgba::hex(&terrain.color)
                .unwrap_or(Srgba::rgb(1.0, 0.0, 1.0))
                .to_u8_array()
        })
        .collect();
    let pixels = sim
        .terrain()
        .tiles()
        .iter()
        .flat_map(|tile| colors[usize::from(tile.0)])
        .collect();
    let mut image = Image::new(
        Extent3d {
            width: u32::try_from(size.x).unwrap_or(1),
            height: u32::try_from(size.y).unwrap_or(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

type MapNodes<'a> = (
    &'a mut Node,
    Has<Minimap>,
    Has<MapImage>,
    Has<CameraFootprint>,
);
type MarkerFilter = (
    With<UnitMarker>,
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
    mut nodes: Query<MapNodes, Without<UnitMarker>>,
    map: Single<Entity, With<MapImage>>,
    root: Single<Entity, With<Minimap>>,
    markers: Query<(Entity, &UnitMarker)>,
    mut marker_styles: Query<(&mut Node, &mut BackgroundColor), MarkerFilter>,
) {
    let size = window.size() / scale.0;
    let layout = HudLayout::new(size, !selection.units.is_empty(), phone.panel);
    let dimensions = game.sim.terrain().size().as_vec2();
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
    let mut existing: BTreeMap<_, _> = markers
        .iter()
        .map(|(entity, marker)| (marker.0, entity))
        .collect();
    for (id, unit) in game.sim.units() {
        let position = map_fraction(to_world(unit.pos), bounds.0);
        let node = Node {
            position_type: PositionType::Absolute,
            left: percent(position.x * 100.0),
            top: percent(position.y * 100.0),
            width: px(4),
            height: px(4),
            margin: UiRect::all(px(-2)),
            ..default()
        };
        let color = if selection.units.contains(&id) {
            SIGNAL
        } else {
            graphics.player_color(unit.owner.0)
        };
        if let Some(entity) = existing.remove(&id) {
            if let Ok((mut old_node, mut fill)) = marker_styles.get_mut(entity) {
                old_node.set_if_neq(node);
                fill.set_if_neq(BackgroundColor(color));
            }
        } else {
            let marker = commands
                .spawn((
                    UnitMarker(id),
                    Pickable::IGNORE,
                    node,
                    BackgroundColor(color),
                ))
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

fn click_map(
    event: On<PointerClick>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform), With<MapImage>>,
    window: Single<&Window, With<PrimaryWindow>>,
    bounds: Res<MapBounds>,
    pause: Res<PauseMenu>,
    scale: Res<UiScale>,
    mut phone: ResMut<PhoneHud>,
    map: Single<Entity, With<MapImage>>,
    camera: Single<(&mut Transform, &Projection), With<RtsCamera>>,
) {
    if event.button == PointerButton::Primary && !pause.open {
        if super::phone_layout(window.size() / scale.0) && phone.panel != Some(PhonePanel::Map) {
            phone.panel = Some(PhonePanel::Map);
            return;
        }
        recenter(
            *map,
            event.pointer.position,
            &nodes,
            &window,
            bounds.0,
            scale.0,
            camera,
        );
    }
}

fn drag_map(
    event: On<PointerDrag>,
    nodes: Query<(&ComputedNode, &UiGlobalTransform), With<MapImage>>,
    window: Single<&Window, With<PrimaryWindow>>,
    bounds: Res<MapBounds>,
    pause: Res<PauseMenu>,
    scale: Res<UiScale>,
    phone: Res<PhoneHud>,
    map: Single<Entity, With<MapImage>>,
    camera: Single<(&mut Transform, &Projection), With<RtsCamera>>,
) {
    if event.button == PointerButton::Primary
        && !pause.open
        && (!super::phone_layout(window.size() / scale.0) || phone.panel == Some(PhonePanel::Map))
    {
        recenter(
            *map,
            event.pointer.position,
            &nodes,
            &window,
            bounds.0,
            scale.0,
            camera,
        );
    }
}

fn recenter(
    entity: Entity,
    position: Vec2,
    nodes: &Query<(&ComputedNode, &UiGlobalTransform), With<MapImage>>,
    window: &Window,
    bounds: Rect,
    scale: f32,
    camera: Single<(&mut Transform, &Projection), With<RtsCamera>>,
) {
    let Ok((node, transform)) = nodes.get(entity) else {
        return;
    };
    let Some(point) = node.normalize_point(*transform, position * window.scale_factor()) else {
        return;
    };
    let point = (point + Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::ONE);
    let (mut transform, projection) = camera.into_inner();
    let Projection::Orthographic(projection) = projection else {
        return;
    };
    let inset = hud_inset(window.size() / scale) * scale;
    let visible_size = Vec2::new(window.width(), (window.height() - inset).max(1.0));
    let half = (visible_size * projection.scale / 2.0).min(bounds.half_size());
    let target = Vec2::new(
        bounds.width().mul_add(point.x, bounds.min.x),
        bounds.height().mul_add(-point.y, bounds.max.y),
    );
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
