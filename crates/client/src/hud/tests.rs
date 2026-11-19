use bevy::{state::app::StatesPlugin, window::PrimaryWindow};
use rts_base::PlayerId;
use rts_net::MatchSetup;

use super::*;
use crate::{camera::RtsCamera, map::MapBounds, menu::Menu};

fn app() -> App {
    app_with_setup(MatchSetup {
        rules: include_str!("../../../../assets/data/core.rules.ron").into(),
        map: include_str!("../../../../assets/maps/crossroads.map.ron").into(),
        seed: 0,
        players: 4,
    })
}

fn app_with_setup(setup: MatchSetup) -> App {
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
        .init_resource::<Menu>()
        .add_plugins((HudPlugin, skirmish::SkirmishPlugin));
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

#[test]
fn desktop_skirmish_training_uses_the_existing_command_grid_and_reports_activity() {
    let mut app = app_with_setup(skirmish::test_setup());
    let headquarters = UnitId(0);
    app.world_mut().resource_mut::<Selection>().units = [headquarters].into();
    app.update();

    let world = app.world_mut();
    let (button, slot) = world
        .query::<(Entity, &Name, &Node, &ChildOf)>()
        .iter(world)
        .find_map(|(entity, name, node, parent)| {
            (name.as_str() == "Train worker" && node.display == Display::Flex)
                .then_some((entity, parent.parent()))
        })
        .unwrap();
    assert_eq!(world.get::<CommandSlot>(slot).unwrap().0, 4);
    world.trigger(Activate { entity: button });
    app.update();
    {
        let mut game = app.world_mut().resource_mut::<Match>();
        let commands = std::mem::take(&mut game.pending);
        assert_eq!(
            commands,
            [rts_base::Command::TrainWorker {
                units: vec![headquarters]
            }]
        );
        game.step_with(commands.into_iter().map(|command| (PlayerId(0), command)));
    }
    app.update();
    let world = app.world_mut();
    let mut labels = world.query::<(&HudText, &Text)>();
    assert!(
        labels
            .iter(world)
            .any(|(role, text)| matches!(role.0, TextRole::Detail) && text.0.contains("Training"))
    );
    assert!(
        labels
            .iter(world)
            .any(|(role, text)| matches!(role.0, TextRole::Hint)
                && text.0 == "Worker · 1s · 1 queued")
    );
}

#[test]
fn desktop_barracks_offer_scouts_in_the_next_command_slot() {
    let mut app = app_with_setup(skirmish::test_setup());
    let barracks = UnitId(4);
    app.world_mut().resource_mut::<Selection>().units = [barracks].into();
    app.update();

    let world = app.world_mut();
    let (button, slot) = world
        .query::<(Entity, &Name, &Node, &ChildOf)>()
        .iter(world)
        .find_map(|(entity, name, node, parent)| {
            (name.as_str() == "Train scout" && node.display == Display::Flex)
                .then_some((entity, parent.parent()))
        })
        .unwrap();
    assert_eq!(world.get::<CommandSlot>(slot).unwrap().0, 6);
    world.trigger(Activate { entity: button });
    app.update();
    let mut game = app.world_mut().resource_mut::<Match>();
    let commands = std::mem::take(&mut game.pending);
    assert_eq!(
        commands,
        [rts_base::Command::TrainScout {
            units: vec![barracks]
        }]
    );
    game.step_with(commands.into_iter().map(|command| (PlayerId(0), command)));
    assert_eq!(
        game.view.production(barracks).unwrap()[0].kind,
        rts_base::TrainingKind::Scout
    );
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
    assert_eq!(*app.world().resource::<Targeting>(), Targeting::None);
    let ids: Vec<_> = app
        .world()
        .resource::<Match>()
        .view
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
    let hash = app
        .world()
        .resource::<Match>()
        .local_sim()
        .unwrap()
        .state_hash();
    assert_eq!(
        app.world_mut()
            .query::<&RosterUnit>()
            .iter(app.world())
            .count(),
        2
    );
    order(&mut app, Order::Move);
    assert_eq!(*app.world().resource::<Targeting>(), Targeting::Move);
    order(&mut app, Order::Stop);
    assert_eq!(*app.world().resource::<Targeting>(), Targeting::None);
    assert_eq!(
        app.world().resource::<Match>().pending,
        vec![rts_base::Command::Stop { units: ids.clone() }]
    );
    assert_eq!(
        app.world()
            .resource::<Match>()
            .local_sim()
            .unwrap()
            .state_hash(),
        hash
    );
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

#[test]
fn unit_images_keep_their_aspect_ratio_in_square_frames() {
    let mut app = app();
    let id = app
        .world()
        .resource::<Match>()
        .view
        .units()
        .find(|(_, unit)| unit.owner == PlayerId(0))
        .unwrap()
        .0;
    app.world_mut().resource_mut::<Selection>().units = [id].into();
    app.update();
    let mut portraits = app
        .world_mut()
        .query_filtered::<&ImageNode, With<PortraitImage>>();
    let handle = portraits.single(app.world()).unwrap().image.clone();
    let tall = Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: 1,
            height: 4,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[255; 4],
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(handle.id(), tall)
        .unwrap();
    app.update();
    let mut images = app
        .world_mut()
        .query_filtered::<&Node, (With<UnitImage>, With<PortraitImage>)>();
    let node = images.single(app.world()).unwrap();
    assert_eq!(node.width, percent(25));
    assert_eq!(node.height, percent(100));
}

#[test]
fn minimap_markers_track_removal_and_recreated_hud_without_duplicates() {
    let mut app = app();
    let expected = app.world().resource::<Match>().view.units().count();
    let mut markers = app
        .world_mut()
        .query_filtered::<Entity, With<minimap::UnitMarker>>();
    assert_eq!(markers.iter(app.world()).count(), expected);
    let removed = app
        .world()
        .resource::<Match>()
        .view
        .units()
        .next()
        .unwrap()
        .0;
    app.world_mut().resource_mut::<Match>().remove_unit(removed);
    app.update();
    assert_eq!(markers.iter(app.world()).count(), expected - 1);
    app.update();
    assert_eq!(markers.iter(app.world()).count(), expected - 1);
    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::Menu);
    app.update();
    assert_eq!(markers.iter(app.world()).count(), 0);
    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::InGame);
    app.update();
    app.update();
    assert_eq!(markers.iter(app.world()).count(), expected - 1);
}

/// A 30-tile corridor whose far end starts unexplored.
const CORRIDOR: &str = r#"(
    name: "Corridor",
    players: [(start: (0.5, 0.5)), (start: (29.5, 1.5))],
    tiles: ["..............................", ".............................."],
    units: [
        (kind: "headquarters", owner: 0, pos: (0.5, 0.5)),
        (kind: "headquarters", owner: 1, pos: (29.5, 1.5)),
        (kind: "worker", owner: 0, pos: (12.5, 0.5)),
    ],
    skirmish: Some((deposits: [
        (kind: Wood, pos: (13.5, 0.5), remaining: 11),
        (kind: Gold, pos: (1.5, 1.5), remaining: 13),
    ])),
)"#;

#[test]
fn minimap_keeps_its_pixels_and_redraws_only_for_new_terrain_knowledge() {
    let mut setup = skirmish::test_setup();
    setup.map = CORRIDOR.into();
    let mut app = app_with_setup(setup);
    let handle = app
        .world_mut()
        .query_filtered::<&ImageNode, With<minimap::MapImage>>()
        .single(app.world())
        .unwrap()
        .image
        .clone();
    let pixels = |app: &mut App| {
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&handle)
            .expect("the minimap image stays in the main world")
            .data
            .clone()
            .expect("CPU pixels are retained")
    };
    let len = pixels(&mut app).len();
    let mark = |app: &mut App| {
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&handle)
            .unwrap()
            .data = Some(vec![0; len]);
    };

    mark(&mut app);
    let revision = app.world().resource::<Match>().view.terrain_revision();
    app.world_mut().resource_mut::<Match>().step();
    app.update();
    assert_eq!(
        app.world().resource::<Match>().view.terrain_revision(),
        revision
    );
    assert_eq!(
        pixels(&mut app),
        vec![0; len],
        "quiet ticks skip the redraw"
    );

    let mut game = app.world_mut().resource_mut::<Match>();
    let worker = game
        .view
        .units()
        .find(|(_, unit)| unit.owner == PlayerId(0) && game.view.rules().unit(unit.kind).speed > 0)
        .unwrap()
        .0;
    game.step_with([(
        PlayerId(0),
        rts_base::Command::Move {
            units: vec![worker],
            target: rts_base::IVec2::new(
                25 * rts_base::TILE + rts_base::TILE / 2,
                rts_base::TILE / 2,
            ),
        },
    )]);
    for _ in 0..200 {
        if game.view.terrain_revision() != revision {
            break;
        }
        game.step();
    }
    assert_ne!(
        game.view.terrain_revision(),
        revision,
        "the worker discovers terrain"
    );
    app.update();
    assert_ne!(
        pixels(&mut app),
        vec![0; len],
        "discovery redraws the minimap"
    );
}
