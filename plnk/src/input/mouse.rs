//! Mouse fallback for the ink overlay.
//!
//! Windows promotes pen and touch contacts to `WM_MOUSE*` messages only when
//! the window does not handle `WM_POINTER` itself; the overlay handles the
//! pointer messages, so these samples describe a physical mouse (the only
//! verified input device on the current machine). A mouse has no pressure and
//! no coalesced history — each message is exactly one sample, and the
//! timestamp is the monotonic clock reading taken when the message is
//! dispatched.

use std::time::Duration;

use crate::ink::geometry::PhysicalPointF;
use crate::ink::model::PenPoint;

/// Builds full-pressure [`PenPoint`]s with clamped non-decreasing timestamps.
#[derive(Debug, Default)]
pub struct MouseNormalizer {
    last_timestamp: Option<Duration>,
}

impl MouseNormalizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// One mouse sample in physical screen pixels. Pressure is always 1.0
    /// (pressureless devices draw at the configured fixed width).
    pub fn sample(&mut self, position: PhysicalPointF, now: Duration) -> PenPoint {
        let timestamp = match self.last_timestamp {
            Some(last) if now < last => last,
            _ => now,
        };
        self.last_timestamp = Some(timestamp);
        PenPoint::new(position, 1.0, timestamp)
    }

    pub fn last_timestamp(&self) -> Option<Duration> {
        self.last_timestamp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_samples_are_monotonic_and_full_pressure() {
        let mut normalizer = MouseNormalizer::new();
        let first = normalizer.sample(PhysicalPointF::new(1.0, 2.0), Duration::from_millis(10));
        assert!((first.pressure - 1.0).abs() < f32::EPSILON);
        assert_eq!(first.timestamp, Duration::from_millis(10));
        let backwards = normalizer.sample(PhysicalPointF::new(3.0, 4.0), Duration::from_millis(5));
        assert_eq!(backwards.timestamp, Duration::from_millis(10));
        let forward = normalizer.sample(PhysicalPointF::new(5.0, 6.0), Duration::from_millis(11));
        assert_eq!(forward.timestamp, Duration::from_millis(11));
        assert_eq!(normalizer.last_timestamp(), Some(Duration::from_millis(11)));
    }
}
