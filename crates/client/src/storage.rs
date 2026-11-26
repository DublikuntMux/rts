//! Persistent text documents: settings, saved games, and the replay library.
//!
//! Documents are stored as files under the platform data directory. Keys are relative paths such
//! as `settings.ron` or `replays/0190c2a1.meta.ron`.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use rts_net::MAX_REPLAY_BYTES;

/// Largest document read back from storage.
const MAX_DOCUMENT_BYTES: usize = MAX_REPLAY_BYTES;

/// Where documents live on this platform.
#[derive(Clone, Debug)]
pub struct Store {
    root: Option<PathBuf>,
}

impl Default for Store {
    fn default() -> Self {
        Self { root: data_dir() }
    }
}

impl Store {
    /// A store rooted at `root`, for tests.
    #[cfg(test)]
    pub const fn at(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }

    /// Reads a document. Returns `None` when it does not exist.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable or the document cannot be read.
    pub fn read(&self, key: &str) -> Result<Option<String>, String> {
        read_file(&self.root()?.join(key))
    }

    /// Writes a document, replacing any previous version.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable or full.
    pub fn write(&self, key: &str, text: &str) -> Result<(), String> {
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err("The file is too large to store.".into());
        }
        write_file(&self.root()?.join(key), text)
    }

    /// Deletes a document. Deleting a missing document succeeds.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable or the document cannot be removed.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        delete_file(&self.root()?.join(key))
    }

    /// Names of the documents directly inside `folder` whose names end with `suffix`, with the
    /// suffix removed, in no particular order.
    ///
    /// # Errors
    /// Returns an error when storage is unavailable.
    pub fn list(&self, folder: &str, suffix: &str) -> Result<Vec<String>, String> {
        list_dir(&self.root()?.join(folder), suffix)
    }

    fn root(&self) -> Result<&Path, String> {
        self.root
            .as_deref()
            .ok_or_else(|| "Cannot locate app storage.".into())
    }
}

/// The platform data folder that stores documents, the log, and crash reports.
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    return bevy::android::ANDROID_APP
        .get()
        .and_then(bevy::android::android_activity::AndroidApp::internal_data_path);
    #[cfg(not(target_os = "android"))]
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|directory| directory.join("rts"))
}

/// Milliseconds since the Unix epoch, for ordering saved documents.
#[must_use]
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn read_file(path: &Path) -> Result<Option<String>, String> {
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

fn write_file(path: &Path, text: &str) -> Result<(), String> {
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

fn delete_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("Cannot delete {}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

fn list_dir(folder: &Path, suffix: &str) -> Result<Vec<String>, String> {
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

#[cfg(test)]
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
