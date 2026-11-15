//! Ink input normalization.
//!
//! Two paths feed the same [`PenPoint`](crate::ink::PenPoint) pipeline:
//!
//! * [`pointer`] consumes `WM_POINTER` messages (pen, touch and — when a device
//!   reports them — mouse pointers). It expands the coalesced history of one
//!   message with `GetPointerPenInfoHistory`/`GetPointerTouchInfoHistory` so a
//!   fast burst of samples is not reduced to the one position that happened to
//!   be current when the message was dispatched.
//! * [`mouse`] is the fallback for the plain mouse message path: Windows still
//!   routes `WM_MOUSE*` messages to the ink overlay for a physical mouse, and
//!   those messages carry no pressure and no coalesced history.
//!
//! Both normalizers emit physical-pixel [`PenPoint`](crate::ink::PenPoint)
//! values with non-decreasing timestamps; the controller and the stroke model
//! assume that contract.

pub mod mouse;
pub mod pointer;

pub use mouse::MouseNormalizer;
pub use pointer::{PointerKind, PointerNormalizer, PointerSample};
