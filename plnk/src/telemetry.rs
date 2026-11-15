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

        if self.written > 0
            && self.written + line_bytes > self.policy.max_file_bytes
            && self.rotate().is_err()
        {
            // Keep writing to the current file if rotation failed; a full disk
            // is reported by the caller's own error path, not by panicking.
            return;
        }
        if let Some(file) = self.file.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            self.written += line_bytes;
        }
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        rotate_files(&self.path, self.policy.max_files)?;
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        self.written = 0;
        Ok(())
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
