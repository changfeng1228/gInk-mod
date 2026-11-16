//! The options model: everything the settings window can change.
//!
//! The window itself is a thin Win32 shell over this module, so the rules live
//! here where they are headless-testable: which hotkeys are rebindable, what a
//! rebind does, when a rebind is rejected as a conflict, and when the window may
//! be opened at all.
//!
//! Scope is deliberately small for this step: hotkey rebinding and autostart.
//! The legacy Pens page (colours, alpha, widths) and the 31-language UI are not
//! part of it — the settings they write already round-trip through the INI layer
//! that Task 2 built.

use crate::app::AppMode;
use crate::hotkeys::{self, Command};
use crate::settings::{Hotkey, HotkeyKey, Settings};

/// One row of the hotkey page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyRow {
    /// The action this row rebinds.
    pub command: Command,
    /// Human-readable label shown in the window.
    pub label: &'static str,
    /// The legacy INI key, used for conflict reporting and for the log.
    pub ini_key: &'static str,
    /// Whether the combination must contain a modifier. System-wide hotkeys
    /// cannot be a bare key: Windows refuses them and every keystroke in every
    /// application would match.
    pub requires_modifier: bool,
    /// The value currently configured.
    pub hotkey: Hotkey,
}

/// A rejected rebind. The window shows `message` and leaves the row unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebindError {
    pub message: String,
}

impl std::fmt::Display for RebindError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for RebindError {}

/// Labels and INI keys for the non-pen actions, in the legacy display order.
const ACTION_ROWS: &[(Command, &str, &str, bool)] = &[
    (Command::ToggleInk, "启动/退出批注", "Hotkey_Global", true),
    (Command::Eraser, "橡皮", "Hotkey_Eraser", false),
    (
        Command::InkVisible,
        "隐藏/显示笔迹",
        "Hotkey_InkVisible",
        false,
    ),
    (Command::Pointer, "指针模式", "Hotkey_Pointer", false),
    (Command::Pan, "平移", "Hotkey_Pan", false),
    (Command::Undo, "撤销", "Hotkey_Undo", false),
    (Command::Redo, "重做", "Hotkey_Redo", false),
    (Command::Snapshot, "截图", "Hotkey_Snapshot", false),
    (Command::SnapPin, "截图并贴图", "Hotkey_SnapPin", true),
    (Command::Clear, "清空", "Hotkey_Clear", false),
];

/// Every hotkey the options window lets the user rebind, in display order.
///
/// The ten pen rows come after the actions, matching the legacy options page.
pub fn rebindable_hotkeys(settings: &Settings) -> Vec<HotkeyRow> {
    let mut rows: Vec<HotkeyRow> = ACTION_ROWS
        .iter()
        .map(|(command, label, ini_key, requires_modifier)| HotkeyRow {
            command: *command,
            label,
            ini_key,
            requires_modifier: *requires_modifier,
            hotkey: current_hotkey(settings, *command),
        })
        .collect();

    for index in 0..settings.pens.len() {
        rows.push(HotkeyRow {
            command: Command::SelectPen(index),
            // The pen labels are numbers; the window prefixes them with the pen
            // colour swatch, so the label stays short.
            label: "",
            ini_key: pen_ini_key(index),
            requires_modifier: false,
            hotkey: current_hotkey(settings, Command::SelectPen(index)),
        });
    }
    rows
}

fn pen_ini_key(index: usize) -> &'static str {
    // The ten legacy pen keys, as literal strings so the existing INI writer in
    // `settings.rs` stays the single source of the spelling.
    const KEYS: [&str; 10] = [
        "Pen0_Hotkey",
        "Pen1_Hotkey",
        "Pen2_Hotkey",
        "Pen3_Hotkey",
        "Pen4_Hotkey",
        "Pen5_Hotkey",
        "Pen6_Hotkey",
        "Pen7_Hotkey",
        "Pen8_Hotkey",
        "Pen9_Hotkey",
    ];
    KEYS.get(index).copied().unwrap_or("Pen_Hotkey")
}

/// The value currently configured for `command`.
pub fn current_hotkey(settings: &Settings, command: Command) -> Hotkey {
    let hotkeys = &settings.hotkeys;
    match command {
        Command::ToggleInk => hotkeys.global,
        Command::Eraser => hotkeys.eraser,
        Command::InkVisible => hotkeys.ink_visible,
        Command::Pointer => hotkeys.pointer,
        Command::Pan => hotkeys.pan,
        Command::Undo => hotkeys.undo,
        Command::Redo => hotkeys.redo,
        Command::Snapshot => hotkeys.snapshot,
        Command::SnapPin => hotkeys.snap_pin,
        Command::Clear => hotkeys.clear,
        Command::SelectPen(index) => hotkeys.pens.get(index).copied().unwrap_or_default(),
    }
}

fn hotkey_slot(settings: &mut Settings, command: Command) -> Option<&mut Hotkey> {
    let hotkeys = &mut settings.hotkeys;
    Some(match command {
        Command::ToggleInk => &mut hotkeys.global,
        Command::Eraser => &mut hotkeys.eraser,
        Command::InkVisible => &mut hotkeys.ink_visible,
        Command::Pointer => &mut hotkeys.pointer,
        Command::Pan => &mut hotkeys.pan,
        Command::Undo => &mut hotkeys.undo,
        Command::Redo => &mut hotkeys.redo,
        Command::Snapshot => &mut hotkeys.snapshot,
        Command::SnapPin => &mut hotkeys.snap_pin,
        Command::Clear => &mut hotkeys.clear,
        Command::SelectPen(index) => hotkeys.pens.get_mut(index)?,
    })
}

/// Applies a rebind, rejecting combinations Windows cannot register and
/// combinations that would duplicate another action.
///
/// On any rejection `settings` is left byte-identical to how it was: the window
/// keeps showing the old value and a message.
pub fn apply_hotkey(
    settings: &mut Settings,
    command: Command,
    hotkey: Hotkey,
) -> Result<(), RebindError> {
    // A combination with modifiers but no key is half-entered; the legacy INI
    // parser reports it as invalid, so the window must not save it.
    if hotkey.key.is_none() && has_modifier(&hotkey) {
        return Err(RebindError {
            message: "已选择修饰键但还没有按下主键".to_owned(),
        });
    }

    let requires_modifier = ACTION_ROWS
        .iter()
        .find(|(candidate, ..)| *candidate == command)
        .is_some_and(|(_, _, _, requires)| *requires);
    if requires_modifier && hotkey.key.is_some() && !has_modifier(&hotkey) {
        return Err(RebindError {
            message: "系统级热键必须包含 Ctrl、Alt、Shift 或 Win".to_owned(),
        });
    }

    // Clearing to `None` is always allowed and cannot conflict.
    if hotkey.key.is_some()
        && let Some(other) = conflicting_command(settings, command, hotkey)
    {
        return Err(RebindError {
            message: format!(
                "该组合已被「{}」占用：{}",
                label_for(other),
                hotkeys::format_hotkey(&hotkey)
            ),
        });
    }

    if let Some(slot) = hotkey_slot(settings, command) {
        *slot = hotkey;
    }
    Ok(())
}

/// The first other action already bound to `hotkey`, if any.
fn conflicting_command(settings: &Settings, target: Command, hotkey: Hotkey) -> Option<Command> {
    rebindable_hotkeys(settings)
        .into_iter()
        .find(|row| row.command != target && row.hotkey == hotkey)
        .map(|row| row.command)
}

fn label_for(command: Command) -> &'static str {
    ACTION_ROWS
        .iter()
        .find(|(candidate, ..)| *candidate == command)
        .map(|(_, label, ..)| *label)
        .unwrap_or("某个笔")
}

/// What one keystroke means while a hotkey box has focus.
///
/// The settings window turns every key press into one of these four outcomes
/// before it touches the configuration, so the decision is testable without a
/// window: the window only reads the modifier state and shows the result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// A complete combination. It still has to pass [`apply_hotkey`]: a bare
    /// key is legal for a pen row but not for a system-wide hotkey.
    Key(Hotkey),
    /// Backspace or Delete: bind nothing to this action.
    Clear,
    /// A modifier went down on its own. The capture keeps waiting for the main
    /// key, so pressing Ctrl first does not end the gesture.
    ModifierOnly,
    /// The key has no [`HotkeyKey`] counterpart (arrows, Space, Esc, symbols).
    /// The window reports it instead of binding something the user cannot see.
    Unsupported(u16),
}

/// Maps one Win32 virtual-key code plus the live modifier state to an outcome.
///
/// `vk` is the `wParam` of `WM_KEYDOWN`/`WM_SYSKEYDOWN`. The four flags are the
/// queried state of Ctrl/Alt/Shift/Win at the moment the main key went down --
/// a combination is "whatever was held when the last key of the gesture landed",
/// which is what the user typed and what the legacy application recorded.
pub fn capture_from_vk(vk: u16, control: bool, alt: bool, shift: bool, win: bool) -> CaptureOutcome {
    match vk {
        // VK_A..VK_Z are the ASCII uppercase letters.
        0x41..=0x5A => CaptureOutcome::Key(Hotkey {
            control,
            alt,
            shift,
            win,
            key: Some(HotkeyKey::Letter(char::from(vk as u8))),
        }),
        // VK_0..VK_9 are the ASCII digits; the numpad keys sit at 0x60.
        0x30..=0x39 => CaptureOutcome::Key(Hotkey {
            control,
            alt,
            shift,
            win,
            key: Some(HotkeyKey::Digit((vk - 0x30) as u8)),
        }),
        0x60..=0x69 => CaptureOutcome::Key(Hotkey {
            control,
            alt,
            shift,
            win,
            key: Some(HotkeyKey::NumPad((vk - 0x60) as u8)),
        }),
        // VK_F1..VK_F12.
        0x70..=0x7B => CaptureOutcome::Key(Hotkey {
            control,
            alt,
            shift,
            win,
            key: Some(HotkeyKey::Function((vk - 0x70 + 1) as u8)),
        }),
        0x08 | 0x2E => CaptureOutcome::Clear,
        // Shift, Ctrl, Alt, the left/right variants, and both Win keys.
        0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5 => {
            CaptureOutcome::ModifierOnly
        }
        other => CaptureOutcome::Unsupported(other),
    }
}

fn has_modifier(hotkey: &Hotkey) -> bool {
    hotkey.control || hotkey.alt || hotkey.shift || hotkey.win
}

/// Opens the options window only from the tray.
///
/// While an annotation session runs the two system-wide hotkeys are registered
/// and the overlay owns the pointer, so a modal settings window would fight the
/// drawing session; the legacy application enforced the same rule.
pub fn can_open_in(mode: AppMode) -> bool {
    matches!(mode, AppMode::Tray)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::HotkeyKey;

    #[test]
    fn rows_cover_actions_and_all_ten_pens() {
        let settings = Settings::defaults();
        let rows = rebindable_hotkeys(&settings);
        assert_eq!(rows.len(), ACTION_ROWS.len() + settings.pens.len());
    }

    #[test]
    fn only_system_wide_rows_require_a_modifier() {
        let settings = Settings::defaults();
        let rows = rebindable_hotkeys(&settings);
        let required: Vec<&str> = rows
            .iter()
            .filter(|row| row.requires_modifier)
            .map(|row| row.ini_key)
            .collect();
        assert_eq!(required, vec!["Hotkey_Global", "Hotkey_SnapPin"]);
    }

    #[test]
    fn bare_key_is_rejected_for_the_global_hotkey() {
        let mut settings = Settings::defaults();
        let bare = Hotkey {
            control: false,
            alt: false,
            shift: false,
            win: false,
            key: Some(HotkeyKey::Letter('G')),
        };
        assert!(apply_hotkey(&mut settings, Command::ToggleInk, bare).is_err());
        assert!(apply_hotkey(&mut settings, Command::SelectPen(0), bare).is_ok());
    }

    #[test]
    fn modifier_without_key_is_rejected() {
        let mut settings = Settings::defaults();
        let half = Hotkey {
            control: true,
            alt: false,
            shift: false,
            win: false,
            key: None,
        };
        assert!(apply_hotkey(&mut settings, Command::Undo, half).is_err());
    }

    #[test]
    fn clearing_to_none_is_not_a_conflict() {
        let mut settings = Settings::defaults();
        assert!(apply_hotkey(&mut settings, Command::Undo, Hotkey::default()).is_ok());
        assert!(apply_hotkey(&mut settings, Command::Redo, Hotkey::default()).is_ok());
        assert!(settings.hotkeys.undo.key.is_none());
        assert!(settings.hotkeys.redo.key.is_none());
    }

    #[test]
    fn rebinding_to_the_same_value_is_allowed() {
        let mut settings = Settings::defaults();
        let current = settings.hotkeys.eraser;
        assert!(apply_hotkey(&mut settings, Command::Eraser, current).is_ok());
    }
}
