//! Portable asset locations shared by the toolbar and notification icon.
//!
//! Assets are deliberately resolved from the executable path, never from the
//! process working directory, so double-clicking a packaged build and launching
//! it from a terminal behave identically.

use std::io;
use std::path::{Path, PathBuf};

/// Returns `<executable directory>/assets/icons` without touching the working directory.
pub fn icon_directory_for(executable: &Path) -> PathBuf {
    executable
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("assets")
        .join("icons")
}

/// Resolves the asset directory for the running executable.
pub fn icon_directory() -> io::Result<PathBuf> {
    std::env::current_exe().map(|executable| {
        let packaged = icon_directory_for(&executable);
        #[cfg(debug_assertions)]
        if !packaged.is_dir() {
            // Cargo's integration tests run from target/debug/build/<hash>/out
            // rather than target/debug. Development builds may use the source
            // assets; release builds never take this branch.
            let development = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join("icons");
            if development.is_dir() {
                return development;
            }
        }
        packaged
    })
}

/// Legacy `Root.SetTrayIconColor` selection.
pub fn tray_icon_filename(white: bool) -> &'static str {
    if white {
        "icon_white.ico"
    } else {
        "icon_red.ico"
    }
}
