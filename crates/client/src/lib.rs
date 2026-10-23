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

use bevy::{app::PanicHandlerPlugin, log::LogPlugin, prelude::*};
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
    let mut app = App::new();
    app.add_plugins(PanicHandlerPlugin);
    // Keep Bevy's platform hook while also capturing asset-loading and plugin-build panics.
    crash::install();
    rts_assets::with_bundle(move |bundle| {
        build_app(&mut app, bundle);
        app.run();
    });
}

fn build_app(app: &mut App, bundle: AssetBundle) {
    app.insert_resource(CrashReport::load())
        .add_plugins(BundledAssetPlugin::new(bundle))
        .add_plugins(
            DefaultPlugins
                .build()
                .disable::<PanicHandlerPlugin>()
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
}
