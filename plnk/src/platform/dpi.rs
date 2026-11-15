//! DPI map and coordinate types.
//!
//! Ink and snapshot coordinates are physical pixels; DIPs exist only for
//! toolbar/options layout. Windows in this process are per-monitor DPI aware
//! v2, so window rectangles and pointer positions are already physical pixels
//! and never virtualized.
//!
//! Conversions are computed in `f64` and only stored as `f32` in DIP types. For
//! every screen coordinate a window system can realistically produce
//! (|coordinate| <= 1,000,000 physical pixels) the physical -> DIP -> physical
//! roundtrip is exact at all Windows DPI values.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::GetDpiForWindow;

/// A point in physical screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

impl PhysicalPoint {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A rectangle in physical screen pixels (`right`/`bottom` exclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PhysicalRect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn from_xywh(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self::new(x, y, x + width, y + height)
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn center(&self) -> PhysicalPoint {
        PhysicalPoint::new(self.left + self.width() / 2, self.top + self.height() / 2)
    }
}

/// A point in device-independent pixels (1/96 inch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DipPoint {
    pub x: f32,
    pub y: f32,
}

impl DipPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A rectangle in device-independent pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DipRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl DipRect {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// Window/display scale expressed as a DPI value (96 = 100%).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpiMap {
    dpi: u32,
}

impl DpiMap {
    /// Builds a map from a DPI value. Zero or invalid values fall back to 96;
    /// Windows reports 0 for `GetDpiForWindow` only when the handle is invalid.
    pub const fn from_dpi(dpi: u32) -> Self {
        Self {
            dpi: if dpi == 0 { 96 } else { dpi },
        }
    }

    /// Reads the live per-window DPI (physical pixels per 96 DIPs).
    pub fn from_window(hwnd: HWND) -> Self {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        Self::from_dpi(dpi)
    }

    pub const fn dpi(&self) -> u32 {
        self.dpi
    }

    /// Scale factor where 1.0 is 96 DPI (e.g. 1.5 at 150%).
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    pub fn physical_to_dip(&self, point: PhysicalPoint) -> DipPoint {
        let scale = 96.0 / f64::from(self.dpi);
        DipPoint {
            x: (f64::from(point.x) * scale) as f32,
            y: (f64::from(point.y) * scale) as f32,
        }
    }

    pub fn dip_to_physical(&self, point: DipPoint) -> PhysicalPoint {
        let scale = f64::from(self.dpi) / 96.0;
        PhysicalPoint {
            x: (f64::from(point.x) * scale).round() as i32,
            y: (f64::from(point.y) * scale).round() as i32,
        }
    }

    pub fn physical_rect_to_dip(&self, rect: PhysicalRect) -> DipRect {
        let top_left = self.physical_to_dip(PhysicalPoint::new(rect.left, rect.top));
        let bottom_right = self.physical_to_dip(PhysicalPoint::new(rect.right, rect.bottom));
        DipRect {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        }
    }

    pub fn dip_rect_to_physical(&self, rect: DipRect) -> PhysicalRect {
        let top_left = self.dip_to_physical(DipPoint::new(rect.left, rect.top));
        let bottom_right = self.dip_to_physical(DipPoint::new(rect.right, rect.bottom));
        PhysicalRect::new(top_left.x, top_left.y, bottom_right.x, bottom_right.y)
    }
}
