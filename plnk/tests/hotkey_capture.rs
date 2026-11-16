//! Acceptance tests for capturing a hotkey from the keyboard.
//!
//! The settings window lets the user click a hotkey box and press the
//! combination instead of typing it. The window itself is Win32 glue, so the
//! decision table lives in `options::capture_from_vk` and is pinned here:
//! one virtual-key code plus the four modifier states must map to exactly one
//! outcome -- a complete combination, "clear this row", "keep waiting for the
//! main key", or "this key cannot be bound".
//!
//! The expected values below are the Win32 virtual-key codes the shell
//! delivers (VK_A = 0x41, VK_0 = 0x30, VK_F5 = 0x74, VK_NUMPAD3 = 0x63,
//! VK_BACK = 0x08, VK_DELETE = 0x2E, VK_CONTROL = 0x11).

use plnk::options::{CaptureOutcome, capture_from_vk};
use plnk::settings::{Hotkey, HotkeyKey};

fn key(control: bool, alt: bool, shift: bool, win: bool, key: HotkeyKey) -> Hotkey {
    Hotkey {
        control,
        alt,
        shift,
        win,
        key: Some(key),
    }
}

#[test]
fn a_bare_letter_is_captured() {
    assert_eq!(
        capture_from_vk(0x45, false, false, false, false), // 'E'
        CaptureOutcome::Key(key(false, false, false, false, HotkeyKey::Letter('E')))
    );
}

#[test]
fn modifiers_held_while_pressing_a_letter_are_part_of_the_combination() {
    assert_eq!(
        capture_from_vk(0x5A, true, true, false, false), // Ctrl+Alt+Z
        CaptureOutcome::Key(key(true, true, false, false, HotkeyKey::Letter('Z')))
    );
    assert_eq!(
        capture_from_vk(0x47, false, false, true, true), // Shift+Win+G
        CaptureOutcome::Key(key(false, false, true, true, HotkeyKey::Letter('G')))
    );
}

#[test]
fn digits_numpad_digits_and_function_keys_are_bindable() {
    assert_eq!(
        capture_from_vk(0x30, false, false, false, false), // '0'
        CaptureOutcome::Key(key(false, false, false, false, HotkeyKey::Digit(0)))
    );
    assert_eq!(
        capture_from_vk(0x63, false, false, false, false), // VK_NUMPAD3
        CaptureOutcome::Key(key(false, false, false, false, HotkeyKey::NumPad(3)))
    );
    assert_eq!(
        capture_from_vk(0x74, false, false, false, false), // VK_F5
        CaptureOutcome::Key(key(false, false, false, false, HotkeyKey::Function(5)))
    );
    assert_eq!(
        capture_from_vk(0x7B, true, false, false, false), // VK_F12
        CaptureOutcome::Key(key(true, false, false, false, HotkeyKey::Function(12)))
    );
}

#[test]
fn backspace_and_delete_clear_the_row() {
    assert_eq!(
        capture_from_vk(0x08, false, false, false, false), // VK_BACK
        CaptureOutcome::Clear
    );
    assert_eq!(
        capture_from_vk(0x2E, true, false, false, false), // VK_DELETE (modifiers ignored)
        CaptureOutcome::Clear
    );
}

#[test]
fn a_lone_modifier_keeps_waiting_for_the_main_key() {
    for vk in [
        0x10u16, // VK_SHIFT
        0x11,    // VK_CONTROL
        0x12,    // VK_MENU (Alt)
        0x5B,    // VK_LWIN
        0x5C,    // VK_RWIN
        0xA0,    // VK_LSHIFT
        0xA2,    // VK_LCONTROL
        0xA4,    // VK_LMENU
    ] {
        assert_eq!(
            capture_from_vk(vk, true, false, false, false),
            CaptureOutcome::ModifierOnly,
            "virtual key 0x{vk:02X} is a modifier and must not end the capture"
        );
    }
}

#[test]
fn keys_outside_the_model_are_reported_not_swallowed() {
    for vk in [
        0x25u16, // VK_LEFT
        0x20,    // VK_SPACE
        0x0D,    // VK_RETURN
        0x1B,    // VK_ESCAPE
        0x6A,    // VK_MULTIPLY (not a numpad digit)
    ] {
        assert_eq!(
            capture_from_vk(vk, false, false, false, false),
            CaptureOutcome::Unsupported(vk),
            "virtual key 0x{vk:02X} has no HotkeyKey and must be reported"
        );
    }
}

#[test]
fn a_captured_bare_key_still_fails_the_system_wide_rule() {
    // Capturing must not bypass the existing validation: the global hotkey row
    // requires a modifier, and that rule lives in `apply_hotkey`.
    let mut settings = plnk::settings::Settings::defaults();
    let CaptureOutcome::Key(captured) = capture_from_vk(0x47, false, false, false, false) else {
        panic!("a letter must produce a combination");
    };
    assert!(
        plnk::options::apply_hotkey(
            &mut settings,
            plnk::hotkeys::Command::ToggleInk,
            captured
        )
        .is_err(),
        "a bare key must still be rejected for the system-wide row"
    );
    assert!(
        plnk::options::apply_hotkey(&mut settings, plnk::hotkeys::Command::SelectPen(0), captured)
            .is_ok(),
        "the same bare key is legal for a pen row"
    );
}
