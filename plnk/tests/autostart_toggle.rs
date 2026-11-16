//! The autostart row must read as a *state*, not as a push button.
//!
//! The row owns a Startup-folder shortcut; the user needs to see at a glance
//! whether it exists. A BS_PUSHBUTTON control answers BM_SETCHECK by ignoring
//! it, so the box looked identical whether autostart was on or off -- the exact
//! doubt this test exists to prevent. The style is exposed as a value and pinned
//! here: auto checkbox, no client border, still reachable with Tab.

use plnk::ui::options_window::autostart_toggle_style;
use windows::Win32::UI::WindowsAndMessaging::{
    BS_AUTOCHECKBOX, WS_BORDER, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

#[test]
fn autostart_control_is_an_auto_checkbox_without_a_client_border() {
    let style = autostart_toggle_style();

    assert_ne!(
        style & BS_AUTOCHECKBOX as u32,
        0,
        "the autostart control must be a BS_AUTOCHECKBOX, otherwise BM_SETCHECK draws nothing          and the user cannot tell whether autostart is on"
    );
    assert_eq!(
        style & WS_BORDER.0,
        0,
        "a checkbox paints its own box; a client border makes it look like the push button it          used to be"
    );
    assert_ne!(style & WS_TABSTOP.0, 0, "the toggle must stay reachable with Tab");
    assert_ne!(style & WS_CHILD.0, 0, "it is a child control of the dialog");
    assert_ne!(style & WS_VISIBLE.0, 0, "it must be visible when the dialog opens");
}
