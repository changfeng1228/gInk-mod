//! Native graphics layer: Direct2D device and DirectComposition window
//! visuals (plan §6.3). All objects are UI-thread owned.

pub mod composition;
pub mod device;
pub mod ink_surface;

pub use composition::Easing;
pub use device::{CompositionFrameStats, GfxError, GraphicsDevice};
pub use ink_surface::{InkSurface, PresentStats};

/// Straight (non-premultiplied) 8-bit color used at the API boundary.
///
/// Direct2D premultiplies when drawing into the premultiplied composition
/// target, so callers never deal with premultiplication themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}
