//! Replay file import and export through browser downloads and file pickers.
//!
//! Native builds keep replays in the library folder instead (see `storage`).

use std::sync::mpsc::{self, Receiver, SyncSender};

use rts_net::MAX_REPLAY_BYTES;

pub struct ReplayIo {
    #[cfg_attr(
        not(any(test, target_arch = "wasm32")),
        expect(dead_code, reason = "native builds have no file picker")
    )]
    sender: SyncSender<Result<String, String>>,
    receiver: Receiver<Result<String, String>>,
    #[cfg(target_arch = "wasm32")]
    picker: Option<browser::Picker>,
}

impl Default for ReplayIo {
    fn default() -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        Self {
            sender,
            receiver,
            #[cfg(target_arch = "wasm32")]
            picker: None,
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
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            clippy::needless_pass_by_ref_mut,
            reason = "browsers release the owned picker"
        )
    )]
    pub fn take_loaded(&mut self) -> Option<Result<String, String>> {
        let result = self.receiver.try_recv().ok()?;
        #[cfg(target_arch = "wasm32")]
        {
            self.picker = None;
        }
        Some(result)
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
        expect(
            clippy::unused_self,
            clippy::needless_pass_by_ref_mut,
            reason = "only browsers retain a mutable file picker"
        )
    )]
    pub fn load(&mut self) -> Result<String, String> {
        #[cfg(target_arch = "wasm32")]
        {
            let picker = browser::Picker::new(self.sender.clone())?;
            self.picker.insert(picker).open();
            return Ok("Choose a replay file.".into());
        }
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

    /// The channel owner retains both the element and callback until the picker finishes.
    /// Dropping a canceled picker must release the callback even if `change` never fires.
    pub struct Picker {
        input: web_sys::HtmlInputElement,
        listener: Closure<dyn FnMut(web_sys::Event)>,
    }

    impl Picker {
        pub fn new(sender: SyncSender<Result<String, String>>) -> Result<Self, String> {
            let input = document()?
                .create_element("input")
                .map_err(|_| "Cannot open file picker.")?
                .dyn_into::<web_sys::HtmlInputElement>()
                .map_err(|_| "Cannot open file picker.")?;
            input.set_type("file");
            input.set_accept(".ron");
            let mut handled = false;
            let listener = Closure::new(move |event: web_sys::Event| {
                if handled {
                    return;
                }
                handled = true;
                let file = event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
                    .and_then(|input| input.files())
                    .and_then(|files| files.get(0));
                let Some(file) = file.filter(|_| event.type_() != "cancel") else {
                    let _ = sender.try_send(Err("Replay import canceled.".into()));
                    return;
                };
                #[expect(clippy::cast_precision_loss, reason = "the limit is far below 2^52")]
                let limit = MAX_REPLAY_BYTES as f64;
                if file.size() > limit {
                    let _ = sender.try_send(Err("Replay file is too large.".into()));
                    return;
                }
                let sender = sender.clone();
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
            let picker = Self { input, listener };
            for event in ["change", "cancel"] {
                picker
                    .input
                    .add_event_listener_with_callback(
                        event,
                        picker.listener.as_ref().unchecked_ref(),
                    )
                    .map_err(|_| "Cannot open file picker.")?;
            }
            Ok(picker)
        }

        pub fn open(&self) {
            self.input.click();
        }
    }

    impl Drop for Picker {
        fn drop(&mut self) {
            for event in ["change", "cancel"] {
                let _ = self.input.remove_event_listener_with_callback(
                    event,
                    self.listener.as_ref().unchecked_ref(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_completion_is_consumed_once_and_errors_allow_another_import() {
        let mut io = ReplayIo::default();
        assert!(io.take_loaded().is_none());
        io.sender
            .try_send(Err("Replay import canceled.".into()))
            .unwrap();
        assert_eq!(
            io.take_loaded(),
            Some(Err("Replay import canceled.".into()))
        );
        assert!(io.take_loaded().is_none());
        io.complete_for_test("replay".into());
        assert_eq!(io.take_loaded(), Some(Ok("replay".into())));
        assert!(io.take_loaded().is_none());
    }
}
