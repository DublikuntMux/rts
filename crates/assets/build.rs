use std::{
    env,
    path::{Path, PathBuf},
};

use bevy_asset_packer::{AssetBundler, AssetBundlingOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The game's assets live at the repository root, shared by every crate.
    let assets_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../assets");
    println!("cargo:rerun-if-changed={}", assets_dir.display());
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let profile_dir = out_dir
        .ancestors()
        .find(|dir| dir.file_name() == Some("build".as_ref()))
        .and_then(Path::parent)
        .ok_or("unexpected OUT_DIR layout")?;
    AssetBundler::new(
        assets_dir,
        profile_dir.join("assets.pack"),
        AssetBundlingOptions::default(),
    )
    .build()?;
    Ok(())
}
