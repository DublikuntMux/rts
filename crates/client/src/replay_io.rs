//! Persistent replay files on native platforms; upload/download through browser APIs on web.

use std::sync::{
    Mutex,
    mpsc::{self, Receiver, SyncSender},
};

use bevy::prelude::Resource;
use rts_net::MAX_REPLAY_BYTES;

#[derive(Resource)]
pub struct ReplayIo {
    sender: SyncSender<Result<String, String>>,
    receiver: Mutex<Receiver<Result<String, String>>>,
}

impl Default for ReplayIo {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        Self {
            sender,
            receiver: Mutex::new(receiver),
        }
    }
}

impl ReplayIo {
    #[cfg(test)]
    pub(crate) fn complete_for_test(&self, text: String) {
        self.sender
            .try_send(Ok(text))
            .expect("test replay channel is empty");
    }
    pub fn take_loaded(&mut self) -> Option<Result<String, String>> {
        self.receiver.get_mut().ok()?.try_recv().ok()
    }

    pub fn save(text: &str) -> Result<String, String> {
        if text.len() > MAX_REPLAY_BYTES {
            return Err("Replay is too large to save.".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = replay_path()?;
            std::fs::create_dir_all(path.parent().ok_or("Cannot locate replay directory.")?)
                .map_err(|error| format!("Cannot create replay directory: {error}"))?;
            let temporary = path.with_extension("ron.tmp");
            std::fs::write(&temporary, text)
                .map_err(|error| format!("Cannot write replay: {error}"))?;
            std::fs::rename(&temporary, &path)
                .map_err(|error| format!("Cannot finish saving replay: {error}"))?;
            Ok(format!("Replay saved to {}", path.display()))
        }
        #[cfg(target_arch = "wasm32")]
        browser::save(text)
    }

    pub fn load(&self) -> Result<String, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::io::Read;
            let path = replay_path()?;
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
            let mut text = String::new();
            file.take((MAX_REPLAY_BYTES + 1) as u64)
                .read_to_string(&mut text)
                .map_err(|error| format!("Cannot read replay: {error}"))?;
            if text.len() > MAX_REPLAY_BYTES {
                return Err("Replay file is too large.".into());
            }
            self.sender
                .try_send(Ok(text))
                .map_err(|_| "A replay is already loading.".to_owned())?;
            Ok("Loading saved replay…".into())
        }
        #[cfg(target_arch = "wasm32")]
        browser::load(self.sender.clone())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn replay_path() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "android")]
    let directory = bevy::android::ANDROID_APP
        .get()
        .and_then(|app| app.internal_data_path())
        .ok_or("Cannot locate app storage.")?;
    #[cfg(not(target_os = "android"))]
    let directory = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
        })
        .ok_or("Cannot locate replay storage.")?
        .join("rts");
    Ok(directory.join("replays/latest.replay.ron"))
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use super::*;

    fn document() -> Result<web_sys::Document, String> {
        web_sys::window()
            .and_then(|window| window.document())
            .ok_or("Browser document is unavailable.".into())
    }

    pub fn save(text: &str) -> Result<String, String> {
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("application/x-ron");
        let parts = js_sys::Array::new();
        parts.push(&JsValue::from_str(text));
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)
            .map_err(|_| "Cannot prepare replay download.")?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)
            .map_err(|_| "Cannot prepare replay download.")?;
        let link = document()?
            .create_element("a")
            .map_err(|_| "Cannot prepare replay download.")?
            .dyn_into::<web_sys::HtmlAnchorElement>()
            .map_err(|_| "Cannot prepare replay download.")?;
        link.set_href(&url);
        link.set_download("match.replay.ron");
        link.click();
        web_sys::Url::revoke_object_url(&url).map_err(|_| "Cannot finish replay download.")?;
        Ok("Replay download started.".into())
    }

    pub fn load(sender: SyncSender<Result<String, String>>) -> Result<String, String> {
        let input = document()?
            .create_element("input")
            .map_err(|_| "Cannot open file picker.")?
            .dyn_into::<web_sys::HtmlInputElement>()
            .map_err(|_| "Cannot open file picker.")?;
        input.set_type("file");
        input.set_accept(".ron");
        let picker = input.clone();
        let handler = Closure::once_into_js(move |_event: web_sys::Event| {
            let Some(file) = picker.files().and_then(|files| files.get(0)) else {
                return;
            };
            if file.size() > MAX_REPLAY_BYTES as f64 {
                let _ = sender.try_send(Err("Replay file is too large.".into()));
                return;
            }
            spawn_local(async move {
                let result = JsFuture::from(file.text())
                    .await
                    .map_err(|_| "Cannot read replay file.".to_owned())
                    .and_then(|value| value.as_string().ok_or("Replay is not text.".into()));
                let _ = sender.try_send(result);
            });
        });
        input
            .add_event_listener_with_callback("change", handler.unchecked_ref())
            .map_err(|_| "Cannot open file picker.")?;
        input.click();
        Ok("Choose a replay file.".into())
    }
}
