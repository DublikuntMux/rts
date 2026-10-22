//! Game assets: opening the asset pack on each platform and loading game data files.
//!
//! `build.rs` packs the repository's `assets/` folder into `assets.pack` next to the build
//! output. Desktop reads it beside the executable (in `Contents/Resources` for macOS app
//! bundles), web fetches it, and Android maps it from the APK.

mod data;
mod graphics;

pub use bevy_asset_packer::{AssetBundle, BundledAssetPlugin};

pub use crate::data::{DataPlugin, MapAsset, RulesAsset};
pub use crate::graphics::{Graphics, SelectionStyle, TerrainDetail};

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

/// Opens the desktop pack, using the Resources directory inside macOS app bundles.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn open_bundle() -> AssetBundle {
    let executable = std::env::current_exe().expect("locate the game executable");
    #[cfg(target_os = "macos")]
    let path = macos_bundle_pack_path(&executable)
        .unwrap_or_else(|| executable.with_file_name(ASSET_PACK));
    #[cfg(not(target_os = "macos"))]
    let path = executable.with_file_name(ASSET_PACK);
    AssetBundle::from_file(&path, None)
        .unwrap_or_else(|error| panic!("open {}: {error:#}", path.display()))
}

#[cfg(any(target_os = "macos", test))]
fn macos_bundle_pack_path(executable: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| contents.join("Resources").join(ASSET_PACK))
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::macos_bundle_pack_path;

    #[test]
    fn macos_bundle_uses_resources_after_moving_or_renaming_the_app() {
        for app in [
            "/Applications/RTS.app",
            "/Users/player/My Games/Renamed RTS.app",
        ] {
            assert_eq!(
                macos_bundle_pack_path(&Path::new(app).join("Contents/MacOS/rts")),
                Some(Path::new(app).join("Contents/Resources/assets.pack")),
            );
        }
    }

    #[test]
    fn standalone_builds_are_not_app_bundles() {
        for executable in [
            "/work/rts/target/debug/rts",
            "/work/rts/target/aarch64-apple-darwin/release/rts",
            "/tmp/Contents/MacOS/rts",
            "/tmp/RTS.app/Other/MacOS/rts",
            "/tmp/RTS.app/Contents/Other/rts",
            "rts",
        ] {
            assert_eq!(macos_bundle_pack_path(Path::new(executable)), None);
        }
    }
}
