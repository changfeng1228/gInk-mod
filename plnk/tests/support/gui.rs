//! Safety gate and shared facilities for the desktop acceptance tests.
//!
//! Every test that takes over the real pointer goes through [`InputGate`]. The
//! gate exists because these tests move the user's pointer: without it an agent
//! running the default-off desktop set would steal input from whatever the user
//! is doing. A refused gate means SKIP, never PASS.
//!
//! The runtime that launches the command already yields when the user has just
//! used the machine (`RIVET_CU_YIELD_MS`, default 1200ms). That guard covers the
//! instant the command starts; this gate covers every second the test runs
//! afterwards, which a single desktop test can span for over ten seconds.

// This module is compiled into every test binary that includes it, but each
// binary uses only the subset it needs (the gate self-tests use `for_test`,
// the desktop tests use `production`). Allowing dead code here is what makes a
// single shared module possible instead of duplicating it per target.
#![allow(dead_code)]

use std::time::Duration;

use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

/// How long the user must have been idle before the harness may take over.
/// Short enough to keep an unattended run responsive, long enough that a
/// keystroke in progress is never interrupted.
pub const IDLE_THRESHOLD: Duration = Duration::from_secs(2);

/// Time since the last user input, system-wide.
pub fn user_idle_time() -> Duration {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    let ok = unsafe { GetLastInputInfo(&mut info) };
    if !ok.as_bool() {
        // A failed query must never be read as "idle": that would let the
        // harness inject input while the user works.
        return Duration::ZERO;
    }
    let now = unsafe { GetTickCount() };
    Duration::from_millis(u64::from(now.wrapping_sub(info.dwTime)))
}

/// Decides whether the harness may take over the pointer and keyboard.
pub struct InputGate {
    idle_threshold: Duration,
    /// `None` measures the real system; `Some` is a test override.
    forced_idle: Option<Duration>,
}

impl InputGate {
    /// The production gate: measures the real system idle time.
    pub fn production() -> Self {
        Self {
            idle_threshold: IDLE_THRESHOLD,
            forced_idle: None,
        }
    }

    /// A gate with a fixed idle time, for testing the gate itself.
    pub fn for_test(idle: Duration) -> Self {
        Self {
            idle_threshold: IDLE_THRESHOLD,
            forced_idle: Some(idle),
        }
    }

    /// Idle time this gate observes.
    pub fn idle(&self) -> Duration {
        self.forced_idle.unwrap_or_else(user_idle_time)
    }

    /// True when input may be injected.
    pub fn input_allowed(&self) -> bool {
        self.idle() >= self.idle_threshold
    }
}

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

/// Owns the desktop for the duration of one test.
///
/// `Drop` restores the pointer position, so a panic mid-test cannot leave the
/// user's pointer somewhere unexpected. Acquiring a guard is the only way the
/// harness is allowed to take over input (safety invariant S2).
pub struct DesktopGuard {
    saved: POINT,
}

impl DesktopGuard {
    /// Acquires the desktop when the user is idle.
    ///
    /// `None` means the caller must report SKIP rather than fail.
    pub fn acquire(gate: &InputGate) -> Option<Self> {
        if !gate.input_allowed() {
            return None;
        }
        Some(Self::acquire_unchecked())
    }

    /// Acquires without consulting the gate. Only for the guard's own tests;
    /// production tests must use [`Self::acquire`].
    pub fn acquire_for_test() -> Self {
        Self::acquire_unchecked()
    }

    fn acquire_unchecked() -> Self {
        let mut saved = POINT::default();
        if unsafe { GetCursorPos(&mut saved) }.is_err() {
            saved = POINT { x: 0, y: 0 };
        }
        Self { saved }
    }
}

impl Drop for DesktopGuard {
    fn drop(&mut self) {
        // Best effort: a failure here must not mask the test result.
        let _ = unsafe { SetCursorPos(self.saved.x, self.saved.y) };
    }
}

/// Result of one desktop test, in a form the PowerShell driver can parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed(String),
    Skipped(String),
}

impl Outcome {
    /// Whether this outcome counts as a success. A skip does not.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Passed)
    }

    /// One line with a stable prefix, so the driver can split the three cases
    /// without parsing Cargo's own output format.
    pub fn line(&self) -> String {
        match self {
            Self::Passed => "PLNK_PASS:".to_owned(),
            Self::Failed(reason) => format!("PLNK_FAIL: {reason}"),
            Self::Skipped(reason) => format!("PLNK_SKIP: {reason}"),
        }
    }
}

/// Records a deliberate skip and prints the marker line.
pub fn skip(reason: &str) -> Outcome {
    let outcome = Outcome::Skipped(reason.to_owned());
    println!("{}", outcome.line());
    outcome
}
