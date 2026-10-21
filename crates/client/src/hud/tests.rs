use bevy::{state::app::StatesPlugin, window::PrimaryWindow};
use rts_base::PlayerId;
use rts_net::MatchSetup;

use super::*;
use crate::{camera::RtsCamera, map::MapBounds};

fn app() -> App {
    let setup = MatchSetup {
        rules: include_str!("../../../../assets/data/core.rules.ron").into(),
        map: include_str!("../../../../assets/maps/crossroads.map.ron").into(),
        seed: 0,
        players: 4,
    };
    let sim = setup.simulation().unwrap();
    let dimensions = sim.terrain().size().as_vec2() * crate::map::to_world_distance(rts_base::TILE);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_asset::<Font>()
        .insert_state(AppState::InGame)
        .insert_resource(Match::new(sim, PlayerId(0), true))
        .insert_resource(MapBounds(Rect::from_corners(
            Vec2::new(0.0, -dimensions.y),
            Vec2::new(dimensions.x, 0.0),
        )))
        .insert_resource(GameFont(Handle::default()))
        .insert_resource(
            Graphics::from_ron(include_str!("../../../../assets/data/client.graphics.ron"))
                .unwrap(),
        )
        .init_resource::<UiScale>()
        .init_resource::<Selection>()
        .init_resource::<PauseMenu>()
        .add_plugins(HudPlugin);
    let mut window = Window::default();
    window.resolution.set(1280.0, 720.0);
    app.world_mut().spawn((window, PrimaryWindow));
    app.world_mut().spawn((
        RtsCamera::default(),
        Transform::from_xyz(256.0, -256.0, 0.0),
        Projection::Orthographic(OrthographicProjection::default_2d()),
    ));
    app.update();
    app.update();
    app
}

fn order(app: &mut App, order: Order) {
    let entity = app
        .world_mut()
        .query::<(Entity, &Order)>()
        .iter(app.world())
        .find(|(_, candidate)| **candidate == order)
        .unwrap()
        .0;
    app.world_mut().trigger(Activate { entity });
    app.update();
}

#[test]
fn console_selection_and_orders_keep_simulation_authoritative() {
    let mut app = app();
    order(&mut app, Order::Move);
    assert!(!app.world().resource::<MoveTargeting>().0);
    let ids: Vec<_> = app
        .world()
        .resource::<Match>()
        .sim
        .units()
        .filter(|(_, unit)| unit.owner == PlayerId(0))
        .take(2)
        .map(|(id, _)| id)
        .collect();
    app.world_mut()
        .resource_mut::<Selection>()
        .units
        .extend(&ids);
    app.update();
    let hash = app.world().resource::<Match>().sim.state_hash();
    assert_eq!(
        app.world_mut()
            .query::<&RosterUnit>()
            .iter(app.world())
            .count(),
        2
    );
    order(&mut app, Order::Move);
    assert!(app.world().resource::<MoveTargeting>().0);
    order(&mut app, Order::Stop);
    assert!(!app.world().resource::<MoveTargeting>().0);
    assert_eq!(
        app.world().resource::<Match>().pending,
        vec![rts_base::Command::Stop { units: ids.clone() }]
    );
    assert_eq!(app.world().resource::<Match>().sim.state_hash(), hash);
    let (portrait, id) = app
        .world_mut()
        .query::<(Entity, &RosterUnit)>()
        .iter(app.world())
        .next()
        .map(|(entity, unit)| (entity, unit.0))
        .unwrap();
    app.world_mut().trigger(Activate { entity: portrait });
    app.update();
    assert_eq!(
        app.world().resource::<Selection>().units,
        std::collections::BTreeSet::from([id])
    );
    app.world_mut()
        .resource_mut::<Selection>()
        .units
        .extend(&ids);
    app.update();
    app.world_mut().resource_mut::<PauseMenu>().open = true;
    app.update();
    order(&mut app, Order::Clear);
    assert_eq!(app.world().resource::<Selection>().units.len(), 2);
    app.world_mut().resource_mut::<PauseMenu>().open = false;
    app.update();
    order(&mut app, Order::Clear);
    assert_eq!(app.world().resource::<Selection>().units.len(), 0);
    assert_eq!(
        app.world_mut()
            .query::<&RosterUnit>()
            .iter(app.world())
            .count(),
        0
    );
}

#[test]
fn phone_controls_float_and_keep_full_sized_targets_in_both_orientations() {
    for dimensions in [Vec2::new(320.0, 640.0), Vec2::new(844.0, 390.0)] {
        for scale in [1.0, 1.5] {
            let mut app = app();
            app.world_mut()
                .query_filtered::<&mut Window, With<PrimaryWindow>>()
                .single_mut(app.world_mut())
                .unwrap()
                .resolution
                .set(dimensions.x, dimensions.y);
            app.world_mut().resource_mut::<UiScale>().0 = scale;
            app.update();
            let (root, fill) = app
                .world_mut()
                .query_filtered::<(&Node, &BackgroundColor), With<Console>>()
                .single(app.world())
                .unwrap();
            assert_eq!(root.height, percent(100));
            assert_eq!(fill.0, Color::NONE);
            assert_eq!(
                app.world_mut()
                    .query_filtered::<&Node, With<CommandCard>>()
                    .single(app.world())
                    .unwrap()
                    .display,
                Display::None
            );
            app.world_mut()
                .resource_mut::<Selection>()
                .units
                .insert(rts_base::UnitId(0));
            app.update();
            assert_eq!(
                app.world_mut()
                    .query_filtered::<&Node, With<CommandCard>>()
                    .single(app.world())
                    .unwrap()
                    .display,
                Display::Grid
            );
            for node in app
                .world_mut()
                .query_filtered::<&Node, With<Order>>()
                .iter(app.world())
            {
                assert_eq!(node.width, px(48));
                assert_eq!(node.height, px(48));
                assert_ne!(node.display, Display::None);
            }
            let layout = HudLayout::new(dimensions / scale, true, None);
            let card = app
                .world_mut()
                .query_filtered::<&Node, With<CommandCard>>()
                .single(app.world())
                .unwrap();
            assert_eq!(card.left, px(layout.commands.unwrap().min.x));
            assert_eq!(card.top, px(layout.commands.unwrap().min.y));
            assert_eq!(
                app.world_mut()
                    .query_filtered::<&Node, With<SelectionPanel>>()
                    .single(app.world())
                    .unwrap()
                    .display,
                Display::None
            );
        }
    }
}

#[test]
fn phone_details_open_from_selection_and_close_without_leaving_hidden_focus() {
    use bevy::input_focus::{FocusCause, InputFocus};
    let mut app = app();
    app.init_resource::<InputFocus>();
    app.world_mut()
        .query_filtered::<&mut Window, With<PrimaryWindow>>()
        .single_mut(app.world_mut())
        .unwrap()
        .resolution
        .set(320.0, 640.0);
    app.world_mut()
        .resource_mut::<Selection>()
        .units
        .insert(rts_base::UnitId(0));
    app.update();
    let toggle = app
        .world_mut()
        .query_filtered::<Entity, With<phone::SelectionButton>>()
        .single(app.world())
        .unwrap();
    app.world_mut().trigger(Activate { entity: toggle });
    app.update();
    assert_eq!(
        app.world().resource::<PhoneHud>().panel,
        Some(PhonePanel::Selection)
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<&Node, With<SelectionPanel>>()
            .single(app.world())
            .unwrap()
            .display,
        Display::Flex
    );
    let close = app
        .world_mut()
        .query::<(Entity, &Name)>()
        .iter(app.world())
        .find(|(_, name)| name.as_str() == "Close HUD panel")
        .unwrap()
        .0;
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(close, FocusCause::Navigated);
    app.world_mut().trigger(Activate { entity: close });
    app.update();
    assert_eq!(app.world().resource::<PhoneHud>().panel, None);
    assert_eq!(app.world().resource::<InputFocus>().get(), None);
    assert_eq!(
        app.world_mut()
            .query_filtered::<&Node, With<SelectionPanel>>()
            .single(app.world())
            .unwrap()
            .display,
        Display::None
    );
}

#[test]
fn short_scaled_phone_scrolls_all_selection_details_together() {
    let mut app = app();
    app.world_mut()
        .query_filtered::<&mut Window, With<PrimaryWindow>>()
        .single_mut(app.world_mut())
        .unwrap()
        .resolution
        .set(568.0, 320.0);
    app.world_mut().resource_mut::<UiScale>().0 = 1.5;
    app.world_mut()
        .resource_mut::<Selection>()
        .units
        .insert(rts_base::UnitId(0));
    app.world_mut().resource_mut::<PhoneHud>().panel = Some(PhonePanel::Selection);
    app.update();

    let (panel, area) = app
        .world_mut()
        .query_filtered::<(&Node, Option<&bevy::ui_widgets::ScrollArea>), With<SelectionPanel>>()
        .single(app.world())
        .unwrap();
    let Val::Px(height) = panel.height else {
        panic!("phone details need a fixed viewport height");
    };
    assert!((height - (320.0 / 1.5 - 88.0)).abs() < 0.001);
    assert_eq!(panel.overflow, Overflow::scroll_y());
    assert!(area.is_some());
    let (roster, nested_area) = app
        .world_mut()
        .query_filtered::<(&Node, Option<&bevy::ui_widgets::ScrollArea>), With<Roster>>()
        .single(app.world())
        .unwrap();
    assert_eq!(roster.height, auto());
    assert_eq!(roster.max_height, auto());
    assert_eq!(roster.overflow, Overflow::visible());
    assert!(nested_area.is_none());
}

#[test]
fn command_labels_keep_transparent_backgrounds() {
    let mut app = app();
    let mut labels = app
        .world_mut()
        .query_filtered::<Option<&BackgroundColor>, (With<ButtonInk>, With<Text>)>();
    let mut count = 0;
    for fill in labels.iter(app.world()) {
        if let Some(fill) = fill {
            assert_eq!(fill.0.to_srgba().alpha, 0.0);
        }
        count += 1;
    }
    assert!(count >= 3);
}

#[test]
fn command_images_reference_decodable_transparent_assets() {
    use bevy::{
        asset::RenderAssetUsages,
        image::{CompressedImageFormats, ImageSampler, ImageType},
    };

    let mut app = app();
    let world = app.world_mut();
    let mut count = 0;
    for children in world.query_filtered::<&Children, With<Order>>().iter(world) {
        let icons: Vec<_> = children
            .iter()
            .filter_map(|child| world.get::<ImageNode>(child))
            .collect();
        assert_eq!(icons.len(), 1, "each command needs an image icon");
        let path = icons[0].image.path().unwrap();
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets")
            .join(path.path());
        let image = Image::from_buffer(
            &std::fs::read(file).unwrap(),
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::linear(),
            RenderAssetUsages::MAIN_WORLD,
        )
        .unwrap();
        let data = image.data.unwrap();
        let pixels = data.as_chunks::<4>().0;
        assert!(pixels.iter().any(|pixel| pixel[3] == 0));
        assert!(pixels.contains(&[255, 255, 255, 255]));
        count += 1;
    }
    assert_eq!(count, 3);
}
