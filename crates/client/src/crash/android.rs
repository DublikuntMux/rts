//! Export a completed crash report to Android's shared Downloads collection.

use jni::{Env, JValue, JavaVM, errors::Error, jni_sig, jni_str, objects::JObject};

pub(super) fn save_to_downloads(name: &str, report: &[u8]) -> Result<(), String> {
    let app = bevy::android::ANDROID_APP
        .get()
        .ok_or("Android app storage is unavailable.")?;
    // SAFETY: AndroidApp supplies the running process's JVM.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread_for_scope(|env| {
        let activity = app.activity_as_ptr().cast();
        // SAFETY: AndroidApp owns this global reference and keeps it valid during the export.
        let activity = unsafe { env.as_cast_raw::<JObject>(&activity)? };
        write_download(env, &activity, name, report)
    })
    .map_err(|error| format!("Cannot save crash report in Downloads: {error}"))
}

fn write_download(
    env: &mut Env,
    activity: &JObject,
    name: &str,
    report: &[u8],
) -> Result<(), Error> {
    let resolver = env
        .call_method(
            activity,
            jni_str!("getContentResolver"),
            jni_sig!("()Landroid/content/ContentResolver;"),
            &[],
        )?
        .l()?;
    let downloads = env
        .get_static_field(
            jni_str!("android/provider/MediaStore$Downloads"),
            jni_str!("EXTERNAL_CONTENT_URI"),
            jni_sig!("Landroid/net/Uri;"),
        )?
        .l()?;
    let values = content_values(env, 1)?;
    for (key, value) in [("_display_name", name), ("mime_type", "text/plain")] {
        let key = env.new_string(key)?;
        let value = env.new_string(value)?;
        env.call_method(
            &values,
            jni_str!("put"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
            &[JValue::Object(&key), JValue::Object(&value)],
        )?;
    }

    let uri = env
        .call_method(
            &resolver,
            jni_str!("insert"),
            jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;"),
            &[JValue::Object(&downloads), JValue::Object(&values)],
        )?
        .l()?;
    if uri.is_null() {
        return Err(Error::NullPtr("Downloads entry"));
    }

    let outcome = write_entry(env, &resolver, &uri, report);
    if outcome.is_err() {
        // Release the pending row even when writing or publishing threw a Java exception.
        env.exception_clear();
        let null = JObject::null();
        let _ = env.call_method(
            &resolver,
            jni_str!("delete"),
            jni_sig!("(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I"),
            &[
                JValue::Object(&uri),
                JValue::Object(&null),
                JValue::Object(&null),
            ],
        );
        env.exception_clear();
    }
    outcome
}

fn write_entry(
    env: &mut Env,
    resolver: &JObject,
    uri: &JObject,
    report: &[u8],
) -> Result<(), Error> {
    let bytes = env.byte_array_from_slice(report)?;
    let stream = env
        .call_method(
            resolver,
            jni_str!("openOutputStream"),
            jni_sig!("(Landroid/net/Uri;)Ljava/io/OutputStream;"),
            &[JValue::Object(uri)],
        )?
        .l()?;
    let write = env.call_method(
        &stream,
        jni_str!("write"),
        jni_sig!("([B)V"),
        &[JValue::Object(&bytes)],
    );
    if write.is_err() {
        env.exception_clear();
    }
    let close = env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[]);
    write?;
    close?;
    let values = content_values(env, 0)?;
    let null = JObject::null();
    let updated = env
        .call_method(
            resolver,
            jni_str!("update"),
            jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I"),
            &[JValue::Object(uri), JValue::Object(&values), JValue::Object(&null), JValue::Object(&null)],
        )?
        .i()?;
    if updated != 1 {
        return Err(Error::NullPtr("published Downloads entry"));
    }
    Ok(())
}

fn content_values<'local>(env: &mut Env<'local>, pending: i32) -> Result<JObject<'local>, Error> {
    let values = env.new_object(
        jni_str!("android/content/ContentValues"),
        jni_sig!("()V"),
        &[],
    )?;
    let key = env.new_string("is_pending")?;
    let pending = env
        .call_static_method(
            jni_str!("java/lang/Integer"),
            jni_str!("valueOf"),
            jni_sig!("(I)Ljava/lang/Integer;"),
            &[JValue::Int(pending)],
        )?
        .l()?;
    env.call_method(
        &values,
        jni_str!("put"),
        jni_sig!("(Ljava/lang/String;Ljava/lang/Integer;)V"),
        &[JValue::Object(&key), JValue::Object(&pending)],
    )?;
    Ok(values)
}
