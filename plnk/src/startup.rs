//! Per-user autostart.
//!
//! Autostart is a single `.lnk` file in the current user's Startup folder, the
//! same mechanism the release packaging uses
//! (`scripts/update-plnk-shortcuts.ps1` writes the same file). Chosen
//! deliberately over `HKCU\...\Run`:
//!
//! - No administrator rights and no registry access at all.
//! - Uninstalling is one file delete, and the user can inspect or remove it
//!   from Explorer without any tool.
//! - It matches what the packaging script already does, so enabling autostart
//!   from the options window and running a release update converge on the same
//!   artefact instead of fighting over two different mechanisms.
//!
//! The shortcut is created through `WScript.Shell` reached with PowerShell,
//! the same tool the packaging script uses, so both produce the same file.

use std::path::{Path, PathBuf};

/// Name of the autostart shortcut inside the Startup folder.
pub const SHORTCUT_NAME: &str = "plnk.lnk";

/// Failure of an autostart operation. Carries the path so the options window
/// can tell the user exactly which file it could not write.
#[derive(Debug)]
pub struct StartupError {
    pub path: PathBuf,
    pub message: String,
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "autostart shortcut {}: {}",
            self.path.display(),
            self.message
        )
    }
}

impl std::error::Error for StartupError {}

/// The current user's Startup folder.
pub fn startup_directory() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup"),
    )
}

/// Path of the autostart shortcut for this user.
pub fn shortcut_path() -> Option<PathBuf> {
    startup_directory().map(|directory| directory.join(SHORTCUT_NAME))
}

/// Whether autostart is currently enabled for this user. Absence of the file is
/// the definition of "disabled", so a user who deletes it from Explorer sees
/// the checkbox already off on the next open.
pub fn is_enabled() -> bool {
    shortcut_path().is_some_and(|path| is_enabled_at(&path))
}

/// Testable core of [`is_enabled`]: the shortcut exists and is a file.
pub fn is_enabled_at(path: &Path) -> bool {
    path.is_file()
}

/// Enables or disables autostart for this user.
pub fn set_enabled(enabled: bool) -> Result<(), StartupError> {
    let path = shortcut_path().ok_or_else(|| StartupError {
        path: PathBuf::from("<APPDATA unavailable>"),
        message: "the user profile has no APPDATA directory".to_owned(),
    })?;
    let executable = std::env::current_exe().map_err(|error| StartupError {
        path: path.clone(),
        message: format!("cannot resolve the running executable: {error}"),
    })?;
    set_enabled_at(&path, enabled, &executable)
}

/// Testable core of [`set_enabled`]. `enabled = true` creates or refreshes the
/// shortcut so it always points at the running executable; `enabled = false`
/// removes it, and removing an absent shortcut is success.
pub fn set_enabled_at(path: &Path, enabled: bool, executable: &Path) -> Result<(), StartupError> {
    if !enabled {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StartupError {
                path: path.to_path_buf(),
                message: format!("cannot remove the shortcut: {error}"),
            }),
        };
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| StartupError {
            path: path.to_path_buf(),
            message: format!("cannot create the Startup folder: {error}"),
        })?;
    }
    write_shortcut(path, executable)
}

/// Writes the `.lnk` through `WScript.Shell` via PowerShell.
fn write_shortcut(path: &Path, executable: &Path) -> Result<(), StartupError> {
    let working = executable
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_default();
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $s=(New-Object -ComObject WScript.Shell).CreateShortcut('{link}'); \
         $s.TargetPath='{target}'; \
         $s.WorkingDirectory='{working}'; \
         $s.WindowStyle=7; \
         $s.Description='plnk on-screen annotation'; \
         $s.Save()",
        link = path.display(),
        target = executable.display(),
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|error| StartupError {
            path: path.to_path_buf(),
            message: format!("cannot run PowerShell to create the shortcut: {error}"),
        })?;
    if !output.status.success() {
        return Err(StartupError {
            path: path.to_path_buf(),
            message: format!(
                "PowerShell failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    if !path.is_file() {
        return Err(StartupError {
            path: path.to_path_buf(),
            message: "PowerShell reported success but wrote no shortcut".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_name_matches_the_packaging_script() {
        assert_eq!(SHORTCUT_NAME, "plnk.lnk");
    }

    #[test]
    fn missing_shortcut_means_disabled() {
        let path = std::env::temp_dir().join("plnk-definitely-absent.lnk");
        let _ = std::fs::remove_file(&path);
        assert!(!is_enabled_at(&path));
    }

    #[test]
    fn disabling_an_absent_shortcut_succeeds() {
        let path = std::env::temp_dir().join("plnk-absent-disable.lnk");
        let _ = std::fs::remove_file(&path);
        set_enabled_at(&path, false, Path::new("plnk.exe"))
            .expect("removing an absent shortcut must be a no-op success");
    }
}
