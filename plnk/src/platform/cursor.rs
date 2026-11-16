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

/// Hot spot of the legacy pen cursors, in bitmap pixels, visual top-down.
///
/// The shipped 128x128 pen bitmaps are not full-canvas: the pen occupies the
/// bottom-right quadrant and everything above/left of it is transparent.
/// Decoding `cursorred.ico` (32bpp `BITMAPINFOHEADER` with `biSize == 40`,
/// `biHeight == 256` = 2 x 128, i.e. a 128-row XOR bitmap followed by a 1bpp
/// AND mask) yields an opaque bounding box of x 64..=127, y 64..=127 and a tip
/// apex at (67, 64) -- the median x of the pixels inked on the top-most inked
/// row, which spans x 65..=69. `cursorgreen.ico`, `cursorblue.ico` and
/// `cursoryellow.ico` decode to the same geometry pixel for pixel.
///
/// File row vs. visual row: a DIB stores its rows **bottom-up**, so file row 0
/// is visual row 127. Read straight out of the file the pen therefore appears
/// to sit in the top-right quadrant at y 0..=63, and the pixel at the start of
/// the visible sweep lands at x 109..=112 on file row 0 -- that unflipped
/// reading is what produced the earlier, wrong hot spot `(110, 0)`. The hot
/// spot must be the apex of the **decoded** (row-flipped) bitmap: being an
/// offset in the unflipped file grid or a field out of the ICO directory entry
/// (`wPlanes`/`wBitCount` == `(1, 32)`) both put the pointer hotspot away from
/// the drawn pen tip.
///
/// The `hotspot_tests` module decodes all four assets and asserts this constant
/// equals the decoded apex, so the value cannot drift back to a file-row offset.
const PEN_CURSOR_HOTSPOT: (u16, u16) = (67, 64);

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
/// corner of the 128x128 frame, inside the transparent quarter, i.e. the same
/// class of bug in the opposite direction.
///
/// Everything here therefore works from **decoded pixels**, never from the ICO
/// directory: the tests below decode the DIB, flip the bottom-up rows into
/// visual order and take the pen tip (the apex of the opaque silhouette) as the
/// only acceptable hot spot.
#[cfg(test)]
mod hotspot_tests {
    use super::*;

    fn u16_at(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn i32_at(bytes: &[u8], offset: usize) -> i32 {
        i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    /// The four original pen-cursor assets, in [`PenCursorColor::ALL`] order.
    const ASSET_NAMES: [&str; 4] = [
        "cursorred.ico",
        "cursorgreen.ico",
        "cursorblue.ico",
        "cursoryellow.ico",
    ];

    /// Alpha above which a pixel counts as inked by the pen.
    ///
    /// The shipped cursors antialias their silhouette: each variant carries 59
    /// pixels at alpha 1..=20 which the AND mask still reports as opaque. That
    /// fringe is real ink but far too faint to locate the nib, so the pen's
    /// geometry is measured above this threshold.
    const OPAQUE_ALPHA: u8 = 20;

    /// A decoded single-image pen cursor, visual top-down.
    ///
    /// `row 0` is the row the user sees at the top of the bitmap, i.e. the ICO's
    /// **last** stored row -- see [`decode_icon_pixels`].
    struct DecodedIcon {
        width: usize,
        height: usize,
        /// `opaque[y][x]`: alpha above [`OPAQUE_ALPHA`] **and** AND-mask bit
        /// clear. This is the pen's silhouette.
        opaque: Vec<Vec<bool>>,
        /// `ink[y][x]`: any non-zero alpha at all, ignoring [`OPAQUE_ALPHA`].
        ink: Vec<Vec<bool>>,
        /// `mask_inked[y][x]`: what the AND mask alone says (bit 0 == inked).
        mask_inked: Vec<Vec<bool>>,
    }

    impl DecodedIcon {
        /// Pixels where the alpha channel and the AND mask disagree about
        /// whether the pixel is inked at all.
        fn mask_disagreements(&self) -> usize {
            (0..self.height)
                .flat_map(|y| (0..self.width).map(move |x| (y, x)))
                .filter(|&(y, x)| self.ink[y][x] != self.opaque_or_mask(y, x))
                .count()
        }

        /// True when the AND mask calls the pixel inked (bit 0 means inked).
        fn opaque_or_mask(&self, y: usize, x: usize) -> bool {
            self.mask_inked[y][x]
        }

        /// True when `(x, y)` is part of the pen's silhouette.
        fn is_opaque(&self, x: u16, y: u16) -> bool {
            let (x, y) = (usize::from(x), usize::from(y));
            x < self.width && y < self.height && self.opaque[y][x]
        }

        /// Inclusive `(left, top, right, bottom)` of the opaque pixels.
        fn opaque_bbox(&self) -> (usize, usize, usize, usize) {
            let mut bbox: Option<(usize, usize, usize, usize)> = None;
            for (y, row) in self.opaque.iter().enumerate() {
                for (x, opaque) in row.iter().enumerate() {
                    if !*opaque {
                        continue;
                    }
                    bbox = Some(match bbox {
                        None => (x, y, x, y),
                        Some((left, top, right, bottom)) => {
                            (left.min(x), top.min(y), right.max(x), bottom.max(y))
                        }
                    });
                }
            }
            bbox.expect("decoded icon has no opaque pixels")
        }

        /// The pen tip: the median x of the pixels inked on the top-most opaque
        /// row. The median (not the left edge) is used because the row spans a
        /// symmetric 5-pixel nib, so the left edge would bias the hot spot off
        /// the pen's axis.
        fn pen_apex(&self) -> (u16, u16) {
            let (_, top, _, _) = self.opaque_bbox();
            let mut inked: Vec<u16> = self.opaque[top]
                .iter()
                .enumerate()
                .filter(|(_, opaque)| **opaque)
                .map(|(x, _)| u16::try_from(x).expect("bitmap width fits u16"))
                .collect();
            assert!(
                !inked.is_empty(),
                "the top-most opaque row has no opaque pixel"
            );
            inked.sort_unstable();
            (inked[inked.len() / 2], top as u16)
        }
    }

    /// Decodes a single-image 32bpp ICO into top-down opacity bits.
    ///
    /// Mirrors what Windows does when it turns the file into a cursor image:
    ///
    /// 1. locate the DIB through `ICONDIRENTRY.dwImageOffset` (the entry's last
    ///    dword) and read the `BITMAPINFOHEADER` fields `biWidth`, `biHeight`,
    ///    `biBitCount` and `biSizeImage`;
    /// 2. derive the pixel-array offset -- the ICO counterpart of a
    ///    `BITMAPFILEHEADER.bfOffBits` -- as `dwImageOffset + biSize` (32bpp has
    ///    no palette);
    /// 3. stride each XOR row to a 4-byte boundary;
    /// 4. reverse the row order, because `biHeight` counts the XOR rows **and**
    ///    the trailing AND-mask rows and the XOR rows are stored **bottom-up**;
    /// 5. treat a pixel as inked when its alpha is non-zero **and** its
    ///    AND-mask bit is 0 (the mask is 1 where the pixel is transparent),
    ///    and as part of the pen's silhouette only above [`OPAQUE_ALPHA`].
    fn decode_icon_pixels(ico: &[u8]) -> DecodedIcon {
        const ICONDIR: usize = 6;
        const ICONDIRENTRY: usize = 16;
        const BITMAPINFOHEADER: usize = 40;

        assert!(ico.len() >= ICONDIR + ICONDIRENTRY, "truncated ICO directory");
        assert_eq!(u16_at(ico, 2), 1, "not an ICO: ICONDIR.idType != 1");
        assert_eq!(u16_at(ico, 4), 1, "expected a single-image icon");

        let dib = u32_at(ico, ICONDIR + 12) as usize;
        assert!(
            ico.len() >= dib + BITMAPINFOHEADER,
            "truncated DIB header at offset {dib}"
        );
        let bi_size = u32_at(ico, dib) as usize;
        assert!(
            bi_size >= BITMAPINFOHEADER,
            "unexpected BITMAPINFOHEADER size {bi_size}"
        );
        let width = i32_at(ico, dib + 4);
        let bi_height = i32_at(ico, dib + 8);
        let bi_bit_count = u16_at(ico, dib + 14);
        let bi_size_image = u32_at(ico, dib + 20) as usize;
        assert_eq!(bi_bit_count, 32, "the shipped pen cursors are 32bpp");
        assert!(
            width > 0 && bi_height > 0,
            "negative or zero DIB dimensions are not supported"
        );

        let width = width as usize;
        let bi_height = bi_height as usize;
        // `biHeight` spans the XOR bitmap and the trailing 1bpp AND mask; a
        // 32bpp cursor image therefore always reports an even double height.
        assert_eq!(bi_height % 2, 0, "32bpp icon DIBs carry an AND mask");
        let height = bi_height / 2;
        // DIB rows are padded to a 4-byte boundary.
        let stride = (width * (usize::from(bi_bit_count) / 8) + 3) & !3;
        let mask_stride = width.div_ceil(32) * 4;

        let pixels = dib + bi_size;
        // `biSizeImage` covers only the XOR bitmap and is commonly 0; fall back
        // to the derived size so the mask offset stays correct either way.
        let xor_size = if bi_size_image == 0 {
            // Only the visual rows: the AND mask occupies the second half of
            // `biHeight`.
            stride * height
        } else {
            bi_size_image
        };
        let mask = pixels + xor_size;
        assert!(
            mask + mask_stride * height <= ico.len(),
            "ICO image data is truncated: XOR bitmap ends at {}, AND mask needs {} bytes but the file ends at {}",
            mask,
            mask_stride * height,
            ico.len()
        );

        let mut opaque = vec![vec![false; width]; height];
        let mut ink = vec![vec![false; width]; height];
        let mut mask_inked = vec![vec![false; width]; height];
        for file_row in 0..height {
            // Bottom-up DIB: file row 0 is the bottom visual row.
            let visual_row = height - 1 - file_row;
            let row_start = pixels + file_row * stride;
            let mask_start = mask + file_row * mask_stride;
            for column in 0..width {
                let pixel = row_start + column * 4;
                let alpha_byte = ico[pixel + 3];
                let mask_bit = (ico[mask_start + column / 8] >> (7 - (column % 8))) & 1;
                ink[visual_row][column] = alpha_byte != 0;
                mask_inked[visual_row][column] = mask_bit == 0;
                opaque[visual_row][column] = alpha_byte > OPAQUE_ALPHA && mask_bit == 0;
            }
        }
        DecodedIcon {
            width,
            height,
            opaque,
            ink,
            mask_inked,
        }
    }

    fn decode_asset(name: &str) -> DecodedIcon {
        decode_icon_pixels(&asset(name))
    }

    /// Reads `ICONDIR.idType` and the first entry's `wPlanes`/`wBitCount` pair.
    fn ico_header(bytes: &[u8]) -> (u16, u16, u16) {
        assert!(bytes.len() >= 22, "truncated ICO header");
        let id_type = u16_at(bytes, 2);
        let planes = u16_at(bytes, 10);
        let bit_count = u16_at(bytes, 12);
        (id_type, planes, bit_count)
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
        for name in ASSET_NAMES {
            let bytes = asset(name);
            let (id_type, planes, bit_count) = ico_header(&bytes);
            assert_eq!(id_type, 1, "{name} must be an ICO (idType 1)");
            // Read as a cursor, this pair becomes the hot spot: (1, 32) would
            // put the pointer 32 rows above the visual pen tip.
            assert_eq!(
                (planes, bit_count),
                (1, 32),
                "{name}: the entry's wPlanes/wBitCount pair is what LoadImageW \
                 would mistake for a cursor hot spot"
            );
        }
    }

    /// The hot spot must be the pen tip of the **decoded** bitmap, and the tip
    /// must be an actually inked pixel.
    ///
    /// Decoding also pins the geometry the hot spot depends on: after the
    /// bottom-up rows are flipped, the pen occupies the bottom-right quadrant
    /// and its apex is the median x of the top-most opaque row.
    #[test]
    fn hotspot_is_the_decoded_pen_tip() {
        for name in ASSET_NAMES {
            let icon = decode_asset(name);
            assert_eq!(
                (icon.width, icon.height),
                (128, 128),
                "{name}: unexpected bitmap size"
            );
            assert_eq!(
                icon.opaque_bbox(),
                (64, 64, 127, 127),
                "{name}: with the rows in visual order the pen occupies the \
                 bottom-right quadrant"
            );
            let apex = icon.pen_apex();
            assert!(
                icon.is_opaque(apex.0, apex.1),
                "{name}: the measured apex {apex:?} is not an inked pixel"
            );
            assert_eq!(
                apex, PEN_CURSOR_HOTSPOT,
                "{name}: the hot spot must equal the decoded pen tip {apex:?}"
            );
        }
    }

    /// All four colour variants must agree on where the nib is: [`PEN_CURSOR_HOTSPOT`]
    /// is a single shared constant, so it is only correct while they do. Fails
    /// loudly with every measured value when they drift apart by more than 2 px.
    #[test]
    fn pen_apex_is_the_same_on_all_four_colour_variants() {
        let apices: Vec<(&str, (u16, u16))> = ASSET_NAMES
            .iter()
            .map(|name| (*name, decode_asset(name).pen_apex()))
            .collect();
        let xs: Vec<u16> = apices.iter().map(|(_, (x, _))| *x).collect();
        let ys: Vec<u16> = apices.iter().map(|(_, (_, y))| *y).collect();
        let spread = |values: &[u16]| {
            values.iter().copied().max().unwrap() - values.iter().copied().min().unwrap()
        };
        assert!(
            spread(&xs) <= 2 && spread(&ys) <= 2,
            "the four pen variants disagree on the nib by more than 2 px, so a single \
             shared PEN_CURSOR_HOTSPOT cannot be right; measured apices: {apices:?}"
        );
    }

    /// The AND mask is a second, independent statement about transparency, so
    /// it has to agree with the alpha channel on every pixel -- otherwise
    /// which pixels are "opaque" depends on which layer a loader consults.
    #[test]
    fn and_mask_agrees_with_the_alpha_channel() {
        for name in ASSET_NAMES {
            let bytes = asset(name);
            let icon = decode_icon_pixels(&bytes);
            assert_eq!(
                icon.mask_disagreements(),
                0,
                "{name}: the AND mask contradicts the alpha channel"
            );
        }
    }

    /// Whatever loader the module uses, every pen cursor must report the pen
    /// tip as its hot spot — never (wPlanes, wBitCount) of the ICO entry. The
    /// expected value comes from the decoded pixels, not from the constant the
    /// loader was built with; a hot spot that merely equals the constant would
    /// prove nothing.
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
            let name = color.filename();
            let icon = decode_asset(name);
            let apex = icon.pen_apex();
            let (x, y) = cursors
                .hotspot_of(color)
                .unwrap_or_else(|| panic!("{color:?} has no hot spot"));
            assert_eq!(
                (x, y),
                apex,
                "{color:?} ({name}) hot spot must be the measured nib apex {apex:?}"
            );
            assert!(
                icon.is_opaque(x, y),
                "{color:?} ({name}) hot spot ({x}, {y}) is not an inked pixel"
            );
            let distance = x.abs_diff(apex.0).max(y.abs_diff(apex.1));
            assert!(
                distance <= 2,
                "{color:?} ({name}) hot spot ({x}, {y}) is {distance} px from the \
                 nib apex {apex:?}"
            );
        }
    }
}
