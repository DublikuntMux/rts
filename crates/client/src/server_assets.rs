//! Client-side storage and disk cache for server-provided textures and translations.

use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::*,
};
use rts_net::assets::{hex_sha256, sha256};

/// Textures downloaded from the currently connected server.
///
/// Cleared whenever the client disconnects or begins a solo/replay match so server mods never
/// bleed into local play.
#[derive(Resource, Default)]
pub struct ServerAssets {
    textures: BTreeMap<String, Handle<Image>>,
    raw_textures: BTreeMap<String, Vec<u8>>,
}

impl ServerAssets {
    /// Decodes PNG `bytes` and registers them under relative `path` (e.g. `textures/units/hero.png`).
    ///
    /// # Errors
    /// Returns an error if `bytes` cannot be decoded as a valid PNG image.
    pub fn install_texture(
        &mut self,
        path: String,
        bytes: Vec<u8>,
        images: &mut Assets<Image>,
    ) -> Result<(), String> {
        let image = Image::from_buffer(
            &bytes,
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::Default,
            RenderAssetUsages::default(),
        )
        .map_err(|error| format!("invalid server texture `{path}`: {error}"))?;
        let handle = images.add(image);
        self.textures.insert(path.clone(), handle);
        self.raw_textures.insert(path, bytes);
        Ok(())
    }

    /// Returns the server-provided image handle for `path`, if any.
    #[must_use]
    pub fn image(&self, path: &str) -> Option<Handle<Image>> {
        self.textures.get(path).cloned()
    }

    /// Returns the server-provided image for `path` when present, falling back to the bundled asset.
    #[must_use]
    pub fn load_image(&self, assets: &AssetServer, path: &str) -> Handle<Image> {
        self.image(path)
            .unwrap_or_else(|| assets.load(path.to_owned()))
    }

    /// Decodes a server-provided terrain texture with CPU pixels retained (`MAIN_WORLD`) for
    /// terrain array assembly.
    #[must_use]
    pub fn terrain_image(&self, path: &str) -> Option<Result<Image, String>> {
        let bytes = self.raw_textures.get(path)?;
        Some(
            Image::from_buffer(
                bytes,
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::Default,
                RenderAssetUsages::MAIN_WORLD,
            )
            .map_err(|error| format!("invalid server terrain texture `{path}`: {error}")),
        )
    }

    /// Clears all server-provided textures.
    pub fn clear(&mut self) {
        self.textures.clear();
        self.raw_textures.clear();
    }

    /// Number of installed server textures.
    #[cfg(test)]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.textures.len()
    }
}

/// Helper for render systems whose `ServerAssets` resource is optional (e.g. standalone tests).
#[must_use]
pub fn load_image(
    server_assets: Option<&ServerAssets>,
    assets: &AssetServer,
    path: &str,
) -> Handle<Image> {
    server_assets.map_or_else(
        || assets.load(path.to_owned()),
        |synced| synced.load_image(assets, path),
    )
}

/// Content-addressed disk cache folder for server-synced assets.
#[must_use]
pub fn cache_dir() -> Option<PathBuf> {
    crate::storage::data_dir().map(|dir| dir.join("asset_cache"))
}

/// Reads a cached asset by SHA-256 digest and verifies its integrity before returning it.
#[must_use]
pub fn read_cached(
    cache_dir: Option<&Path>,
    expected_sha256: &[u8; 32],
    max_bytes: usize,
) -> Option<Vec<u8>> {
    let path = cache_dir?.join(hex_sha256(expected_sha256));
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || bytes.len() > max_bytes {
        return None;
    }
    (sha256(&bytes) == *expected_sha256).then_some(bytes)
}

/// Atomically stores verified asset bytes in the content-addressed disk cache.
pub fn write_cached(cache_dir: Option<&Path>, digest: &[u8; 32], bytes: &[u8]) {
    let Some(dir) = cache_dir else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let hex = hex_sha256(digest);
    let target = dir.join(&hex);
    if target.exists() {
        return;
    }
    let tmp = dir.join(format!("{hex}.tmp"));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PNG: &[u8] = include_bytes!("../../../assets/textures/terrain/wood.png");

    #[test]
    fn server_assets_decode_textures_and_clear_cleanly() {
        let mut images = Assets::<Image>::default();
        let mut server = ServerAssets::default();
        server
            .install_texture(
                "textures/units/custom.png".into(),
                SAMPLE_PNG.to_vec(),
                &mut images,
            )
            .unwrap();
        assert_eq!(server.len(), 1);
        let handle = server.image("textures/units/custom.png").unwrap();
        assert!(images.get(&handle).is_some());
        let terrain = server
            .terrain_image("textures/units/custom.png")
            .unwrap()
            .unwrap();
        assert!(terrain.data.is_some());

        assert!(
            server
                .install_texture(
                    "textures/units/bad.png".into(),
                    b"not a png".to_vec(),
                    &mut images
                )
                .is_err()
        );

        server.clear();
        assert_eq!(server.len(), 0);
        assert!(server.image("textures/units/custom.png").is_none());
    }

    #[test]
    fn disk_cache_roundtrips_and_rejects_corrupted_entries() {
        let dir = std::env::temp_dir().join(format!("rts-asset-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let digest = sha256(SAMPLE_PNG);
        assert!(read_cached(Some(&dir), &digest, 1024 * 1024).is_none());
        write_cached(Some(&dir), &digest, SAMPLE_PNG);
        assert_eq!(
            read_cached(Some(&dir), &digest, 1024 * 1024).as_deref(),
            Some(SAMPLE_PNG)
        );
        // Corrupt the cached file and verify hash check rejects it.
        std::fs::write(dir.join(hex_sha256(&digest)), b"corrupted").unwrap();
        assert!(read_cached(Some(&dir), &digest, 1024 * 1024).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
