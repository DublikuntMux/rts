mod camera;
mod font;
mod game;
mod map;
mod units;

use bevy::prelude::*;
use rts_assets::{AssetBundle, BundledAssetPlugin, DataPlugin};

use crate::{
    camera::RtsCameraPlugin, font::GameFont, game::GamePlugin, map::MapPlugin, units::UnitsPlugin,
};

/// Starts the game.
///
/// # Panics
///
/// Panics if the asset pack is missing or corrupt.
pub fn run() {
    rts_assets::with_bundle(|bundle| {
        build_app(bundle).run();
    });
}

fn build_app(bundle: AssetBundle) -> App {
    let mut app = App::new();
    app.add_plugins(BundledAssetPlugin::new(bundle))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "RTS".into(),
                name: Some("cudzinec.rts".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            DataPlugin,
            GamePlugin,
            MapPlugin,
            UnitsPlugin,
            RtsCameraPlugin,
        ))
        .init_resource::<GameFont>();
    app
}
