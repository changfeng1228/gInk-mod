//! Loading, importing and saving the plnk settings file.
//!
//! The new settings live in a writable storage root:
//!
//! 1. `<executable dir>/data` (portable, preferred), or
//! 2. `%LOCALAPPDATA%\plnk` when the portable directory cannot be created or
//!    written.
//!
//! The root is resolved once and reused, so loading and saving always use the
//! same directory. Import is strictly read-only: legacy gInk files are never
//! created, rewritten or deleted.

use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::error::SettingsError;
use crate::settings::{
    Settings, SettingsImport, decode_text, render_settings_document, rewrite_ini_document,
};

/// Candidate storage roots for the new settings file.
#[derive(Debug, Clone)]
pub struct StorageRoots {
    portable: PathBuf,
    fallback: PathBuf,
}

impl StorageRoots {
    pub fn new(portable: PathBuf, fallback: PathBuf) -> Self {
        Self { portable, fallback }
    }

    /// The production roots: `<exe dir>/data`, with `%LOCALAPPDATA%\plnk` as
    /// the fallback. If LOCALAPPDATA is missing, `<exe dir>/plnk` is used as a
    /// last resort so resolution never panics.
    pub fn from_environment() -> Self {
        let executable_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let fallback_parent = match std::env::var_os("LOCALAPPDATA") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => executable_dir.clone(),
        };
        Self::new(executable_dir.join("data"), fallback_parent.join("plnk"))
    }

    /// Returns the portable root when it can be created and written, otherwise
    /// the fallback root. The chosen directory is created when needed.
    ///
    /// Public so the application shell can put its log file in the same data
    /// directory that holds `settings.ini`.
    pub fn resolve(&self) -> Result<PathBuf, SettingsError> {
        if ensure_writable_directory(&self.portable).is_ok() {
            return Ok(self.portable.clone());
        }
        match ensure_writable_directory(&self.fallback) {
            Ok(()) => Ok(self.fallback.clone()),
            Err(source) => Err(SettingsError::StorageUnavailable {
                portable: self.portable.clone(),
                fallback: self.fallback.clone(),
                source,
            }),
        }
    }
}

/// Creates `root` if needed, then proves it is writable by creating and
/// removing a probe file. The probe name is unique per process and call.
fn ensure_writable_directory(root: &Path) -> io::Result<()> {
    fs::create_dir_all(root)?;
    static PROBE_COUNTER: AtomicU32 = AtomicU32::new(0);
    let probe = root.join(format!(
        ".plnk-write-probe-{}-{}",
        std::process::id(),
        PROBE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = fs::write(&probe, b"plnk");
    let _ = fs::remove_file(&probe);
    result
}

/// Owns the new settings location plus the imported legacy source documents.
pub struct SettingsStore {
    storage: StorageRoots,
    legacy_search_dirs: Vec<PathBuf>,
    /// Raw legacy documents in legacy read order (pens, config, hotkeys), kept
    /// so saving can preserve comments and unknown lines.
    sources: Vec<String>,
    /// The root chosen on first use; cached so load and save stay consistent.
    resolved_settings_path: Option<PathBuf>,
}

impl SettingsStore {
    pub fn new(storage: StorageRoots, legacy_search_dirs: Vec<PathBuf>) -> Self {
        Self {
            storage,
            legacy_search_dirs,
            sources: Vec::new(),
            resolved_settings_path: None,
        }
    }

    /// Loads the new settings file when it exists; otherwise imports the legacy
    /// files using the legacy lookup order (each search directory in turn,
    /// current directory first, executable directory second).
    pub fn load_or_import(&mut self) -> Result<SettingsImport, SettingsError> {
        self.sources.clear();
        let settings_path = self.resolve_settings_path()?;

        if settings_path.exists() {
            let text = read_text(&settings_path)?;
            let import = Settings::from_legacy(
                &mut Cursor::new(text.as_bytes()),
                &mut Cursor::new(Vec::<u8>::new()),
                &mut Cursor::new(Vec::<u8>::new()),
            )?;
            self.sources.push(text);
            return Ok(import);
        }

        let pens = self.read_legacy("pens.ini")?.unwrap_or_default();
        let config = self.read_legacy("config.ini")?.unwrap_or_default();
        let hotkeys = self.read_legacy("hotkeys.ini")?.unwrap_or_default();
        let import = Settings::from_legacy(
            &mut Cursor::new(config.as_bytes()),
            &mut Cursor::new(pens.as_bytes()),
            &mut Cursor::new(hotkeys.as_bytes()),
        )?;
        for text in [pens, config, hotkeys] {
            if !text.is_empty() {
                self.sources.push(text);
            }
        }
        Ok(import)
    }

    /// Writes the new settings file. Recognised keys are updated from
    /// `settings`; comments, section headers, blank lines and unknown lines of
    /// the imported documents are preserved.
    ///
    /// This is intentionally a direct write; Task 9 replaces it with an atomic
    /// save together with the options workflow.
    pub fn save(&mut self, settings: &Settings) -> Result<(), SettingsError> {
        let settings_path = self.resolve_settings_path()?;
        let text = if self.sources.is_empty() {
            render_settings_document(settings)
        } else {
            let mut text = String::new();
            for source in &self.sources {
                text.push_str(&rewrite_ini_document(source, settings));
            }
            text
        };

        fs::write(&settings_path, text).map_err(|source| SettingsError::Write {
            path: settings_path,
            source,
        })
    }

    fn resolve_settings_path(&mut self) -> Result<PathBuf, SettingsError> {
        if let Some(path) = &self.resolved_settings_path {
            return Ok(path.clone());
        }
        let path = self.storage.resolve()?.join("settings.ini");
        self.resolved_settings_path = Some(path.clone());
        Ok(path)
    }

    fn read_legacy(&self, file_name: &str) -> Result<Option<String>, SettingsError> {
        for directory in &self.legacy_search_dirs {
            let candidate = directory.join(file_name);
            if candidate.is_file() {
                return Ok(Some(read_text(&candidate)?));
            }
        }
        Ok(None)
    }
}

impl Default for SettingsStore {
    /// The production layout: settings in `<exe dir>/data/settings.ini`, with a
    /// `%LOCALAPPDATA%\plnk` fallback when that directory is not writable.
    /// Legacy files are searched in the current directory first and the
    /// executable directory second, exactly like the legacy program.
    fn default() -> Self {
        let executable_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let mut legacy_search_dirs = Vec::new();
        if let Ok(current_dir) = std::env::current_dir() {
            legacy_search_dirs.push(current_dir);
        }
        legacy_search_dirs.push(executable_dir);
        Self::new(StorageRoots::from_environment(), legacy_search_dirs)
    }
}

fn read_text(path: &Path) -> Result<String, SettingsError> {
    let bytes = fs::read(path).map_err(|source| SettingsError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(decode_text(&bytes))
}
