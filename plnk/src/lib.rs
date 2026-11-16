//! plnk: a native reimplementation of gInk.
//!
//! The settings layer is the first product code of the rewrite; the platform
//! and graphics modules add the native windows, DPI mapping and the
//! Direct2D/DirectComposition layer validated by the graphics decision gate.
//! The application shell adds the single instance, tray, hotkey routing and
//! bounded logging that every later task runs inside.

pub mod app;
pub mod assets;
pub mod error;
pub mod gfx;
pub mod hotkeys;
pub mod ink;
pub mod input;
pub mod platform;
pub mod settings;
pub mod settings_store;
pub mod telemetry;
pub mod tray;
pub mod ui;
