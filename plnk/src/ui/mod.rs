//! Native user-interface layer: the toolbar (model, layout, icons, docking and
//! compositor-driven slide animation).

pub mod animation;
pub mod icons;
pub mod options_window;
pub mod pen_width;
pub mod toolbar;

pub use animation::{ENTER_DURATION, FAST_EXIT_DURATION, ToolbarAnimator};
pub use pen_width::{PenWidthModel, PenWidthPanel};
pub use toolbar::{ButtonId, ToolbarLayout, ToolbarModel, ToolbarWindow};
