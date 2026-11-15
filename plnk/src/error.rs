//! Whole-file errors for the settings layer.
//!
//! A single invalid value inside an INI document is not an error: it produces a
//! [`crate::settings::SettingsWarning`] and the rest of the document still
//! imports. Only failures that make a whole document unusable (or a save
//! impossible) reach this type.

use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum SettingsError {
    /// A settings or legacy INI file could not be read. The path is shown to
    /// the user.
    Read { path: PathBuf, source: io::Error },
    /// The new settings file could not be written. The path is shown to the
    /// user.
    Write { path: PathBuf, source: io::Error },
    /// Neither the portable settings directory nor the `%LOCALAPPDATA%`
    /// fallback could be created or written. Both candidate paths are shown to
    /// the user.
    StorageUnavailable {
        portable: PathBuf,
        fallback: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::Write { path, source } => {
                write!(formatter, "failed to write {}: {source}", path.display())
            }
            Self::StorageUnavailable {
                portable,
                fallback,
                source,
            } => write!(
                formatter,
                "failed to create a writable settings directory (tried {} then {}): {source}",
                portable.display(),
                fallback.display()
            ),
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. }
            | Self::Write { source, .. }
            | Self::StorageUnavailable { source, .. } => Some(source),
        }
    }
}
