//! Native platform layer: window plumbing, DPI mapping and the monotonic
//! clock (plan §7).

pub mod cursor;
pub mod dpi;
pub mod window;

pub use cursor::{CanvasCursors, PenCursorColor, default_class_cursor};
pub use dpi::{DipPoint, DipRect, DpiMap, PhysicalPoint, PhysicalRect};
pub use window::{
    CallbackCounts, MonotonicClock, OverlayWindow, ToolbarWindow, current_monitor_rect,
    ensure_process_dpi_awareness,
};
