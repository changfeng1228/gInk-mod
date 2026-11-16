//! Self-tests for the desktop acceptance gate.
//!
//! The gate is the only thing standing between an unattended agent run and the
//! user's pointer, so it is tested directly rather than only through the
//! desktop tests that consume it.

mod support;

use std::time::Duration;

#[test]
fn idle_time_is_always_finite() {
    let idle = support::gui::user_idle_time();
    assert!(
        idle < Duration::from_secs(60 * 60 * 24),
        "idle time must be finite, got {idle:?}"
    );
}

#[test]
fn input_is_refused_when_the_user_is_active() {
    let gate = support::gui::InputGate::for_test(Duration::ZERO);
    assert!(!gate.input_allowed(), "zero idle time must refuse input");
}

#[test]
fn input_is_allowed_when_the_user_has_been_idle() {
    let gate = support::gui::InputGate::for_test(Duration::from_secs(60));
    assert!(gate.input_allowed(), "60s idle must permit input");
}

#[test]
fn desktop_guard_restores_the_pointer_even_on_panic() {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

    let mut before = POINT::default();
    unsafe { GetCursorPos(&mut before) }.expect("read the pointer before the test");

    let moved = std::panic::catch_unwind(|| {
        let _guard = support::gui::DesktopGuard::acquire_for_test();
        unsafe { SetCursorPos(before.x.saturating_add(200), before.y.saturating_add(200)) }
            .expect("move the pointer");
        panic!("intentional");
    });
    assert!(moved.is_err(), "the test body must have panicked");

    let mut after = POINT::default();
    unsafe { GetCursorPos(&mut after) }.expect("read the pointer after the test");
    assert_eq!(
        (after.x, after.y),
        (before.x, before.y),
        "the guard must restore the pointer after a panic"
    );
}

#[test]
fn skip_is_never_counted_as_pass() {
    let outcome = support::gui::skip("user is active");
    assert!(!outcome.is_pass(), "SKIP must never count as PASS");
    assert!(outcome.line().starts_with("PLNK_SKIP:"));
}

#[test]
fn pass_and_fail_have_distinct_markers() {
    assert!(
        support::gui::Outcome::Passed
            .line()
            .starts_with("PLNK_PASS:")
    );
    assert!(
        support::gui::Outcome::Failed("boom".to_owned())
            .line()
            .starts_with("PLNK_FAIL:")
    );
}
