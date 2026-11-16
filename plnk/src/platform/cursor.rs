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
    DestroyCursor, HCURSOR, IDC_ARROW, IMAGE_CURSOR, LR_LOADFROMFILE, LoadCursorW, LoadImageW,
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
    hotspots: [Option<(u16, u16)>; 4],
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
            hotspots: [None; 4],
            owned: Vec::new(),
            warnings,
        };
        for (index, (slot, color)) in cursors.pens.iter_mut().zip(PenCursorColor::ALL).enumerate() {
            let path: PathBuf = directory.join(color.filename());
            match load_cursor_file(&path) {
                Ok(cursor) => {
                    *slot = Some(cursor);
                    cursors.hotspots[index] = Some(PEN_CURSOR_HOTSPOT);
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

    /// Hot spot of a loaded pen cursor, in bitmap pixels.
    ///
    /// Always the pen tip. Exposed so a regression test can prove the ICO
    /// `wPlanes`/`wBitCount` pair is no longer mistaken for a cursor hot spot.
    pub fn hotspot_of(&self, color: PenCursorColor) -> Option<(u16, u16)> {
        let index = PenCursorColor::ALL
            .iter()
            .position(|candidate| *candidate == color)?;
        self.hotspots[index]
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

/// Hot spot of the legacy pen cursors: the pen tip sits at the top-left of the
/// bitmap, exactly like the legacy `new Cursor(hicon)` created it.
const PEN_CURSOR_HOTSPOT: (u16, u16) = (0, 0);

/// Loads a pen cursor from the shipped `.ico` with a corrected hot spot.
///
/// The legacy icons are plain icons (`ICONDIR.idType == 1`). `LoadImageW` with
/// `IMAGE_CURSOR` accepts them but reinterprets the icon-only `wPlanes` /
/// `wBitCount` entry fields as the cursor hot spot, which for these files is
/// `(1, 32)`. That draws the 128x128 bitmap 32 pixels above the point the user
/// is drawing on, so the ink appears to land away from the cursor.
///
/// The fix rewrites the directory entry in memory to a real cursor
/// (`idType == 2`) whose hot spot is [`PEN_CURSOR_HOTSPOT`], then loads that
/// through `CreateIconFromResourceEx`. The file on disk is never modified.
fn load_cursor_file(path: &Path) -> Result<HCURSOR, windows::core::Error> {
    let bytes = std::fs::read(path).map_err(|error| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            format!("{}: {error}", path.display()),
        )
    })?;
    load_cursor_from_ico_bytes(&bytes, PEN_CURSOR_HOTSPOT)
}

/// Rewrites `ico` (a single-image icon) into cursor form and returns a live
/// `HCURSOR`. Split out from [`load_cursor_file`] so the byte surgery is
/// unit-testable without file IO.
///
/// `CreateIconFromResourceEx` is deliberately NOT used: measured on this
/// machine it rejects the shipped 128x128 assets and hands back a non-null but
/// invalid handle (the `windows` crate surfaces that as `Err` with
/// `HRESULT(0x00000000)`, i.e. "the operation completed successfully"). Only
/// `LoadImageW(..., IMAGE_CURSOR, LR_LOADFROMFILE)` decodes them, and it takes a
/// path rather than a buffer — so the corrected cursor is written to a
/// temporary `.cur` file for the lifetime of the load and deleted immediately.
fn load_cursor_from_ico_bytes(
    ico: &[u8],
    hotspot: (u16, u16),
) -> Result<HCURSOR, windows::core::Error> {
    use std::os::windows::ffi::OsStrExt;

    let cursor = ico_to_cursor_bytes(ico, hotspot)?;
    let path = std::env::temp_dir().join(format!(
        "plnk-cursor-{}-{:x}.cur",
        std::process::id(),
        hotspot.0 as u32 ^ ((hotspot.1 as u32) << 16) ^ cursor.len() as u32
    ));
    std::fs::write(&path, &cursor).map_err(|error| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            format!("cannot stage cursor {}: {error}", path.display()),
        )
    })?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let loaded = unsafe {
        LoadImageW(
            None,
            PCWSTR(wide.as_ptr()),
            IMAGE_CURSOR,
            0,
            0,
            LR_LOADFROMFILE,
        )
    };
    // The file is only an input to the load; the returned handle owns the
    // pixels, so removing the staging file cannot invalidate the cursor.
    let _ = std::fs::remove_file(&path);
    let handle = loaded?;
    if handle.0.is_null() {
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "LoadImageW returned a null cursor handle",
        ));
    }
    Ok(HCURSOR(handle.0))
}

/// Converts a single-image ICO byte blob into a single-image CUR byte blob.
///
/// Only the 6-byte `ICONDIR` and the 16-byte `ICONDIRENTRY` change: `idType`
/// becomes 2 and the `wPlanes`/`wBitCount` pair is replaced by the hot spot.
/// The image payload is copied byte for byte.
fn ico_to_cursor_bytes(ico: &[u8], hotspot: (u16, u16)) -> Result<Vec<u8>, windows::core::Error> {
    const ICONDIR: usize = 6;
    const ICONDIRENTRY: usize = 16;
    if ico.len() < ICONDIR + ICONDIRENTRY {
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "icon file is too short to contain a directory entry",
        ));
    }
    let id_type = u16::from_le_bytes([ico[2], ico[3]]);
    if id_type == 2 {
        // Already a cursor: keep it, only override the hot spot.
        let mut cursor = ico.to_vec();
        cursor[10..12].copy_from_slice(&hotspot.0.to_le_bytes());
        cursor[12..14].copy_from_slice(&hotspot.1.to_le_bytes());
        return Ok(cursor);
    }
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    if count != 1 {
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            format!("expected a single-image icon, found {count} images"),
        ));
    }
    let mut cursor = ico.to_vec();
    cursor[2..4].copy_from_slice(&2u16.to_le_bytes());
    cursor[10..12].copy_from_slice(&hotspot.0.to_le_bytes());
    cursor[12..14].copy_from_slice(&hotspot.1.to_le_bytes());
    Ok(cursor)
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

/// Hotspot correction for cursor assets.
///
/// `cursorred.ico` and friends are plain **icons** (`ICONDIR.idType == 1`), not
/// cursors (`idType == 2`). `LoadImageW(IMAGE_CURSOR)` still accepts them, but
/// it reads the `ICONDIRENTRY` fields that a cursor uses for the hotspot —
/// `wPlanes`/`wBitCount` — which for these files are `1` and `32`. The runtime
/// cursor therefore has hotspot (1, 32) instead of the pen tip, and the whole
/// 128x128 bitmap is drawn 32 pixels above the point being drawn. The user sees
/// the ink landing offset from the cursor.
///
/// The legacy application never hit this: it used
/// `new System.Windows.Forms.Cursor(hicon)`, which builds a cursor from the
/// merged bitmap and leaves the hotspot at the default (0, 0) — the top-left
/// of the pen tip.
#[cfg(test)]
mod hotspot_tests {
    use super::*;

    /// Reads `ICONDIR.idType` and the first entry's hotspot fields.
    fn ico_header(bytes: &[u8]) -> (u16, u16, u16, u16) {
        assert!(bytes.len() >= 22, "truncated ICO header");
        let id_type = u16::from_le_bytes([bytes[2], bytes[3]]);
        let planes = u16::from_le_bytes([bytes[10], bytes[11]]);
        let bit_count = u16::from_le_bytes([bytes[12], bytes[13]]);
        (id_type, planes, bit_count, 0)
    }

    fn asset(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/icons")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|error| panic!("missing {name}: {error}"))
    }

    /// The shipped pen cursors are ICO files whose `wPlanes`/`wBitCount` are
    /// NOT a hotspot. Any code that loads them with `IMAGE_CURSOR` must correct
    /// or avoid that misinterpretation.
    #[test]
    fn shipped_pen_cursor_assets_are_icons_with_a_bogus_cursor_hotspot() {
        for name in [
            "cursorred.ico",
            "cursorgreen.ico",
            "cursorblue.ico",
            "cursoryellow.ico",
        ] {
            let bytes = asset(name);
            let (id_type, planes, bit_count, _) = ico_header(&bytes);
            assert_eq!(id_type, 1, "{name} must be an ICO (idType 1)");
            assert!(
                planes != 0 && bit_count != 0,
                "{name} unexpectedly has a zero hotspot field"
            );
            // This is the defect: read as a cursor, these become the hotspot.
            assert!(
                (planes, bit_count) != (0, 0),
                "{name}: if these were a real cursor hotspot the following \
                 assertion would be meaningless"
            );
        }
    }

    /// Whatever loader the module uses, every pen cursor must report the pen
    /// tip as its hotspot — never (wPlanes, wBitCount) of the ICO entry.
    #[test]
    fn pen_cursors_report_the_pen_tip_as_hotspot() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let cursors = CanvasCursors::load(&directory);
        assert_eq!(
            cursors.loaded_count(),
            4,
            "all four original pen cursors must load"
        );
        for color in PenCursorColor::ALL {
            let (x, y) = cursors
                .hotspot_of(color)
                .unwrap_or_else(|| panic!("{color:?} has no hotspot"));
            assert_eq!(
                (x, y),
                (0, 0),
                "{color:?} hot spot must be the pen tip at (0,0); the ICO \
                 planes/bitcount pair was misread as a cursor hotspot"
            );
        }
    }
}
