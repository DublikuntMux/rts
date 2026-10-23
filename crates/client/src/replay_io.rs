//! Replay file import and export through browser downloads and file pickers.
//!
//! Native builds keep replays in the library folder instead (see `storage`).

use std::sync::{
    Mutex,
    mpsc::{self, Receiver, SyncSender},
};

use bevy::prelude::Resource;
use rts_net::MAX_REPLAY_BYTES;

#[derive(Resource)]
pub struct ReplayIo {
    #[cfg_attr(
        not(any(test, target_arch = "wasm32")),
        expect(dead_code, reason = "native builds have no file picker")
    )]
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

    /// Downloads a replay file.
    ///
    /// # Errors
    /// Returns an error when the browser refuses or on platforms without file downloads.
    pub fn save(text: &str) -> Result<String, String> {
        if text.len() > MAX_REPLAY_BYTES {
            return Err("Replay is too large to export.".into());
        }
        #[cfg(target_arch = "wasm32")]
        return browser::save(text);
        #[cfg(not(target_arch = "wasm32"))]
        Err("Replay export is only available in the browser.".into())
    }

    /// Opens a file picker; the chosen file arrives through [`Self::take_loaded`].
    ///
    /// # Errors
    /// Returns an error when the picker cannot open or on platforms without one.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(clippy::unused_self, reason = "only browsers have a file picker")
    )]
    pub fn load(&self) -> Result<String, String> {
        #[cfg(target_arch = "wasm32")]
        return browser::load(self.sender.clone());
        #[cfg(not(target_arch = "wasm32"))]
        Err("Replay import is only available in the browser.".into())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::{JsCast, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use super::{MAX_REPLAY_BYTES, SyncSender};

    fn document() -> Result<web_sys::Document, String> {
        web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "Browser document is unavailable.".into())
    }

    pub fn save(text: &str) -> Result<String, String> {
        crate::download::save("match.replay.ron", "application/x-ron", text)?;
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
            #[expect(clippy::cast_precision_loss, reason = "the limit is far below 2^52")]
            let limit = MAX_REPLAY_BYTES as f64;
            if file.size() > limit {
                let _ = sender.try_send(Err("Replay file is too large.".into()));
                return;
            }
            spawn_local(async move {
                let result = JsFuture::from(file.text())
                    .await
                    .map_err(|_| "Cannot read replay file.".to_owned())
                    .and_then(|value| {
                        value
                            .as_string()
                            .ok_or_else(|| "Replay is not text.".into())
                    });
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
