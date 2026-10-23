mod camera;
mod crash;
#[cfg(target_arch = "wasm32")]
mod download;
mod font;
mod game;
mod hud;
mod input;
mod library;
mod map;
mod menu;
mod pause_menu;
mod preview;
mod replay_io;
mod session;
mod settings;
mod storage;
mod units;

use bevy::{log::LogPlugin, prelude::*};
use rts_assets::{AssetBundle, BundledAssetPlugin, DataPlugin};

use crate::{
    camera::RtsCameraPlugin, crash::CrashReport, font::GameFont, game::GamePlugin, hud::HudPlugin,
    input::SelectionPlugin, library::Library, map::MapPlugin, menu::MenuPlugin,
    pause_menu::PauseMenuPlugin, session::SessionPlugin, settings::SettingsPlugin,
    units::UnitsPlugin,
};

/// Starts the game.
///
/// # Panics
///
/// Panics if the asset pack is missing or corrupt.
pub fn run() {
    rts_assets::with_bundle(|bundle| {
        let mut app = build_app(bundle);
        // After the app is built, so Bevy's own hook stays in the chain.
        crash::install();
        app.run();
    });
}

fn build_app(bundle: AssetBundle) -> App {
    let mut app = App::new();
    app.insert_resource(CrashReport::load())
        .add_plugins(BundledAssetPlugin::new(bundle))
        .add_plugins(
            DefaultPlugins
                .set(LogPlugin {
                    custom_layer: |_| Some(crash::log_layer()),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "RTS".into(),
                        name: Some("cudzinec.rts".into()),
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins((
            DataPlugin,
            GamePlugin,
            MapPlugin,
            UnitsPlugin,
            RtsCameraPlugin,
            SelectionPlugin,
            HudPlugin,
            SessionPlugin,
            SettingsPlugin,
            MenuPlugin,
            PauseMenuPlugin,
        ))
        .init_resource::<Library>()
        .init_resource::<GameFont>();
    app
}
