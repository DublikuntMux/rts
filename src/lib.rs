use bevy::{prelude::*, window::WindowResized};

pub fn run() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "RTS".into(),
                name: Some("cudzinec.rts".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, (setup_camera, setup_ui))
        .add_systems(Update, on_resize_system)
        .run();
}

#[derive(Component)]
struct ResolutionText;

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn setup_ui(mut commands: Commands) {
    commands
        .spawn(Node {
            width: percent(100),
            ..default()
        })
        .with_child((
            Text::new("Resolution"),
            TextFont {
                font_size: FontSize::Px(42.0),
                ..default()
            },
            ResolutionText,
        ));
}

fn on_resize_system(
    mut text: Single<&mut Text, With<ResolutionText>>,
    mut resize_reader: MessageReader<WindowResized>,
) {
    for e in resize_reader.read() {
        text.0 = format!("{:.1} x {:.1}", e.width, e.height);
    }
}
