//! Ink: physical-pixel strokes, smoothing geometry, dirty regions, command
//! history, the active-stroke controller and the Direct2D stroke renderer.
//!
//! The model layer (model/geometry/dirty/history) has no Win32, Direct2D or
//! window dependencies and stays headless-testable. [`controller`] adds the
//! live active-stroke lifecycle over the model; [`renderer`] is the only part
//! that touches Direct2D and turns smoothed samples into a filled ribbon.
//! [`Stroke::smoothed_samples`] feeds the renderer; [`DirtyRegion`]s bound both
//! the active-layer updates and the static-cache repaints.

pub mod controller;
pub mod dirty;
pub mod geometry;
pub mod history;
pub mod model;
pub mod renderer;

pub use controller::InkController;
pub use dirty::{DirtyRect, DirtyRegion};
pub use geometry::{PhysicalPointF, StrokeSample};
pub use model::{
    DEFAULT_STROKE_WIDTH, Edit, HIGHLIGHTER_MAX_ALPHA, InkDocument, MAX_ERASER_RADIUS,
    MAX_STROKE_WIDTH, MIN_PRESSURE_WIDTH_RATIO, MIN_STROKE_WIDTH, PenPoint, PenStyle, Stroke,
    StrokeId,
};
pub use renderer::StrokeRenderer;
