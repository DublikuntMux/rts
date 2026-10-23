//! Local-only crash reports: a panic hook, a bounded log, and manual export.
//!
//! Nothing is sent anywhere. A panic writes `crash-report.txt` next to the saved games, holding
//! the version, platform, panic message, location, backtrace, and the end of `game.log`. The next
//! launch reads that report, the menu offers it, and the player exports or discards it. Browser
//! builds keep the report in `localStorage` and also record JavaScript errors, which never reach
//! a Rust panic hook.

use std::{
    fmt::{Debug, Write as _},
    panic::PanicHookInfo,
};

use bevy::{
    log::{
        BoxedLayer,
        tracing::{
            Event, Subscriber,
            field::{Field, Visit},
        },
        tracing_subscriber::{Layer, layer::Context},
    },
    prelude::Resource,
};

/// Report file name, in the data folder or browser storage.
const REPORT_NAME: &str = "crash-report.txt";
/// Log file name, in the data folder.
#[cfg(not(target_arch = "wasm32"))]
const LOG_NAME: &str = "game.log";
/// The log kept after a rotation, so one previous run stays readable.
#[cfg(not(target_arch = "wasm32"))]
const ROTATED_LOG_NAME: &str = "game.log.1";
/// The log rotates once it grows past this size, so it cannot fill the disk.
#[cfg(not(target_arch = "wasm32"))]
const LOG_LIMIT: u64 = 512 * 1024;
/// Log bytes a report keeps, counted from the end.
const REPORT_LOG_BYTES: usize = 48 * 1024;
/// Longest JavaScript error report kept, in bytes.
#[cfg(target_arch = "wasm32")]
const JS_REPORT_LIMIT: usize = 64 * 1024;
/// Log lines browser builds keep in memory, because they have no log file.
#[cfg(target_arch = "wasm32")]
const RING_LINES: usize = 500;
/// Longest log line kept, in bytes. Third-party crates log whole payloads.
const LINE_LIMIT: usize = 1000;

/// The data folder, resolved once. Browsers have browser storage instead.
#[cfg(not(target_arch = "wasm32"))]
static DATA_DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

/// The application data folder, or nothing in browsers.
#[cfg(not(target_arch = "wasm32"))]
fn data_dir() -> Option<&'static std::path::Path> {
    DATA_DIR.get_or_init(crate::storage::data_dir).as_deref()
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// A crash recorded by a previous run, waiting for the player.
#[derive(Resource, Default)]
pub struct CrashReport {
    /// Report text, kept until it is exported or discarded.
    text: Option<String>,
    /// Whether a copy is waiting in storage and must be removed once the report is settled.
    stored: bool,
}

impl CrashReport {
    /// Reads the report a previous run left behind.
    #[must_use]
    pub fn load() -> Self {
        let text = stored();
        Self {
            stored: text.is_some(),
            text,
        }
    }

    /// A report with known text, for tests.
    #[cfg(test)]
    pub(crate) fn for_test(text: &str) -> Self {
        Self {
            text: Some(text.to_owned()),
            stored: false,
        }
    }

    /// The report waiting for the player.
    #[must_use]
    pub fn pending(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Offers the report to the player and forgets the stored copy.
    ///
    /// Browsers download a text file; native builds copy it to a folder the player can reach.
    ///
    /// # Errors
    /// Returns a message for the player when the report cannot be delivered.
    pub fn export(&mut self) -> Result<String, String> {
        let text = self
            .text
            .as_deref()
            .ok_or("There is no crash report to export.")?;
        let message = export_text(text)?;
        self.text = None;
        self.stored = false;
        Ok(message)
    }

    /// Deletes the report without exporting it.
    pub fn discard(&mut self) {
        self.text = None;
        if std::mem::take(&mut self.stored) {
            discard_stored();
        }
    }
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// Installs the panic hook after Bevy's platform hook and before startup work, so startup
/// failures are captured and Bevy still prints the panic to the terminal or browser console.
pub fn install() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            store(&report_for(info));
            previous(info);
        }));
        #[cfg(target_arch = "wasm32")]
        browser::watch_errors();
    });
}

/// The report a panic leaves: a header, a backtrace, and the end of the log.
fn report_for(info: &PanicHookInfo<'_>) -> String {
    let location = info
        .location()
        .map_or_else(|| "unknown".to_owned(), ToString::to_string);
    let mut report = header(panic_message(info), &location);
    #[cfg(not(target_arch = "wasm32"))]
    {
        report.push_str("\nBacktrace:\n");
        report.push_str(&std::backtrace::Backtrace::force_capture().to_string());
        report.push('\n');
    }
    #[cfg(target_arch = "wasm32")]
    {
        report.push_str("\nBacktrace:\n");
        report.push_str(&browser::stack(js_sys::Error::new("Rust panic").into()));
        report.push('\n');
    }
    with_log(report)
}

/// Appends the same recent diagnostics to Rust panics and browser failures.
fn with_log(mut report: String) -> String {
    let log = recent_log(REPORT_LOG_BYTES);
    if !log.is_empty() {
        report.push_str("\n=== LOG (newest last) ===\n");
        report.push_str(&log);
        if !report.ends_with('\n') {
            report.push('\n');
        }
    }
    report
}

/// The first lines of a report.
fn header(message: &str, location: &str) -> String {
    format!(
        "RTS crash report\n\
         \n\
         Game: {} {}\n\
         Platform: {} {}\n\
         Build: {}\n\
         Time: {} UTC\n\
         Panic: {message}\n\
         Location: {location}\n",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        if cfg!(target_arch = "wasm32") {
            "web"
        } else {
            std::env::consts::OS
        },
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        clock(),
    )
}

/// The panic payload, which is text for `panic!` and usually for assertions.
fn panic_message<'a>(info: &'a PanicHookInfo<'_>) -> &'a str {
    info.payload()
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| info.payload().downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic")
}

/// Adds the game log to Bevy's subscriber. Pass this to `LogPlugin::custom_layer`.
pub fn log_layer() -> BoxedLayer {
    Box::new(LogLayer)
}

/// Appends formatted events to the platform log: a bounded file, or a ring buffer in browsers.
struct LogLayer;

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let metadata = event.metadata();
        let line = format!(
            "{} {:<5} {}: {}",
            clock(),
            metadata.level(),
            metadata.target(),
            fields.0
        );
        write_log(clip(&line, LINE_LIMIT));
    }
}

/// An event's fields as `message key=value …`.
#[derive(Default)]
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
}

// ---------------------------------------------------------------------------
// Log storage
// ---------------------------------------------------------------------------

fn write_log(line: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(root) = data_dir() {
        native::log(root, line);
    }
    #[cfg(target_arch = "wasm32")]
    sink::log(line);
}

/// The end of the log, at most `limit` bytes.
fn recent_log(limit: usize) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    return data_dir().map_or_else(String::new, |root| native::recent_log(root, limit));
    #[cfg(target_arch = "wasm32")]
    sink::recent(limit)
}

/// The end of `text`, at most `limit` bytes and never inside a character.
fn tail(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// The start of `text`, at most `limit` bytes and never inside a character.
fn clip(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// ---------------------------------------------------------------------------
// Report storage
// ---------------------------------------------------------------------------

fn stored() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    return native::read(data_dir()?);
    #[cfg(target_arch = "wasm32")]
    browser::read()
}

fn store(text: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(root) = data_dir() {
        native::write(root, text);
    }
    #[cfg(target_arch = "wasm32")]
    browser::write_panic(text);
}

fn discard_stored() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(root) = data_dir() {
        native::discard(root);
    }
    #[cfg(target_arch = "wasm32")]
    browser::delete();
}

fn export_text(text: &str) -> Result<String, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let root = data_dir().ok_or("Cannot locate the app data folder.")?;
        let folder = native::export_folder(root);
        let path = native::export(root, &folder, text)?;
        Ok(format!("Crash report saved to {}.", path.display()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::download::save(REPORT_NAME, "text/plain", text)?;
        browser::delete();
        Ok("Crash report download started.".into())
    }
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// The current UTC time, `YYYY-MM-DD HH:MM:SS`.
fn clock() -> String {
    format_utc(crate::storage::now_millis() / 1000)
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, from Unix seconds.
fn format_utc(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        remainder / 3600,
        remainder % 3600 / 60,
        remainder % 60
    )
}

/// The Gregorian date for a count of days since 1970-01-01, after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

// ---------------------------------------------------------------------------
// Platforms
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Seek, SeekFrom, Write},
        path::{Path, PathBuf},
        sync::{Mutex, OnceLock},
    };

    use super::{LOG_LIMIT, LOG_NAME, REPORT_NAME, ROTATED_LOG_NAME};

    /// The report file in `root`.
    pub fn report_path(root: &Path) -> PathBuf {
        root.join(REPORT_NAME)
    }

    pub fn read(root: &Path) -> Option<String> {
        fs::read_to_string(report_path(root)).ok()
    }

    pub fn write(root: &Path, text: &str) {
        if fs::create_dir_all(root).is_ok() {
            let _ = fs::write(report_path(root), text);
        }
    }

    pub fn discard(root: &Path) {
        let _ = fs::remove_file(report_path(root));
    }

    /// Copies `text` into `folder`, settles the pending report, and returns the written path.
    pub fn export(root: &Path, folder: &Path, text: &str) -> Result<PathBuf, String> {
        fs::create_dir_all(folder)
            .map_err(|error| format!("Cannot create {}: {error}", folder.display()))?;
        let path = report_path(folder);
        fs::write(&path, text)
            .map_err(|error| format!("Cannot write {}: {error}", path.display()))?;
        discard(root);
        Ok(path)
    }

    /// The player's downloads folder when it exists, an exports folder in app storage otherwise.
    /// Keep exports separate from the pending report so settling it survives a restart.
    pub fn export_folder(root: &Path) -> PathBuf {
        ["HOME", "USERPROFILE"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .map(|home| home.join("Downloads"))
            .find(|folder| folder.is_dir())
            .unwrap_or_else(|| root.join("exports"))
    }

    /// The open log file, kept for the life of the process.
    static LOG: OnceLock<Option<Mutex<Log>>> = OnceLock::new();

    pub(super) struct Log {
        file: File,
        bytes: u64,
    }

    /// Opens the log, rotating it first when the previous run filled it.
    pub(super) fn open(root: &Path) -> Option<Log> {
        fs::create_dir_all(root).ok()?;
        let path = root.join(LOG_NAME);
        if fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_LIMIT)
            && fs::rename(&path, root.join(ROTATED_LOG_NAME)).is_err()
        {
            // An obstructed rotation must not turn a bounded log into an unbounded one.
            fs::write(&path, []).ok()?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        let bytes = file.metadata().map_or(0, |meta| meta.len());
        Some(Log { file, bytes })
    }

    pub fn log(root: &Path, line: &str) {
        let Some(log) = LOG.get_or_init(|| open(root).map(Mutex::new)) else {
            return;
        };
        let Ok(mut log) = log.lock() else {
            return;
        };
        log.write(root, line);
    }

    impl Log {
        pub(super) fn write(&mut self, root: &Path, line: &str) {
            if self.bytes > LOG_LIMIT {
                match open(root) {
                    Some(next) => *self = next,
                    None => return,
                }
            }
            if self.file.write_all(line.as_bytes()).is_err() || self.file.write_all(b"\n").is_err()
            {
                return;
            }
            let _ = self.file.flush();
            let written = u64::try_from(line.len() + 1).unwrap_or(u64::MAX);
            self.bytes = self.bytes.saturating_add(written);
        }
    }

    /// The end of the log, at most `limit` bytes.
    pub fn recent_log(root: &Path, limit: usize) -> String {
        let Ok(mut file) = File::open(root.join(LOG_NAME)) else {
            return String::new();
        };
        let Ok(length) = file.metadata().map(|meta| meta.len()) else {
            return String::new();
        };
        let limit = u64::try_from(limit).unwrap_or(u64::MAX);
        // Include the preceding byte so a tail starting exactly at a line boundary keeps
        // that complete line instead of mistaking it for a partial one.
        let start = length.saturating_sub(limit.saturating_add(1));
        if file.seek(SeekFrom::Start(start)).is_err() {
            return String::new();
        }
        let mut bytes = Vec::new();
        if file
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .is_err()
        {
            return String::new();
        }
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        // A cut can land inside a line or a character; drop the preceding partial line.
        if start > 0
            && let Some((_, rest)) = text.split_once('\n')
        {
            text = rest.to_owned();
        }
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        super::tail(&text, limit).to_owned()
    }
}

#[cfg(target_arch = "wasm32")]
mod sink {
    use std::{
        collections::VecDeque,
        sync::{Mutex, OnceLock},
    };

    use super::RING_LINES;

    /// Recent log lines, because browsers have no writable log file.
    static LINES: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

    pub fn log(line: &str) {
        let Ok(mut lines) = LINES.get_or_init(|| Mutex::new(VecDeque::new())).lock() else {
            return;
        };
        if lines.len() == RING_LINES {
            lines.pop_front();
        }
        lines.push_back(line.to_owned());
    }

    pub fn recent(limit: usize) -> String {
        let Some(lines) = LINES.get() else {
            return String::new();
        };
        let Ok(lines) = lines.lock() else {
            return String::new();
        };
        let text = lines.iter().cloned().collect::<Vec<_>>().join("\n");
        super::tail(&text, limit).to_owned()
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::sync::atomic::{AtomicBool, Ordering};

    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    /// Storage key. Reports are plain text, not the compressed documents `storage` keeps.
    const KEY: &str = "rts/crash-report.txt";
    /// The browser also reports the trap or rejected promise caused by a Rust panic. That
    /// secondary event must not replace the panic message and Rust source location.
    static PANICKED: AtomicBool = AtomicBool::new(false);

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    pub fn read() -> Option<String> {
        storage()?.get_item(KEY).ok().flatten()
    }

    fn write(text: &str) {
        if let Some(storage) = storage() {
            let _ = storage.set_item(KEY, text);
        }
    }

    pub fn write_panic(text: &str) {
        PANICKED.store(true, Ordering::Relaxed);
        write(text);
    }

    fn record_error(message: &str, location: &str, stack: &str) {
        if PANICKED.load(Ordering::Relaxed) {
            return;
        }
        let mut report = super::header(message, location);
        report.push_str("\nBacktrace:\n");
        report.push_str(stack);
        report.push('\n');
        let report = super::with_log(report);
        write(super::clip(&report, super::JS_REPORT_LIMIT));
    }

    pub fn delete() {
        if let Some(storage) = storage() {
            let _ = storage.remove_item(KEY);
        }
    }

    /// Records browser errors, which never reach the Rust panic hook: a wasm trap ends the module
    /// without unwinding, and script errors happen outside it.
    pub fn watch_errors() {
        let Some(window) = web_sys::window() else {
            return;
        };
        let errors = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
            if let Ok(event) = event.dyn_into::<web_sys::ErrorEvent>() {
                record_error(
                    &format!("JavaScript error: {}", event.message()),
                    &format!("{}:{}:{}", event.filename(), event.lineno(), event.colno()),
                    &stack(event.error()),
                );
            }
        });
        let _ = window.add_event_listener_with_callback("error", errors.as_ref().unchecked_ref());
        errors.forget();

        let rejections = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
            if let Ok(event) = event.dyn_into::<web_sys::PromiseRejectionEvent>() {
                record_error(
                    "Unhandled promise rejection",
                    "unknown",
                    &stack(event.reason()),
                );
            }
        });
        let _ = window.add_event_listener_with_callback(
            "unhandledrejection",
            rejections.as_ref().unchecked_ref(),
        );
        rejections.forget();
    }

    /// A thrown value's stack trace, when it has one.
    pub(super) fn stack(value: JsValue) -> String {
        js_sys::Reflect::get(&value, &JsValue::from_str("stack"))
            .ok()
            .and_then(|stack| stack.as_string())
            .unwrap_or_else(|| format!("{value:?}"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rts-crash-{name}-{}-{}",
            std::process::id(),
            crate::storage::now_millis()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn reports_are_written_read_and_discarded() {
        let root = folder("report");
        assert!(native::read(&root).is_none());
        native::write(&root, "first");
        assert_eq!(native::read(&root).as_deref(), Some("first"));
        native::write(&root, "replaced");
        assert_eq!(native::read(&root).as_deref(), Some("replaced"));
        native::discard(&root);
        assert!(native::read(&root).is_none());
        native::discard(&root);
    }

    #[test]
    fn the_log_keeps_only_its_end() {
        let root = folder("log");
        let mut log = native::open(&root).unwrap();
        for index in 0..3 {
            log.write(&root, &format!("{index} line"));
        }
        let text = native::recent_log(&root, 1024);
        assert!(text.ends_with("2 line\n"), "{text}");
        let short = native::recent_log(&root, 8);
        assert!(short.len() <= 8, "{short}");
        assert!(!short.starts_with("0 line"), "{short}");
        assert_eq!(native::recent_log(&root, 7), "2 line\n");
    }

    #[test]
    fn exports_settle_the_pending_report_even_in_app_storage() {
        let root = folder("export");
        native::write(&root, "report body");
        let target = root.join("exports");
        let path = native::export(&root, &target, "report body").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "report body");
        assert_eq!(path, target.join(REPORT_NAME));
        assert!(native::read(&root).is_none());
    }

    #[test]
    fn failed_exports_keep_the_pending_report() {
        let root = folder("failed-export");
        native::write(&root, "report body");
        let target = root.join("exports");
        std::fs::write(&target, "a file obstructs the export folder").unwrap();
        assert!(native::export(&root, &target, "report body").is_err());
        assert_eq!(native::read(&root).as_deref(), Some("report body"));
    }

    #[test]
    fn the_log_stays_bounded_when_rotation_is_obstructed() {
        let root = folder("rotation");
        let mut log = native::open(&root).unwrap();
        std::fs::create_dir(root.join(ROTATED_LOG_NAME)).unwrap();
        let line = "a".repeat(LINE_LIMIT);
        for _ in 0..(LOG_LIMIT / (LINE_LIMIT as u64) + 2) {
            log.write(&root, &line);
        }
        let length = std::fs::metadata(root.join(LOG_NAME)).unwrap().len();
        assert!(length <= LOG_LIMIT + LINE_LIMIT as u64 + 1, "{length}");
        assert!(native::recent_log(&root, LINE_LIMIT + 1).contains(&line));
    }

    #[test]
    fn utc_dates_follow_the_calendar() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1_000_000_000), "2001-09-09 01:46:40");
        assert_eq!(format_utc(1_791_849_600), "2026-10-13 00:00:00");
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn headers_name_the_build_and_cause() {
        let text = header("index out of bounds", "src/world.rs:142");
        assert!(text.contains("RTS crash report"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains(std::env::consts::OS));
        assert!(text.contains("Panic: index out of bounds"));
        assert!(text.contains("Location: src/world.rs:142"));
        assert!(text.ends_with("Location: src/world.rs:142\n"));
    }

    #[test]
    fn tails_stay_on_character_boundaries() {
        let text = "aaaé";
        assert_eq!(tail(text, 100), text);
        assert_eq!(tail(text, 3), "aé");
        assert_eq!(tail(text, 2), "é");
        // One byte cannot hold a two-byte character.
        assert_eq!(tail(text, 1), "");
        assert_eq!(tail("", 0), "");
    }

    #[test]
    fn clips_stay_on_character_boundaries() {
        let text = "éaa";
        assert_eq!(clip(text, 100), text);
        assert_eq!(clip(text, 3), "éa");
        assert_eq!(clip(text, 2), "é");
        assert_eq!(clip(text, 1), "");
    }

    #[test]
    fn an_export_clears_the_pending_report() {
        let mut report = CrashReport::for_test("body");
        assert_eq!(report.pending(), Some("body"));
        report.discard();
        assert!(report.pending().is_none());
        assert!(report.export().is_err());
    }
}
