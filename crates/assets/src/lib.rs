//! Game assets: opening the asset pack on each platform and loading game data files.
//!
//! `build.rs` packs the repository's `assets/` folder into `assets.pack` next to the build
//! output. Desktop reads it beside the executable, web fetches it, and Android maps it from the
//! APK.

mod data;

pub use bevy_asset_packer::{AssetBundle, BundledAssetPlugin};

pub use crate::data::{DataPlugin, MapAsset, RulesAsset};

const ASSET_PACK: &str = "assets.pack";

/// Opens this platform's asset pack and passes it to `start`. On the web this happens after the
/// pack is downloaded.
///
/// # Panics
///
/// Panics if the asset pack is missing or corrupt.
pub fn with_bundle(start: impl FnOnce(AssetBundle) + 'static) {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async {
        let bundle = AssetBundle::from_url(ASSET_PACK, None)
            .await
            .unwrap_or_else(|error| panic!("open {ASSET_PACK}: {error:#}"));
        start(bundle);
    });
    #[cfg(not(target_arch = "wasm32"))]
    start(open_bundle());
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
