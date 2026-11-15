//! Headless ink model: physical-pixel strokes, smoothing geometry, dirty
//! regions and command-based edit history.
//!
//! This layer has no Win32, Direct2D or window dependencies. Task 7 renders
//! [`Stroke::smoothed_samples`] into a cached surface and consumes
//! [`DirtyRegion`]s; Task 8 drives tools through [`Edit`] and
//! [`InkDocument::erase_at`]; pan and autoscroll arrive later as further
//! commands over the same stroke list.

pub mod dirty;
pub mod geometry;
pub mod history;
pub mod model;

pub use dirty::{DirtyRect, DirtyRegion};
pub use geometry::{PhysicalPointF, StrokeSample};
pub use model::{
    DEFAULT_STROKE_WIDTH, Edit, HIGHLIGHTER_MAX_ALPHA, InkDocument, MAX_ERASER_RADIUS,
    MAX_STROKE_WIDTH, MIN_PRESSURE_WIDTH_RATIO, MIN_STROKE_WIDTH, PenPoint, PenStyle, Stroke,
    StrokeId,
};
