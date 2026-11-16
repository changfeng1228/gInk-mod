//! Bounded, rolling application log.
//!
//! The log lives under the new app's own data directory and is deliberately
//! small: a fixed number of fixed-size files. It is a diagnostics channel only;
//! callers pass short lifecycle/error strings, never screenshot pixels or other
//! user content. Once the sink is open, write failures are swallowed on
//! purpose: failing to write a log line must never take the application down.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::settings_store::resolve_data_directory;

/// File name of the crash/startup diagnostics log under the data directory.
pub const CRASH_LOG_FILE_NAME: &str = "plnk-crash.log";
/// Upper bound of the crash log. The panic hook can run at any point, so the
/// file is truncated to its newest bytes on every write instead of rotating.
pub const CRASH_LOG_MAX_BYTES: usize = 64 * 1024;

/// Smallest file size accepted by [`LogPolicy`]. A line is truncated to fit one
/// empty file, so the bound (`max_file_bytes * max_files`) always holds.
const MIN_FILE_BYTES: u64 = 64;

/// Size policy for the rolling log. `max_files` includes the current file, so
/// the total on disk never exceeds `max_file_bytes * max_files` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogPolicy {
    pub max_file_bytes: u64,
    pub max_files: usize,
}

impl Default for LogPolicy {
    /// 3 files of 256 KiB: at most 768 KiB on disk.
    fn default() -> Self {
        Self {
            max_file_bytes: 256 * 1024,
            max_files: 3,
        }
    }
}

impl LogPolicy {
    fn normalized(self) -> Self {
        Self {
            max_file_bytes: self.max_file_bytes.max(MIN_FILE_BYTES),
            max_files: self.max_files.max(1),
        }
    }
}

#[derive(Debug)]
pub enum LogError {
    /// The log file or its directory could not be opened for appending.
    Open { path: PathBuf, source: io::Error },
}

impl std::fmt::Display for LogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { path, source } => {
                write!(
                    formatter,
                    "failed to open log file {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for LogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } => Some(source),
        }
    }
}

/// A cloneable handle to the process log. Every clone writes through the same
/// mutex, so the UI thread and the bounded file-IO worker can share it.
#[derive(Clone)]
pub struct LogSink {
    inner: Arc<Mutex<LogState>>,
}

impl LogSink {
    /// Opens the log at `path` with the default size policy, rotating an
    /// already-oversized file before appending.
    pub fn open(path: PathBuf) -> Result<Self, LogError> {
        Self::with_policy(path, LogPolicy::default())
    }

    /// Opens the log with an explicit policy. Useful for tests and for a future
    /// options screen; the policy is normalized so the size bound is real.
    pub fn with_policy(path: PathBuf, policy: LogPolicy) -> Result<Self, LogError> {
        let state = LogState::open(path, policy.normalized())?;
        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
        })
    }

    pub fn info(&self, message: &str) {
        self.write("INFO", message);
    }

    pub fn warn(&self, message: &str) {
        self.write("WARN", message);
    }

    pub fn error(&self, message: &str) {
        self.write("ERROR", message);
    }

    fn write(&self, level: &str, message: &str) {
        self.lock().write(level, message);
    }

    fn lock(&self) -> MutexGuard<'_, LogState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct LogState {
    path: PathBuf,
    policy: LogPolicy,
    file: Option<File>,
    written: u64,
}

impl LogState {
    fn open(path: PathBuf, policy: LogPolicy) -> Result<Self, LogError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| LogError::Open {
                path: path.clone(),
                source,
            })?;
        }
        let existing = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        if existing >= policy.max_file_bytes {
            rotate_files(&path, policy.max_files).map_err(|source| LogError::Open {
                path: path.clone(),
                source,
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| LogError::Open {
                path: path.clone(),
                source,
            })?;
        let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            path,
            policy,
            file: Some(file),
            written,
        })
    }

    fn write(&mut self, level: &str, message: &str) {
        let mut line = format!(
            "{} {level} {}",
            timestamp_now(),
            message.replace(['\r', '\n'], " ")
        );
        // Leave room for the trailing newline; truncate on a UTF-8 boundary.
        truncate_utf8(
            &mut line,
            self.policy.max_file_bytes.saturating_sub(1) as usize,
        );
        line.push('\n');
        let line_bytes = line.len() as u64;

        if self.written > 0 && self.written + line_bytes > self.policy.max_file_bytes {
            // If rotation fails (for example a rolled log file is locked by
            // another process), keep appending to the existing file instead of
            // going silent. The size bound is best-effort on an IO error.
            let _ = self.rotate();
        }
        if let Some(file) = self.file.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            self.written += line_bytes;
        }
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let result = rotate_files(&self.path, self.policy.max_files).and_then(|()| {
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
            self.written = 0;
            Ok(())
        });
        if result.is_err() {
            // Reopen the existing log so later lines are not dropped. Appending
            // past the configured bound is preferable to losing all diagnostics
            // for the rest of the run.
            self.reopen_after_failed_rotation();
        }
        result
    }

    fn reopen_after_failed_rotation(&mut self) {
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                self.written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                self.file = Some(file);
            }
            Err(_) => self.file = None,
        }
    }
}

/// Current file is `<stem>.<ext>`; rolled files are `<stem>.1.<ext>`,
/// `<stem>.2.<ext>`, ... with 1 being the newest rolled file.
fn rotate_files(path: &Path, max_files: usize) -> io::Result<()> {
    if max_files <= 1 {
        return remove_if_exists(path);
    }
    remove_if_exists(&rolled_path(path, max_files - 1))?;
    for index in (1..max_files - 1).rev() {
        rename_if_exists(&rolled_path(path, index), &rolled_path(path, index + 1))?;
    }
    rename_if_exists(path, &rolled_path(path, 1))
}

fn rolled_path(path: &Path, index: usize) -> PathBuf {
    let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or("plnk");
    let name = match path.extension().and_then(OsStr::to_str) {
        Some(extension) => format!("{stem}.{index}.{extension}"),
        None => format!("{stem}.{index}"),
    };
    path.with_file_name(name)
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn truncate_utf8(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

/// Installs the early crash hook used by `main`.
///
/// The release binary is a GUI-subsystem executable, so a panic message on
/// stderr would be invisible. The hook appends one timestamped line with the
/// panic text and source location to [`CRASH_LOG_FILE_NAME`] under the same
/// data directory as the settings and the normal log. It is best effort: any
/// failure (read-only directory, locked file, allocation) is ignored, and the
/// process then terminates with Rust's normal panic behaviour.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "<unknown>".to_owned());
        let message = panic_payload_message(info.payload());
        let line = format!("{} PANIC at {location}: {message}", timestamp_now());
        // A development console still sees the conventional panic text; the
        // crash log is what survives in a double-clicked GUI build. `writeln!`
        // is used instead of `eprintln!` so a closed/invalid stderr handle can
        // never panic inside the hook.
        let _ = writeln!(io::stderr(), "plnk: {line}");
        if let Some(directory) = resolve_data_directory() {
            let _ = append_crash_line(&directory, &line);
        }
    }));
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

/// Records that this process exited because another plnk instance already owns
/// the singleton.
///
/// A second instance never opens the first instance's normal log, so this
/// writes to the crash log instead. It is best effort and must never keep
/// startup alive or fail it when the file is locked.
pub fn log_second_instance_exit() {
    if let Some(directory) = resolve_data_directory() {
        let line = format!(
            "{} INFO second instance (pid {}) exited; another instance is running",
            timestamp_now(),
            std::process::id()
        );
        let _ = append_crash_line(&directory, &line);
    }
}

/// Appends one diagnostics line to `<data_directory>/plnk-crash.log`.
///
/// This log is independent from [`LogSink`] on purpose: the panic hook may run
/// before the main log exists, and a second instance never opens the first
/// instance's log. The file is bounded to the newest [`CRASH_LOG_MAX_BYTES`]
/// bytes, so a crash loop cannot grow it without limit.
pub fn append_crash_line(data_directory: &Path, line: &str) -> io::Result<()> {
    append_crash_line_with_limit(data_directory, line, CRASH_LOG_MAX_BYTES)
}

fn append_crash_line_with_limit(
    data_directory: &Path,
    line: &str,
    max_bytes: usize,
) -> io::Result<()> {
    let max_bytes = max_bytes.max(1);
    fs::create_dir_all(data_directory)?;
    let path = data_directory.join(CRASH_LOG_FILE_NAME);

    // Keep the new line even when it alone exceeds the bound; truncate its
    // tail so the timestamp and the panic location at the front stay visible.
    let mut line = line.replace(['\r', '\n'], " ").into_bytes();
    if line.len() + 1 > max_bytes {
        truncate_bytes(&mut line, max_bytes - 1);
    }
    line.push(b'\n');

    let mut contents = match fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let keep = max_bytes - line.len();
    let start = tail_start_after_line_break(&contents, keep);
    contents.drain(..start);
    contents.extend_from_slice(&line);
    // One truncating write keeps the file bounded even if an earlier crash
    // left it oversized; by construction it can never exceed the limit.
    fs::write(&path, contents)
}

/// Start index of the newest `max_bytes` bytes, preferring a line boundary.
/// A partial multi-byte character is skipped when no line break is available.
fn tail_start_after_line_break(bytes: &[u8], max_bytes: usize) -> usize {
    if bytes.len() <= max_bytes {
        return 0;
    }
    let mut start = bytes.len() - max_bytes;
    if let Some(offset) = bytes[start..].iter().position(|byte| *byte == b'\n') {
        start += offset + 1;
    } else {
        while start < bytes.len() && (bytes[start] & 0xC0) == 0x80 {
            start += 1;
        }
    }
    start
}

/// Truncates `bytes` on a UTF-8 character boundary.
fn truncate_bytes(bytes: &mut Vec<u8>, max_bytes: usize) {
    if bytes.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && (bytes[end] & 0xC0) == 0x80 {
        end -= 1;
    }
    bytes.truncate(end);
}

fn timestamp_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format_utc(seconds)
}

/// Compact UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`), computed without a date
/// crate so logging stays on std plus `windows`.
fn format_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

/// Howard Hinnant's `civil_from_days` algorithm (days since 1970-01-01).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    fn temp_directory(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "plnk-crash-log-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("create the crash-log test directory");
        directory
    }

    /// The crash log keeps only the newest bytes: old lines fall out of the
    /// retained window, the newest line survives, and the file never exceeds
    /// the bound.
    #[test]
    fn crash_log_truncation_keeps_newest_line_within_bound() {
        let directory = temp_directory("truncate");
        let path = directory.join(CRASH_LOG_FILE_NAME);
        let limit = 256;
        for index in 0..50 {
            append_crash_line_with_limit(
                &directory,
                &format!("2026-01-01T00:00:00Z PANIC line {index} {}", "x".repeat(40)),
                limit,
            )
            .expect("append a crash line");
        }

        let bytes = fs::read(&path).expect("read the crash log");
        assert!(
            bytes.len() <= limit,
            "crash log must stay within {limit} bytes, found {}",
            bytes.len()
        );
        let text = String::from_utf8(bytes).expect("line-boundary truncation keeps UTF-8");
        assert!(
            text.contains("PANIC line 49"),
            "the newest line must be kept: {text:?}"
        );
        assert!(
            !text.contains("PANIC line 0 "),
            "old lines must be dropped: {text:?}"
        );
        assert!(!text.is_empty(), "the newest line must not be dropped");
        let _ = fs::remove_dir_all(&directory);
    }

    /// A single panic message larger than the whole log is truncated from the
    /// tail, but its timestamp/location head stays readable.
    #[test]
    fn oversized_crash_line_is_truncated_but_keeps_its_head() {
        let directory = temp_directory("oversized");
        let path = directory.join(CRASH_LOG_FILE_NAME);
        let line = format!(
            "2026-01-01T00:00:00Z PANIC at src/main.rs:1:1: {}",
            "y".repeat(1024)
        );
        append_crash_line_with_limit(&directory, &line, 128).expect("append an oversized line");

        let bytes = fs::read(&path).expect("read the crash log");
        assert!(
            bytes.len() <= 128,
            "oversized crash line must be bounded: {} bytes",
            bytes.len()
        );
        let text = String::from_utf8(bytes).expect("truncation keeps UTF-8");
        assert!(
            text.starts_with("2026-01-01T00:00:00Z PANIC at src/main.rs:1:1:"),
            "the timestamp and location must survive: {text:?}"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    /// Truncation must never split a multi-byte character.
    #[test]
    fn crash_log_truncation_preserves_utf8() {
        let directory = temp_directory("utf8");
        let path = directory.join(CRASH_LOG_FILE_NAME);
        for index in 0..20 {
            append_crash_line_with_limit(&directory, &format!("PANIC 中文消息 {index}"), 64)
                .expect("append a non-ASCII crash line");
            let bytes = fs::read(&path).expect("read the crash log");
            assert!(
                bytes.len() <= 64,
                "crash log must stay bounded with non-ASCII text: {} bytes",
                bytes.len()
            );
            String::from_utf8(bytes).expect("truncation must not split a UTF-8 character");
        }
        let _ = fs::remove_dir_all(&directory);
    }
}
