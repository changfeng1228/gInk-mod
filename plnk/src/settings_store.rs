//! Loading, importing and saving the plnk settings file.
//!
//! `SettingsStore` owns the new settings path and the legacy search order so
//! that no other module needs to know where the old gInk files live. Import is
//! strictly read-only: the legacy files are never created, rewritten or
//! deleted.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::error::SettingsError;
use crate::settings::{
    Settings, SettingsImport, decode_text, render_settings_document, rewrite_ini_document,
};

/// Owns the new settings location plus the imported legacy source documents.
pub struct SettingsStore {
    settings_path: PathBuf,
    legacy_search_dirs: Vec<PathBuf>,
    /// Raw legacy documents in legacy read order (pens, config, hotkeys), kept
    /// so saving can preserve comments and unknown lines.
    sources: Vec<String>,
}

impl SettingsStore {
    pub fn new(settings_path: PathBuf, legacy_search_dirs: Vec<PathBuf>) -> Self {
        Self {
            settings_path,
            legacy_search_dirs,
            sources: Vec::new(),
        }
    }

    /// Loads the new settings file when it exists; otherwise imports the legacy
    /// files using the legacy lookup order (each search directory in turn,
    /// current directory first, executable directory second).
    pub fn load_or_import(&mut self) -> Result<SettingsImport, SettingsError> {
        self.sources.clear();

        if self.settings_path.exists() {
            let text = read_text(&self.settings_path)?;
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
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let text = if self.sources.is_empty() {
            render_settings_document(settings)
        } else {
            let mut text = String::new();
            for source in &self.sources {
                text.push_str(&rewrite_ini_document(source, settings));
            }
            text
        };

        if let Some(parent) = self.settings_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| SettingsError::Write {
                path: self.settings_path.clone(),
                source,
            })?;
        }
        fs::write(&self.settings_path, text).map_err(|source| SettingsError::Write {
            path: self.settings_path.clone(),
            source,
        })
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
    /// The production layout: settings in `<exe dir>/data/settings.ini`, legacy
    /// files searched in the current directory first and the executable
    /// directory second, exactly like the legacy program.
    fn default() -> Self {
        let executable_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let mut legacy_search_dirs = Vec::new();
        if let Ok(current_dir) = std::env::current_dir() {
            legacy_search_dirs.push(current_dir);
        }
        legacy_search_dirs.push(executable_dir.clone());
        Self::new(
            executable_dir.join("data").join("settings.ini"),
            legacy_search_dirs,
        )
    }
}

fn read_text(path: &Path) -> Result<String, SettingsError> {
    let bytes = fs::read(path).map_err(|source| SettingsError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(decode_text(&bytes))
}
