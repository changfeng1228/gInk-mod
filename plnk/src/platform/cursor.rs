//! Canvas cursors matching the original gInk pen cursors.
//!
//! The legacy application loads `cursorred.ico` for the drawing canvas (the
//! original project also ships green/blue/yellow variants); this module maps
//! the active pen colour onto those original ICO assets. Every file is loaded
//! with `LoadImageW(LR_LOADFROMFILE)`; a missing or unreadable file never fails
//! application start — the affected pen colour falls back to the standard
//! arrow cursor and a warning is recorded for the shell log.

use std::path::{Path, PathBuf};

use windows::Win32::UI::WindowsAndMessaging::{
    DestroyCursor, HCURSOR, IDC_ARROW, IMAGE_CURSOR, LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadCursorW,
    LoadImageW,
};
use windows::core::PCWSTR;

/// Original gInk pen-cursor colour variants shipped next to the toolbar assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenCursorColor {
    Red,
    Green,
    Blue,
    Yellow,
}

impl PenCursorColor {
    /// The original asset file for this variant.
    pub const fn filename(self) -> &'static str {
        match self {
            Self::Red => "cursorred.ico",
            Self::Green => "cursorgreen.ico",
            Self::Blue => "cursorblue.ico",
            Self::Yellow => "cursoryellow.ico",
        }
    }

    /// All variants, in the order the legacy icon set lists them.
    pub const ALL: [Self; 4] = [Self::Red, Self::Green, Self::Blue, Self::Yellow];

    /// Legacy-style colour classification for the active pen.
    ///
    /// The four original cursors correspond to the default pen palette (red,
    /// blue, yellow, green). Pens whose colour is not clearly one of those
    /// (grey, white, pink, teal, ...) use the standard arrow instead of a
    /// misleading coloured cursor.
    pub fn for_rgb(red: u8, green: u8, blue: u8) -> Option<Self> {
        let red = u16::from(red);
        let green = u16::from(green);
        let blue = u16::from(blue);
        if blue > red + 40 && blue > green + 20 {
            Some(Self::Blue)
        } else if green > red + 30 && green > blue + 20 {
            Some(Self::Green)
        } else if red > 180 && green > 120 && blue < 120 {
            // Orange highlighters land here too; the yellow cursor is the
            // closest original variant.
            Some(Self::Yellow)
        } else if red > green + 40 && red > blue + 40 {
            Some(Self::Red)
        } else {
            None
        }
    }
}

/// Loaded canvas cursors: the stock arrow plus one optional original ICO per
/// pen colour. Owned cursors are destroyed in [`Drop`].
pub struct CanvasCursors {
    arrow: HCURSOR,
    pens: [Option<HCURSOR>; 4],
    owned: Vec<HCURSOR>,
    warnings: Vec<String>,
}

impl CanvasCursors {
    /// Loads the four original pen cursors from `directory` (the packaged
    /// `assets/icons` directory). A missing file is recorded as a warning and
    /// falls back to the standard arrow cursor.
    pub fn load(directory: &Path) -> Self {
        let mut warnings = Vec::new();
        let arrow = match unsafe { LoadCursorW(None, IDC_ARROW) } {
            Ok(cursor) => cursor,
            Err(error) => {
                warnings.push(format!(
                    "standard arrow cursor could not be loaded: {error}; the pen cursor cannot be restored"
                ));
                HCURSOR::default()
            }
        };
        let mut cursors = Self {
            arrow,
            pens: [None; 4],
            owned: Vec::new(),
            warnings,
        };
        for (slot, color) in cursors.pens.iter_mut().zip(PenCursorColor::ALL) {
            let path: PathBuf = directory.join(color.filename());
            match load_cursor_file(&path) {
                Ok(cursor) => {
                    *slot = Some(cursor);
                    cursors.owned.push(cursor);
                }
                Err(error) => cursors.warnings.push(format!(
                    "legacy pen cursor {} could not be loaded: {error}; falling back to the standard arrow cursor",
                    path.display()
                )),
            }
        }
        cursors
    }

    /// The standard arrow cursor (also the class cursor of plnk windows).
    pub fn arrow(&self) -> HCURSOR {
        self.arrow
    }

    /// Cursor for the active pen colour, falling back to the arrow cursor when
    /// the pen is not one of the original four colours.
    pub fn for_pen(&self, red: u8, green: u8, blue: u8) -> HCURSOR {
        PenCursorColor::for_rgb(red, green, blue)
            .and_then(|color| {
                self.pens[PenCursorColor::ALL
                    .iter()
                    .position(|candidate| *candidate == color)?]
            })
            .unwrap_or(self.arrow)
    }

    /// Number of original pen-cursor files that loaded successfully.
    pub fn loaded_count(&self) -> usize {
        self.owned.len()
    }

    /// Drains accumulated load warnings for the shell log.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// Records an external warning (for example a missing asset directory) in
    /// the same stream as the cursor-load warnings.
    pub fn record_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

impl Drop for CanvasCursors {
    fn drop(&mut self) {
        for cursor in self.owned.drain(..) {
            if cursor != self.arrow && !cursor.is_invalid() {
                let _ = unsafe { DestroyCursor(cursor) };
            }
        }
    }
}

fn load_cursor_file(path: &Path) -> Result<HCURSOR, windows::core::Error> {
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        LoadImageW(
            None,
            PCWSTR(wide.as_ptr()),
            IMAGE_CURSOR,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
    }?;
    Ok(HCURSOR(handle.0))
}

/// Class cursor used by every plnk window: the standard arrow, never the
/// system "app starting"/wait cursor a NULL class cursor would leave behind.
pub fn default_class_cursor() -> HCURSOR {
    unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pen_palette_maps_to_the_original_cursor_colours() {
        assert_eq!(
            PenCursorColor::for_rgb(225, 60, 60),
            Some(PenCursorColor::Red)
        );
        assert_eq!(
            PenCursorColor::for_rgb(30, 110, 200),
            Some(PenCursorColor::Blue)
        );
        assert_eq!(
            PenCursorColor::for_rgb(250, 180, 55),
            Some(PenCursorColor::Yellow)
        );
        assert_eq!(
            PenCursorColor::for_rgb(120, 175, 70),
            Some(PenCursorColor::Green)
        );
    }

    #[test]
    fn neutral_and_unusual_pen_colours_use_the_arrow() {
        for (red, green, blue) in [
            (80, 80, 80),
            (230, 230, 230),
            (240, 140, 200),
            (25, 180, 175),
        ] {
            assert_eq!(
                PenCursorColor::for_rgb(red, green, blue),
                None,
                "colour ({red},{green},{blue}) has no matching original cursor"
            );
        }
    }

    #[test]
    fn missing_cursor_directory_falls_back_to_arrow_with_warnings() {
        let mut cursors = CanvasCursors::load(Path::new("Z:\\plnk-missing-cursor-assets"));
        assert_eq!(cursors.loaded_count(), 0);
        let warnings = cursors.take_warnings();
        assert_eq!(warnings.len(), 4, "one warning per missing original cursor");
        assert!(
            warnings
                .iter()
                .all(|warning| warning.contains("falling back"))
        );
        let arrow = cursors.arrow();
        assert_eq!(cursors.for_pen(225, 60, 60), arrow);
    }
}
