//! Server-hosted client assets (textures and translations) sent to connecting clients.

use std::{
    collections::BTreeMap,
    io::{self, Error, ErrorKind, Read},
    path::Path,
    sync::Arc,
};

use rts_net::assets::{
    AssetKind, MAX_ASSET_FILE_BYTES, MAX_ASSET_PACK_BYTES, MAX_SYNC_ASSETS, sha256,
    validate_asset_path, validate_locale_bytes,
};

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// One validated client asset kept in memory by the server for streaming on connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAsset {
    pub sha256: [u8; 32],
    pub bytes: Arc<[u8]>,
}

impl ServerAsset {
    /// Builds a validated asset from raw bytes for `path`.
    ///
    /// # Errors
    /// Returns an error if `path` or `bytes` fail asset validation.
    pub fn from_bytes(path: &str, bytes: Vec<u8>) -> io::Result<Self> {
        let kind = validate_asset_path(path)
            .map_err(|error| Error::new(ErrorKind::InvalidInput, error.to_string()))?;
        if bytes.is_empty() || bytes.len() > MAX_ASSET_FILE_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "asset `{path}` must be 1 to {MAX_ASSET_FILE_BYTES} bytes (got {})",
                    bytes.len()
                ),
            ));
        }
        validate_asset_content(path, kind, &bytes)?;
        let digest = sha256(&bytes);
        Ok(Self {
            sha256: digest,
            bytes: bytes.into(),
        })
    }
}

/// Loads all supported client assets (`textures/**/*.png` and `locales/*.lang.ron`) from `dir`.
///
/// # Errors
/// Returns an error if `dir` cannot be read, contains invalid texture/locale files, or exceeds
/// file count or size limits.
pub fn load_assets_dir(dir: &Path) -> io::Result<BTreeMap<String, ServerAsset>> {
    if !dir.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("assets path `{}` is not a directory", dir.display()),
        ));
    }
    let mut assets = BTreeMap::new();
    let mut total_bytes: usize = 0;
    for category in ["textures", "locales"] {
        let root = dir.join(category);
        if root.exists() {
            collect_dir(dir, &root, &mut assets, &mut total_bytes)?;
        }
    }
    Ok(assets)
}

fn collect_dir(
    base: &Path,
    current: &Path,
    assets: &mut BTreeMap<String, ServerAsset>,
    total_bytes: &mut usize,
) -> io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(current)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_dir(base, &path, assets, total_bytes)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(base)
            .map_err(|error| Error::new(ErrorKind::InvalidInput, error.to_string()))?;
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let Ok(_) = validate_asset_path(&rel) else {
            continue;
        };
        if assets.len() >= MAX_SYNC_ASSETS {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("assets directory exceeds {MAX_SYNC_ASSETS} files"),
            ));
        }
        let bytes = read_bounded(&path, MAX_ASSET_FILE_BYTES)?;
        *total_bytes = total_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "asset pack size overflow"))?;
        if *total_bytes > MAX_ASSET_PACK_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("assets directory exceeds {MAX_ASSET_PACK_BYTES} bytes"),
            ));
        }
        let asset = ServerAsset::from_bytes(&rel, bytes)?;
        assets.insert(rel, asset);
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("asset `{}` exceeds {limit} bytes", path.display()),
        ));
    }
    Ok(bytes)
}

fn validate_asset_content(path: &str, kind: AssetKind, bytes: &[u8]) -> io::Result<()> {
    match kind {
        AssetKind::Texture => {
            if bytes.len() < PNG_MAGIC.len() || bytes[..PNG_MAGIC.len()] != PNG_MAGIC {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("texture `{path}` is not a valid PNG file"),
                ));
            }
        }
        AssetKind::Locale => {
            validate_locale_bytes(bytes).map_err(|error| {
                Error::new(ErrorKind::InvalidInput, format!("locale `{path}`: {error}"))
            })?;
        }
    }
    Ok(())
}
