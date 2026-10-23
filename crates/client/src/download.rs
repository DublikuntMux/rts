//! File downloads through the browser.
//!
//! Only the browser can start a download; native builds write documents to the data folder
//! instead, so this module is built for `wasm32` only.

use wasm_bindgen::{JsCast, JsValue, closure::Closure};

/// Offers `text` to the player as a downloaded file.
///
/// # Errors
/// Returns a message for the player when the browser refuses.
pub fn save(filename: &str, mime: &str, text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("Browser window is unavailable.")?;
    let link = window
        .document()
        .ok_or("Browser document is unavailable.")?
        .create_element("a")
        .map_err(|_| "Cannot prepare the download.")?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "Cannot prepare the download.")?;
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(text));
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)
        .map_err(|_| "Cannot prepare the download.")?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|_| "Cannot prepare the download.")?;
    link.set_href(&url);
    link.set_download(filename);

    // The browser consumes the URL asynchronously after the click. Revoking it immediately
    // can cancel the download, so give it time to start before releasing the blob.
    let revoke_url = url.clone();
    let revoke = Closure::once(move || {
        let _ = web_sys::Url::revoke_object_url(&revoke_url);
    });
    if window
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            revoke.as_ref().unchecked_ref(),
            30_000,
        )
        .is_err()
    {
        let _ = web_sys::Url::revoke_object_url(&url);
        return Err("Cannot prepare the download.".into());
    }
    revoke.forget();
    link.click();
    Ok(())
}
