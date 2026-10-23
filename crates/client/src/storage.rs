//! Persistent text documents: settings, saved games, and the replay library.
//!
//! Native builds keep one file per document under the platform data directory. Browsers keep
//! documents in `localStorage`, deflate-compressed because replays are large and the quota is
//! small. Keys are relative paths such as `settings.ron` or `replays/0190c2a1.meta.ron`.

use rts_net::MAX_REPLAY_BYTES;

/// Largest document read back from storage.
const MAX_DOCUMENT_BYTES: usize = MAX_REPLAY_BYTES;

/// Where documents live on this platform.
#[derive(Clone, Debug)]
pub struct Store {
    #[cfg(not(target_arch = "wasm32"))]
    root: Option<std::path::PathBuf>,
    /// Prepended to every `localStorage` key.
    #[cfg(target_arch = "wasm32")]
    prefix: &'static str,
}

impl Default for Store {
    fn default() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Self {
            root: native::data_dir(),
        };
        #[cfg(target_arch = "wasm32")]
        Self { prefix: "rts/" }
    }
}

impl Store {
    /// A store rooted at `root`, for tests.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub const fn at(root: std::path::PathBuf) -> Self {
        Self { root: Some(root) }
    }

    /// Reads a document. Returns `None` when it does not exist.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable or the document cannot be read.
    pub fn read(&self, key: &str) -> Result<Option<String>, String> {
        #[cfg(not(target_arch = "wasm32"))]
        return native::read(&self.root()?.join(key));
        #[cfg(target_arch = "wasm32")]
        browser::read(&format!("{}{key}", self.prefix))
    }

    /// Writes a document, replacing any previous version.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable or full.
    pub fn write(&self, key: &str, text: &str) -> Result<(), String> {
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err("The file is too large to store.".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        return native::write(&self.root()?.join(key), text);
        #[cfg(target_arch = "wasm32")]
        browser::write(&format!("{}{key}", self.prefix), text)
    }

    /// Deletes a document. Deleting a missing document succeeds.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable or the document cannot be removed.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        return native::delete(&self.root()?.join(key));
        #[cfg(target_arch = "wasm32")]
        browser::delete(&format!("{}{key}", self.prefix))
    }

    /// Names of the documents directly inside `folder` whose names end with `suffix`, with the
    /// suffix removed, in no particular order.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable.
    pub fn list(&self, folder: &str, suffix: &str) -> Result<Vec<String>, String> {
        #[cfg(not(target_arch = "wasm32"))]
        return native::list(&self.root()?.join(folder), suffix);
        #[cfg(target_arch = "wasm32")]
        browser::list(&format!("{}{folder}/", self.prefix), suffix)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn root(&self) -> Result<&std::path::Path, String> {
        self.root
            .as_deref()
            .ok_or_else(|| "Cannot locate app storage.".into())
    }
}

/// The platform data folder that stores documents, the log, and crash reports. Browsers keep
/// documents in `localStorage` instead, so there is no folder.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn data_dir() -> Option<std::path::PathBuf> {
    native::data_dir()
}

/// Milliseconds since the Unix epoch, for ordering saved documents.
#[must_use]
pub fn now_millis() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    return std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    #[cfg(target_arch = "wasm32")]
    {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Date.now() is a nonnegative integer number of milliseconds"
        )]
        let now = js_sys::Date::now().max(0.0) as u64;
        now
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{
        io::Read,
        path::{Path, PathBuf},
    };

    use super::MAX_DOCUMENT_BYTES;

    pub fn data_dir() -> Option<PathBuf> {
        #[cfg(target_os = "android")]
        return bevy::android::ANDROID_APP
            .get()
            .and_then(|app| app.internal_data_path());
        #[cfg(not(target_os = "android"))]
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|directory| directory.join("rts"))
    }

    pub fn read(path: &Path) -> Result<Option<String>, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Cannot open {}: {error}", path.display())),
        };
        let mut text = String::new();
        file.take((MAX_DOCUMENT_BYTES + 1) as u64)
            .read_to_string(&mut text)
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(format!("{} is too large.", path.display()));
        }
        Ok(Some(text))
    }

    pub fn write(path: &Path, text: &str) -> Result<(), String> {
        let parent = path.parent().ok_or("Cannot locate the storage folder.")?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        std::fs::write(&temporary, text)
            .map_err(|error| format!("Cannot write {}: {error}", path.display()))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("Cannot finish writing {}: {error}", path.display()))
    }

    pub fn delete(path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Cannot delete {}: {error}", path.display()))
            }
            _ => Ok(()),
        }
    }

    pub fn list(folder: &Path, suffix: &str) -> Result<Vec<String>, String> {
        let entries = match std::fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("Cannot list {}: {error}", folder.display())),
        };
        Ok(entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_suffix(suffix))
                    .map(str::to_owned)
            })
            .collect())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    fn storage() -> Result<web_sys::Storage, String> {
        web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .ok_or_else(|| "Browser storage is unavailable.".into())
    }

    pub fn read(key: &str) -> Result<Option<String>, String> {
        let Some(value) = storage()?
            .get_item(key)
            .map_err(|_| "Cannot read browser storage.")?
        else {
            return Ok(None);
        };
        super::codec::decode(&value).map(Some)
    }

    pub fn write(key: &str, text: &str) -> Result<(), String> {
        storage()?
            .set_item(key, &super::codec::encode(text))
            .map_err(|_| "Browser storage is full. Delete old saves or replays.".into())
    }

    pub fn delete(key: &str) -> Result<(), String> {
        storage()?
            .remove_item(key)
            .map_err(|_| "Cannot delete from browser storage.".into())
    }

    pub fn list(start: &str, suffix: &str) -> Result<Vec<String>, String> {
        let storage = storage()?;
        let count = storage
            .length()
            .map_err(|_| "Cannot list browser storage.")?;
        Ok((0..count)
            .filter_map(|index| storage.key(index).ok().flatten())
            .filter_map(|key| {
                key.strip_prefix(start)
                    .filter(|name| !name.contains('/'))
                    .and_then(|name| name.strip_suffix(suffix))
                    .map(str::to_owned)
            })
            .collect())
    }
}

/// Compact text encoding for browser storage: deflate, then one character per byte.
///
/// `localStorage` holds UTF-16 strings, so mapping each byte to a code point in
/// U+0100..=U+01FF costs one code unit per byte and never forms surrogates.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod codec {
    use super::MAX_DOCUMENT_BYTES;

    const BASE: u32 = 0x100;

    pub fn encode(text: &str) -> String {
        miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 6)
            .into_iter()
            .map(|byte| char::from_u32(BASE + u32::from(byte)).expect("valid code point"))
            .collect()
    }

    pub fn decode(value: &str) -> Result<String, String> {
        let bytes = value
            .chars()
            .map(|c| {
                u32::from(c)
                    .checked_sub(BASE)
                    .and_then(|byte| u8::try_from(byte).ok())
            })
            .collect::<Option<Vec<u8>>>()
            .ok_or("Stored data is corrupt.")?;
        let text = miniz_oxide::inflate::decompress_to_vec_with_limit(&bytes, MAX_DOCUMENT_BYTES)
            .map_err(|_| "Stored data is corrupt or too large.")?;
        String::from_utf8(text).map_err(|_| "Stored data is not text.".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn round_trips_and_rejects_foreign_text() {
            let text = "(turns: [(tick: 1, inputs: [])])\u{e9}".repeat(200);
            let encoded = encode(&text);
            assert!(encoded.chars().count() < text.len() / 4);
            assert_eq!(decode(&encoded).unwrap(), text);
            assert!(decode("plain text").is_err());
            assert!(decode(&encoded[..encoded.len() / 2]).is_err());
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub mod tests {
    use super::*;

    /// A store in a fresh temporary folder.
    pub fn temporary_store(name: &str) -> Store {
        let root = std::env::temp_dir().join(format!(
            "rts-store-{name}-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let _ = std::fs::remove_dir_all(&root);
        Store::at(root)
    }

    #[test]
    fn documents_are_written_listed_read_and_deleted() {
        let store = temporary_store("documents");
        assert_eq!(store.read("settings.ron").unwrap(), None);
        assert_eq!(
            store.list("saves", ".meta.ron").unwrap(),
            Vec::<String>::new()
        );
        store.write("saves/a.meta.ron", "first").unwrap();
        store.write("saves/b.meta.ron", "second").unwrap();
        store.write("saves/b.data.ron", "data").unwrap();
        store.write("saves/a.meta.ron", "replaced").unwrap();
        let mut names = store.list("saves", ".meta.ron").unwrap();
        names.sort();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(
            store.read("saves/a.meta.ron").unwrap().as_deref(),
            Some("replaced")
        );
        store.delete("saves/a.meta.ron").unwrap();
        store.delete("saves/missing.meta.ron").unwrap();
        assert_eq!(store.list("saves", ".meta.ron").unwrap(), ["b"]);
    }
}
