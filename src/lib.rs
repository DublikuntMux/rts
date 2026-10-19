mod assets;
mod camera;
mod map;

use bevy::prelude::*;
use bevy_asset_packer::{AssetBundle, BundledAssetPlugin};

use crate::{assets::GameFont, camera::RtsCameraPlugin, map::MapPlugin};

const ASSET_PACK: &str = "assets.pack";

/// # Panics
///
/// Panics if the asset pack is missing or corrupt.
pub fn run() {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async {
        let bundle = AssetBundle::from_url(ASSET_PACK, None)
            .await
            .unwrap_or_else(|error| panic!("open {ASSET_PACK}: {error:#}"));
        build_app(bundle).run();
    });
    #[cfg(not(target_arch = "wasm32"))]
    build_app(open_bundle()).run();
}

/// Opens the pack shipped next to the game executable.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn open_bundle() -> AssetBundle {
    let path = std::env::current_exe()
        .expect("locate the game executable")
        .with_file_name(ASSET_PACK);
    AssetBundle::from_file(&path, None)
        .unwrap_or_else(|error| panic!("open {}: {error:#}", path.display()))
}

/// Maps the pack stored uncompressed in the APK's assets.
#[cfg(target_os = "android")]
fn open_bundle() -> AssetBundle {
    let app = bevy::android::ANDROID_APP
        .get()
        .expect("Android app is initialized");
    // Leaked so the mapped bytes stay valid for the rest of the process.
    let asset = Box::leak(Box::new(
        app.asset_manager()
            .open(c"assets.pack")
            .expect("assets.pack is packaged in the APK"),
    ));
    let bytes = asset.buffer().expect("map assets.pack");
    AssetBundle::from_bytes(bytes, None)
        .unwrap_or_else(|error| panic!("open {ASSET_PACK}: {error:#}"))
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
        .add_plugins((MapPlugin, RtsCameraPlugin))
        .init_resource::<GameFont>();
    app
}
