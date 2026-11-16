//! Options window behaviour: hotkey rebinding, autostart, and atomic save.
//!
//! These are headless tests of the model the window drives. The window itself
//! is a thin Win32 shell over `OptionsModel`, so testing the model covers every
//! rule the user can observe: which hotkeys exist, what a rebind does, whether
//! a conflicting binding is rejected, and whether closing saves.

use plnk::hotkeys::{Command, HotkeyMap};
use plnk::settings::{Hotkey, HotkeyKey, Settings};

/// Every hotkey the user can rebind in the options window, in display order.
/// Mirrors the legacy `Hotkey_*` INI keys.
#[test]
fn options_expose_every_rebindable_hotkey() {
    let settings = Settings::defaults();
    let entries = plnk::options::rebindable_hotkeys(&settings);
    let commands: Vec<Command> = entries.iter().map(|entry| entry.command).collect();

    for required in [
        Command::ToggleInk,
        Command::Eraser,
        Command::InkVisible,
        Command::Pointer,
        Command::Pan,
        Command::Undo,
        Command::Redo,
        Command::Snapshot,
        Command::SnapPin,
        Command::Clear,
    ] {
        assert!(
            commands.contains(&required),
            "options window must let the user rebind {required:?}"
        );
    }
    // Ten pens are rebindable too.
    for index in 0..10 {
        assert!(
            commands.contains(&Command::SelectPen(index)),
            "options window must let the user rebind pen {index}"
        );
    }
}

/// Rebinding writes through to `Settings`, which is what `save` persists.
#[test]
fn rebinding_a_hotkey_updates_the_settings_value() {
    let mut settings = Settings::defaults();
    let new_hotkey = Hotkey {
        control: true,
        alt: true,
        shift: false,
        win: false,
        key: Some(HotkeyKey::Letter('K')),
    };
    plnk::options::apply_hotkey(&mut settings, Command::ToggleInk, new_hotkey).unwrap();
    assert_eq!(settings.hotkeys.global, new_hotkey);

    // The value survives a round trip through the legacy textual form.
    let text = plnk::hotkeys::format_hotkey(&settings.hotkeys.global);
    assert_eq!(Hotkey::parse(&text), Some(new_hotkey));
}

/// A rebind that collides with another configured hotkey is rejected and
/// leaves both values untouched, so the window can flag the conflict instead of
/// silently creating two identical system-wide registrations.
#[test]
fn conflicting_rebind_is_rejected_and_changes_nothing() {
    let mut settings = Settings::defaults();
    let original_global = settings.hotkeys.global;
    let original_eraser = settings.hotkeys.eraser;

    // Give the eraser the exact combination the global hotkey already uses.
    let clash = original_global;
    let result = plnk::options::apply_hotkey(&mut settings, Command::Eraser, clash);

    assert!(result.is_err(), "a duplicate combination must be rejected");
    assert_eq!(
        settings.hotkeys.global, original_global,
        "the rejected change must not touch the existing binding"
    );
    assert_eq!(
        settings.hotkeys.eraser, original_eraser,
        "the rejected change must not be applied to the target"
    );
}

/// Clearing a hotkey to `None` is allowed and is not a conflict.
#[test]
fn clearing_a_hotkey_to_none_is_allowed() {
    let mut settings = Settings::defaults();
    plnk::options::apply_hotkey(&mut settings, Command::Pan, Hotkey::default()).unwrap();
    assert!(settings.hotkeys.pan.key.is_none());
}

/// Autostart state is written to the per-user Startup folder, not the registry,
/// so no administrator rights are needed and uninstalling is a file delete.
#[test]
fn autostart_toggles_a_startup_shortcut() {
    let dir = std::env::temp_dir().join(format!("plnk-autostart-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("plnk.lnk");

    assert!(!plnk::startup::is_enabled_at(&link), "starts disabled");

    plnk::startup::set_enabled_at(&link, true, std::path::Path::new(r"C:/plnk/plnk.exe"))
        .expect("enabling autostart must succeed");
    assert!(link.is_file(), "enabling must create the shortcut");
    assert!(plnk::startup::is_enabled_at(&link));

    plnk::startup::set_enabled_at(&link, false, std::path::Path::new(r"C:/plnk/plnk.exe"))
        .expect("disabling autostart must succeed");
    assert!(!link.exists(), "disabling must remove the shortcut");
    assert!(!plnk::startup::is_enabled_at(&link));

    let _ = std::fs::remove_dir_all(&dir);
}

/// Options are only reachable from the tray while no annotation session is
/// running (legacy `FormOptions` rule): the two global hotkeys are unregistered
/// while the window is open.
#[test]
fn options_require_tray_mode() {
    use plnk::app::AppMode;
    assert!(plnk::options::can_open_in(AppMode::Tray));
    assert!(!plnk::options::can_open_in(AppMode::Drawing));
    assert!(!plnk::options::can_open_in(AppMode::Pointer));
    assert!(!plnk::options::can_open_in(AppMode::Snapping));
}

/// The options window must offer a working hotkey map after a rebind, so
/// registration uses the new combination.
#[test]
fn rebind_is_visible_in_the_registration_map() {
    let mut settings = Settings::defaults();
    let new_hotkey = Hotkey {
        control: false,
        alt: true,
        shift: true,
        win: false,
        key: Some(HotkeyKey::Function(9)),
    };
    plnk::options::apply_hotkey(&mut settings, Command::ToggleInk, new_hotkey).unwrap();

    let map = HotkeyMap::from_settings(&settings);
    let event = plnk::hotkeys::KeyboardEvent {
        control: false,
        alt: true,
        shift: true,
        win: false,
        key: HotkeyKey::Function(9),
    };
    assert_eq!(map.matches(event), Some(Command::ToggleInk));
}
