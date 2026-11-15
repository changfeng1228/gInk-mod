//! Toolbar slide animation bookkeeping.
//!
//! Only the start, end, duration and easing are handed to DirectComposition;
//! DWM evaluates the curve on every composition frame, so the application gets
//! no per-frame callbacks (plan §6.1/§6.2). The animator keeps the same
//! parameters in model space with the monotonic clock, which is what makes a
//! retarget possible: when the user toggles the toolbar mid-slide, the new
//! animation must start at the position the visual currently occupies, not at
//! the previously requested target.

use std::time::Duration;

use crate::gfx::composition::Easing;
use crate::platform::dpi::PhysicalPoint;

/// Legacy slide-in / dock duration (`ButtonsEntering = 1/-1`, 300 ms).
pub const ENTER_DURATION: Duration = Duration::from_millis(300);

/// Legacy fast exit duration (`ButtonsEntering = -9`, 160 ms).
pub const FAST_EXIT_DURATION: Duration = Duration::from_millis(160);

/// One slide: the exact parameters a DirectComposition animation needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlidePlan {
    pub from: PhysicalPoint,
    pub to: PhysicalPoint,
    pub duration: Duration,
    pub easing: Easing,
}

#[derive(Clone, Copy, Debug)]
struct Slide {
    plan: SlidePlan,
    started_at: Duration,
}

/// Tracks the toolbar's current visual position and its in-flight slide.
///
/// `now` values are [`crate::platform::window::MonotonicClock::now`] readings,
/// injected so the retargeting behaviour is testable without a desktop.
#[derive(Clone, Copy, Debug)]
pub struct ToolbarAnimator {
    slide: Option<Slide>,
    settled: PhysicalPoint,
}

impl ToolbarAnimator {
    pub fn new(initial: PhysicalPoint) -> Self {
        Self {
            slide: None,
            settled: initial,
        }
    }

    /// Slide in to `to` in [`ENTER_DURATION`] (legacy `ButtonsEntering = 1`).
    pub fn enter(&mut self, now: Duration, to: PhysicalPoint) -> SlidePlan {
        self.retarget(now, to, ENTER_DURATION)
    }

    /// Fast exit to `to` in [`FAST_EXIT_DURATION`] (legacy `-9`, Esc).
    pub fn fast_exit(&mut self, now: Duration, to: PhysicalPoint) -> SlidePlan {
        self.retarget(now, to, FAST_EXIT_DURATION)
    }

    /// Dock or undock slide in [`ENTER_DURATION`] (legacy `-1`/`1`).
    pub fn dock_to(&mut self, now: Duration, to: PhysicalPoint) -> SlidePlan {
        self.retarget(now, to, ENTER_DURATION)
    }

    /// Current visual position: the in-flight slide interpolated with the same
    /// curve DWM is evaluating, or the settled position when idle.
    pub fn position_at(&self, now: Duration) -> PhysicalPoint {
        let Some(slide) = self.slide else {
            return self.settled;
        };
        let duration = slide.plan.duration.as_secs_f32();
        let progress = if duration <= 0.0 {
            1.0
        } else {
            (now.saturating_sub(slide.started_at).as_secs_f32() / duration).clamp(0.0, 1.0)
        };
        interpolate(slide.plan, progress)
    }

    /// Whether a slide is still moving at `now`.
    pub fn is_active(&self, now: Duration) -> bool {
        self.slide
            .is_some_and(|slide| now.saturating_sub(slide.started_at) < slide.plan.duration)
    }

    /// Jumps the model position without animating (used when the window is
    /// hidden or when its frame is re-anchored to a different window rect).
    pub fn snap_to(&mut self, position: PhysicalPoint) {
        self.slide = None;
        self.settled = position;
    }

    /// Declares the in-flight slide finished and stores its target.
    pub fn finish(&mut self) {
        if let Some(slide) = self.slide.take() {
            self.settled = slide.plan.to;
        }
    }

    fn retarget(&mut self, now: Duration, to: PhysicalPoint, duration: Duration) -> SlidePlan {
        let plan = SlidePlan {
            from: self.position_at(now),
            to,
            duration,
            easing: Easing::SmoothStep,
        };
        // The settled position tracks the start of the newest slide; once it
        // finishes `finish()` moves it to the target.
        self.settled = plan.from;
        self.slide = Some(Slide {
            plan,
            started_at: now,
        });
        plan
    }
}

fn interpolate(plan: SlidePlan, progress: f32) -> PhysicalPoint {
    let eased = f64::from(plan.easing.value_at(progress));
    let x = f64::from(plan.from.x) + f64::from(plan.to.x - plan.from.x) * eased;
    let y = f64::from(plan.from.y) + f64::from(plan.to.y - plan.from.y) * eased;
    PhysicalPoint::new(x.round() as i32, y.round() as i32)
}
