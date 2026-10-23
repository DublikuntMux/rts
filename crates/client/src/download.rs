//! File downloads through the browser.
//!
//! Only the browser can start a download; native builds write documents to the data folder
//! instead, so this module is built for `wasm32` only.

use wasm_bindgen::{JsCast, JsValue};

/// Offers `text` to the player as a downloaded file.
///
/// # Errors
/// Returns a message for the player when the browser refuses.
pub fn save(filename: &str, mime: &str, text: &str) -> Result<(), String> {
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(text));
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)
        .map_err(|_| "Cannot prepare the download.")?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|_| "Cannot prepare the download.")?;
    let link = web_sys::window()
        .and_then(|window| window.document())
        .ok_or("Browser document is unavailable.")?
        .create_element("a")
        .map_err(|_| "Cannot prepare the download.")?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "Cannot prepare the download.")?;
    link.set_href(&url);
    link.set_download(filename);
    link.click();
    web_sys::Url::revoke_object_url(&url).map_err(|_| "Cannot finish the download.")?;
    Ok(())
}
