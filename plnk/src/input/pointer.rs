//! `WM_POINTER` normalization for pen, touch and mouse pointers.
//!
//! One `WM_POINTER` message can stand for several coalesced input samples: the
//! digitizer produces points faster than the message queue delivers them. The
//! history APIs (`GetPointerPenInfoHistory`, `GetPointerTouchInfoHistory`,
//! `GetPointerInfoHistory`) return those samples **in reverse chronological
//! order**, newest first. [`PointerNormalizer::normalize`] reverses them so the
//! stroke follows the physical path, converts the QPC `PerformanceCount` into a
//! monotonic [`Duration`], and maps pressure and the eraser end.
//!
//! The per-pointer history is used instead of `GetPointerFrameInfoHistory`
//! because a drawing stroke belongs to exactly one pointer; a frame can also
//! contain other fingers that must not leak into the stroke. Frame history has
//! the same reverse-chronological contract, so the ordering logic is identical.

use std::time::Duration;

use windows::Win32::UI::Input::Pointer::{
    GetPointerInfo, GetPointerInfoHistory, GetPointerPenInfo, GetPointerPenInfoHistory,
    GetPointerTouchInfo, GetPointerTouchInfoHistory, GetPointerType, POINTER_INFO,
    POINTER_PEN_INFO, POINTER_TOUCH_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, POINTER_INPUT_TYPE, PT_MOUSE, PT_PEN,
    PT_TOUCH, TOUCH_MASK_PRESSURE,
};

use crate::ink::geometry::PhysicalPointF;
use crate::ink::model::{PenPoint, sanitize_pressure};
use crate::platform::window::MonotonicClock;

/// The input device family behind a pointer id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerKind {
    Pen,
    Touch,
    Mouse,
    /// Unknown, or no pointer message has been normalized yet.
    #[default]
    Other,
}

/// One raw pointer sample before timestamp conversion.
///
/// `performance_count` is `POINTER_INFO::PerformanceCount` (QPC ticks) exactly
/// as Windows reports it; `pressure` is `None` when the device does not expose
/// pressure. Tests construct these directly to exercise the pure normalization
/// path without a digitizer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSample {
    pub x: i32,
    pub y: i32,
    pub pressure: Option<f32>,
    pub performance_count: i64,
}

impl PointerSample {
    pub const fn new(x: i32, y: i32, pressure: Option<f32>, performance_count: i64) -> Self {
        Self {
            x,
            y,
            pressure,
            performance_count,
        }
    }

    fn from_info(info: &POINTER_INFO) -> Self {
        Self {
            x: info.ptPixelLocation.x,
            y: info.ptPixelLocation.y,
            pressure: None,
            performance_count: i64::try_from(info.PerformanceCount).unwrap_or(i64::MAX),
        }
    }

    fn from_pen(info: &POINTER_PEN_INFO) -> Self {
        Self {
            x: info.pointerInfo.ptPixelLocation.x,
            y: info.pointerInfo.ptPixelLocation.y,
            pressure: (info.penMask & PEN_MASK_PRESSURE != 0)
                .then_some(f32::from(info.pressure as u16) / 1024.0),
            performance_count: i64::try_from(info.pointerInfo.PerformanceCount).unwrap_or(i64::MAX),
        }
    }

    fn from_touch(info: &POINTER_TOUCH_INFO) -> Self {
        Self {
            x: info.pointerInfo.ptPixelLocation.x,
            y: info.pointerInfo.ptPixelLocation.y,
            pressure: (info.touchMask & TOUCH_MASK_PRESSURE != 0)
                .then_some(f32::from(info.pressure as u16) / 1024.0),
            performance_count: i64::try_from(info.pointerInfo.PerformanceCount).unwrap_or(i64::MAX),
        }
    }
}

/// Converts `WM_POINTER` history into ordered, monotonic [`PenPoint`]s.
///
/// Timestamps are converted from the raw QPC `PerformanceCount` into the
/// process [`MonotonicClock`] base, so a stroke sample can be compared with
/// the presentation timestamp of the frame that submits it. The last seen
/// pointer kind, pressure capability and eraser flag are exposed for the tool
/// layer.
#[derive(Debug)]
pub struct PointerNormalizer {
    last_timestamp: Duration,
    kind: PointerKind,
    pressure_supported: bool,
    eraser: bool,
}

impl Default for PointerNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl PointerNormalizer {
    pub fn new() -> Self {
        Self {
            last_timestamp: Duration::ZERO,
            kind: PointerKind::Other,
            pressure_supported: false,
            eraser: false,
        }
    }

    /// Normalizes one `WM_POINTER` message, expanding its coalesced history.
    ///
    /// Returns the samples oldest first. The message's own `POINTER_INFO` is
    /// used when the history API reports nothing (for example a device that
    /// coalesces no input, or a synthetic message without a live pointer).
    pub fn next(&mut self, info: POINTER_INFO) -> Vec<PenPoint> {
        let kind = pointer_kind(info.pointerId);
        self.kind = kind;
        let newest_first = match kind {
            PointerKind::Pen => self.pen_history(info.pointerId),
            PointerKind::Touch => self.touch_history(info.pointerId),
            PointerKind::Mouse => self.info_history(info.pointerId),
            PointerKind::Other => Vec::new(),
        };
        if newest_first.is_empty() {
            self.normalize(&[PointerSample::from_info(&info)])
        } else {
            self.normalize(&newest_first)
        }
    }

    /// Pure conversion of one coalesced frame, in the Win32 order (newest
    /// first), into chronological [`PenPoint`]s.
    pub fn normalize(&mut self, newest_first: &[PointerSample]) -> Vec<PenPoint> {
        let mut points = Vec::with_capacity(newest_first.len());
        for sample in newest_first.iter().rev() {
            let mut timestamp = MonotonicClock::from_performance_count(sample.performance_count);
            if timestamp < self.last_timestamp {
                // Device counters wrap and histories can overlap; the stroke
                // contract requires non-decreasing timestamps.
                timestamp = self.last_timestamp;
            }
            self.last_timestamp = timestamp;
            // `Stroke::push` sanitizes pressure again; doing it here keeps the
            // normalizer's output contract independent of the model.
            let pressure = sanitize_pressure(sample.pressure.unwrap_or(1.0));
            points.push(PenPoint::new(
                PhysicalPointF::new(sample.x as f32, sample.y as f32),
                pressure,
                timestamp,
            ));
        }
        points
    }

    pub fn kind(&self) -> PointerKind {
        self.kind
    }

    /// True when the last pen contact reported a pressure channel.
    pub fn pressure_supported(&self) -> bool {
        self.pressure_supported
    }

    /// True when the last pen contact was the inverted/eraser end.
    pub fn is_eraser(&self) -> bool {
        self.eraser
    }

    /// Timestamp of the most recent normalized sample.
    pub fn last_timestamp(&self) -> Duration {
        self.last_timestamp
    }

    fn pen_history(&mut self, pointer_id: u32) -> Vec<PointerSample> {
        let mut count = 0u32;
        if unsafe { GetPointerPenInfoHistory(pointer_id, &mut count, None) }.is_err() || count == 0
        {
            // No coalesced history: fall back to the single current pen info so
            // pressure and the eraser end are still known.
            if let Ok(pen) = unsafe {
                let mut pen = POINTER_PEN_INFO::default();
                GetPointerPenInfo(pointer_id, &mut pen).map(|()| pen)
            } {
                self.record_pen_flags(&pen);
                return vec![PointerSample::from_pen(&pen)];
            }
            return Vec::new();
        }
        let mut entries = vec![POINTER_PEN_INFO::default(); count as usize];
        if unsafe { GetPointerPenInfoHistory(pointer_id, &mut count, Some(entries.as_mut_ptr())) }
            .is_err()
        {
            return Vec::new();
        }
        entries.truncate(count.min(entries.len() as u32) as usize);
        if let Some(first) = entries.first() {
            self.record_pen_flags(first);
        }
        entries.iter().map(PointerSample::from_pen).collect()
    }

    fn touch_history(&mut self, pointer_id: u32) -> Vec<PointerSample> {
        let mut count = 0u32;
        if unsafe { GetPointerTouchInfoHistory(pointer_id, &mut count, None) }.is_err()
            || count == 0
        {
            if let Ok(touch) = unsafe {
                let mut touch = POINTER_TOUCH_INFO::default();
                GetPointerTouchInfo(pointer_id, &mut touch).map(|()| touch)
            } {
                return vec![PointerSample::from_touch(&touch)];
            }
            return Vec::new();
        }
        let mut entries = vec![POINTER_TOUCH_INFO::default(); count as usize];
        if unsafe { GetPointerTouchInfoHistory(pointer_id, &mut count, Some(entries.as_mut_ptr())) }
            .is_err()
        {
            return Vec::new();
        }
        entries.truncate(count.min(entries.len() as u32) as usize);
        entries.iter().map(PointerSample::from_touch).collect()
    }

    fn info_history(&mut self, pointer_id: u32) -> Vec<PointerSample> {
        let mut count = 0u32;
        if unsafe { GetPointerInfoHistory(pointer_id, &mut count, None) }.is_err() || count == 0 {
            if let Ok(info) = unsafe {
                let mut info = POINTER_INFO::default();
                GetPointerInfo(pointer_id, &mut info).map(|()| info)
            } {
                return vec![PointerSample::from_info(&info)];
            }
            return Vec::new();
        }
        let mut entries = vec![POINTER_INFO::default(); count as usize];
        if unsafe { GetPointerInfoHistory(pointer_id, &mut count, Some(entries.as_mut_ptr())) }
            .is_err()
        {
            return Vec::new();
        }
        entries.truncate(count.min(entries.len() as u32) as usize);
        entries.iter().map(PointerSample::from_info).collect()
    }

    fn record_pen_flags(&mut self, pen: &POINTER_PEN_INFO) {
        self.pressure_supported = pen.penMask & PEN_MASK_PRESSURE != 0;
        self.eraser = pen.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0;
    }
}

fn pointer_kind(pointer_id: u32) -> PointerKind {
    let mut pointer_type = POINTER_INPUT_TYPE::default();
    if unsafe { GetPointerType(pointer_id, &mut pointer_type) }.is_err() {
        return PointerKind::Other;
    }
    if pointer_type == PT_PEN {
        PointerKind::Pen
    } else if pointer_type == PT_TOUCH {
        PointerKind::Touch
    } else if pointer_type == PT_MOUSE {
        PointerKind::Mouse
    } else {
        PointerKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    fn qpc_now() -> i64 {
        let mut counter = 0i64;
        unsafe { QueryPerformanceCounter(&mut counter) }.expect("QueryPerformanceCounter");
        counter
    }

    fn qpc_frequency() -> i64 {
        let mut frequency = 0i64;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.expect("QueryPerformanceFrequency");
        frequency.max(1)
    }

    #[test]
    fn normalize_reverses_newest_first_samples_and_clamps_backwards_time() {
        let mut normalizer = PointerNormalizer::new();
        let frequency = qpc_frequency();
        let millisecond = (frequency / 1000).max(1);
        // Establish the process clock before the samples (the clock epoch is
        // the first-use QPC reading, so leave a margin before the burst).
        let _ = MonotonicClock::now();
        std::thread::sleep(Duration::from_millis(4));
        let newest = qpc_now();
        // Win32 returns coalesced history newest first.
        let newest_first = [
            PointerSample::new(3, 0, None, newest),
            PointerSample::new(2, 0, None, newest - millisecond),
            PointerSample::new(1, 0, None, newest - 2 * millisecond),
        ];
        let points = normalizer.normalize(&newest_first);
        assert_eq!(
            points
                .iter()
                .map(|point| point.position.x)
                .collect::<Vec<_>>(),
            vec![1.0, 2.0, 3.0]
        );
        assert!(
            points
                .windows(2)
                .all(|pair| pair[0].timestamp <= pair[1].timestamp)
        );
        let span = points[2].timestamp - points[0].timestamp;
        assert!(
            span >= Duration::from_millis(1) && span <= Duration::from_millis(3),
            "two QPC milliseconds must stay inside the conversion tolerance, got {span:?}"
        );

        // A sample older than the last emitted timestamp is clamped, never
        // reordered.
        let stale = [PointerSample::new(0, 0, None, newest - 5 * millisecond)];
        let clamped = normalizer.normalize(&stale);
        assert_eq!(clamped[0].timestamp, points[2].timestamp);
    }
}
