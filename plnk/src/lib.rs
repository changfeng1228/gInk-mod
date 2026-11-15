//! plnk: a native reimplementation of gInk.
//!
//! The settings layer is the first product code of the rewrite; the platform
//! and graphics modules add the native windows, DPI mapping and the
//! Direct2D/DirectComposition layer validated by the graphics decision gate.

pub mod error;
pub mod gfx;
pub mod platform;
pub mod settings;
pub mod settings_store;
