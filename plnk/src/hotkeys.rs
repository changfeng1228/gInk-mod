//! Hotkey mapping, validation and global-hotkey registration.
//!
//! The legacy program registers only two combinations system-wide (`Hotkey_Global`
//! and `Hotkey_SnapPin`); the tool/pen hotkeys are matched locally while an ink
//! window has focus. [`HotkeyMap`] keeps both kinds in one table so later tasks
//! can use [`HotkeyMap::matches`] for local key events, while
//! [`GlobalHotkeys`] registers only the system-wide ones.
//!
//! Nothing here is silent: invalid combinations, duplicate combinations and
//! Windows refusals (typically "already owned by another program") are returned
//! as [`HotkeyIssue`] values with the legacy setting name and the readable
//! combination.

use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};

use crate::settings::{Hotkey, HotkeyKey, MAX_PEN_COUNT, Settings};

/// An action a hotkey can trigger. Later tasks extend the dispatch; the
/// application-shell task only changes [`crate::app::AppMode`] for these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    /// Start ink, cycle drawing/pointer, or return to drawing.
    ToggleInk,
    SelectPen(usize),
    Eraser,
    InkVisible,
    Pointer,
    Pan,
    Undo,
    Redo,
    Snapshot,
    SnapPin,
    Clear,
}

/// A keyboard event in the same terms as the legacy `ModifierMatch` checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: HotkeyKey,
}

/// One configured hotkey. `global` entries are registered with Windows; the
/// others are matched locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyEntry {
    /// The legacy INI key, e.g. `Hotkey_Global` or `Pen3_Hotkey`.
    pub name: String,
    pub command: Command,
    pub hotkey: Hotkey,
    pub global: bool,
}

/// All configured hotkeys in legacy check order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyMap {
    entries: Vec<HotkeyEntry>,
}

impl HotkeyMap {
    pub fn from_settings(settings: &Settings) -> Self {
        let hotkeys = &settings.hotkeys;
        let mut entries = vec![
            entry("Hotkey_Global", Command::ToggleInk, hotkeys.global, true),
            entry("Hotkey_Eraser", Command::Eraser, hotkeys.eraser, false),
            entry(
                "Hotkey_InkVisible",
                Command::InkVisible,
                hotkeys.ink_visible,
                false,
            ),
            entry("Hotkey_Pointer", Command::Pointer, hotkeys.pointer, false),
            entry("Hotkey_Pan", Command::Pan, hotkeys.pan, false),
            entry("Hotkey_Undo", Command::Undo, hotkeys.undo, false),
            entry("Hotkey_Redo", Command::Redo, hotkeys.redo, false),
            entry(
                "Hotkey_Snapshot",
                Command::Snapshot,
                hotkeys.snapshot,
                false,
            ),
            entry("Hotkey_Clear", Command::Clear, hotkeys.clear, false),
            entry("Hotkey_SnapPin", Command::SnapPin, hotkeys.snap_pin, true),
        ];
        for (index, hotkey) in hotkeys.pens.iter().enumerate().take(MAX_PEN_COUNT) {
            entries.push(entry(
                &format!("Pen{index}_Hotkey"),
                Command::SelectPen(index),
                *hotkey,
                false,
            ));
        }
        Self { entries }
    }

    /// Matches a local keyboard event. Disabled (`None`) and invalid entries
    /// never match; duplicate entries resolve to the first configured action,
    /// and the duplicate itself is reported by [`Self::issues`].
    pub fn matches(&self, event: KeyboardEvent) -> Option<Command> {
        self.entries.iter().find_map(|entry| {
            let hotkey = entry.hotkey;
            let key = hotkey.key?;
            if entry.global && !has_modifier(&hotkey) {
                // A system-wide hotkey without a modifier is invalid and never
                // registered; it must not trigger through local matching either.
                return None;
            }
            let modifiers_match = hotkey.control == event.control
                && hotkey.alt == event.alt
                && hotkey.shift == event.shift
                && hotkey.win == event.win;
            (key == event.key && modifiers_match).then_some(entry.command)
        })
    }

    /// Configuration problems that exist independently of Windows: modifier
    /// bits without a key, system-wide combinations without a modifier, and
    /// duplicate combinations.
    pub fn issues(&self) -> Vec<HotkeyIssue> {
        let mut issues = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let hotkey = entry.hotkey;
            let combination = format_hotkey(&hotkey);
            if hotkey.key.is_none() {
                if has_modifier(&hotkey) {
                    issues.push(HotkeyIssue {
                        name: entry.name.clone(),
                        combination,
                        kind: HotkeyIssueKind::Invalid {
                            reason: "a modifier is set but no key is configured".to_owned(),
                        },
                    });
                }
                continue;
            }
            if entry.global && !has_modifier(&hotkey) {
                issues.push(HotkeyIssue {
                    name: entry.name.clone(),
                    combination,
                    kind: HotkeyIssueKind::Invalid {
                        reason: "a system-wide hotkey needs at least one modifier".to_owned(),
                    },
                });
                continue;
            }
            if let Some(previous) = self.entries[..index]
                .iter()
                .find(|previous| previous.hotkey.key.is_some() && previous.hotkey == hotkey)
            {
                issues.push(HotkeyIssue {
                    name: entry.name.clone(),
                    combination,
                    kind: HotkeyIssueKind::Conflict {
                        with: previous.name.clone(),
                    },
                });
            }
        }
        issues
    }
}

fn entry(name: &str, command: Command, hotkey: Hotkey, global: bool) -> HotkeyEntry {
    HotkeyEntry {
        name: name.to_owned(),
        command,
        hotkey,
        global,
    }
}

fn has_modifier(hotkey: &Hotkey) -> bool {
    hotkey.control || hotkey.alt || hotkey.shift || hotkey.win
}

/// The legacy textual form, e.g. `Ctrl + Alt + Shift + P` or `None`.
pub fn format_hotkey(hotkey: &Hotkey) -> String {
    let Some(key) = hotkey.key else {
        return "None".to_owned();
    };
    let mut parts = Vec::new();
    if hotkey.control {
        parts.push("Ctrl".to_owned());
    }
    if hotkey.alt {
        parts.push("Alt".to_owned());
    }
    if hotkey.shift {
        parts.push("Shift".to_owned());
    }
    if hotkey.win {
        parts.push("Win".to_owned());
    }
    parts.push(key.to_string());
    parts.join(" + ")
}

/// A validation or registration problem, reported to the user and the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyIssue {
    pub name: String,
    pub combination: String,
    pub kind: HotkeyIssueKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyIssueKind {
    Invalid { reason: String },
    Conflict { with: String },
    RegistrationFailed { message: String },
}

impl fmt::Display for HotkeyIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.name, self.combination)?;
        match &self.kind {
            HotkeyIssueKind::Invalid { reason } => write!(formatter, ": invalid: {reason}"),
            HotkeyIssueKind::Conflict { with } => {
                write!(formatter, ": conflicts with {with}")
            }
            HotkeyIssueKind::RegistrationFailed { message } => write!(
                formatter,
                ": Windows refused the registration (another program may use it): {message}"
            ),
        }
    }
}

/// Outcome of one registration pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotkeyReport {
    pub registered: Vec<Command>,
    pub issues: Vec<HotkeyIssue>,
}

struct GlobalRegistration {
    id: i32,
    command: Command,
}

const FIRST_GLOBAL_ID: i32 = 1;

/// System-wide hotkeys owned by this thread/window. Dropping the value (or
/// calling [`Self::unregister_all`]) releases every registration, so an exit
/// path can never leave a hotkey captured.
pub struct GlobalHotkeys {
    window: Option<HWND>,
    registrations: Vec<GlobalRegistration>,
    _not_send: PhantomData<Rc<()>>,
}

impl GlobalHotkeys {
    /// Registers the `global` entries of `map`. `enabled = false` is the
    /// development `--no-hotkeys` mode: nothing is registered and no issues are
    /// produced. Registration failures are issues, not a fatal error, because
    /// the app remains usable without them.
    pub fn register(map: &HotkeyMap, window: Option<HWND>, enabled: bool) -> (Self, HotkeyReport) {
        let mut hotkeys = Self {
            window,
            registrations: Vec::new(),
            _not_send: PhantomData,
        };
        if !enabled {
            return (hotkeys, HotkeyReport::default());
        }

        let mut report = HotkeyReport {
            issues: map.issues(),
            registered: Vec::new(),
        };
        for (index, entry) in map.entries.iter().enumerate() {
            if !entry.global {
                continue;
            }
            // Invalid or conflicting entries are already reported; skipping
            // them here avoids letting a broken entry shadow a working one.
            if report.issues.iter().any(|issue| issue.name == entry.name) {
                continue;
            }
            let Some(key) = entry.hotkey.key else {
                continue;
            };
            let Some(virtual_key) = virtual_key(key) else {
                report.issues.push(HotkeyIssue {
                    name: entry.name.clone(),
                    combination: format_hotkey(&entry.hotkey),
                    kind: HotkeyIssueKind::Invalid {
                        reason: "unsupported key".to_owned(),
                    },
                });
                continue;
            };
            let id = FIRST_GLOBAL_ID + index as i32;
            match unsafe { RegisterHotKey(window, id, modifier_flags(&entry.hotkey), virtual_key) }
            {
                Ok(()) => {
                    hotkeys.registrations.push(GlobalRegistration {
                        id,
                        command: entry.command,
                    });
                    report.registered.push(entry.command);
                }
                Err(error) => report.issues.push(HotkeyIssue {
                    name: entry.name.clone(),
                    combination: format_hotkey(&entry.hotkey),
                    kind: HotkeyIssueKind::RegistrationFailed {
                        message: error.to_string(),
                    },
                }),
            }
        }
        (hotkeys, report)
    }

    /// Maps a `WM_HOTKEY` id back to the action that produced it.
    pub fn command_for_id(&self, id: i32) -> Option<Command> {
        self.registrations
            .iter()
            .find(|registration| registration.id == id)
            .map(|registration| registration.command)
    }

    /// Unregisters every owned hotkey and returns how many are now free.
    pub fn unregister_all(&mut self) -> usize {
        let mut released = 0;
        for registration in self.registrations.drain(..) {
            if unsafe { UnregisterHotKey(self.window, registration.id) }.is_ok() {
                released += 1;
            }
        }
        released
    }
}

impl Drop for GlobalHotkeys {
    fn drop(&mut self) {
        self.unregister_all();
    }
}

fn modifier_flags(hotkey: &Hotkey) -> HOT_KEY_MODIFIERS {
    let mut flags = HOT_KEY_MODIFIERS(0);
    if hotkey.control {
        flags |= MOD_CONTROL;
    }
    if hotkey.alt {
        flags |= MOD_ALT;
    }
    if hotkey.shift {
        flags |= MOD_SHIFT;
    }
    if hotkey.win {
        flags |= MOD_WIN;
    }
    flags
}

fn virtual_key(key: HotkeyKey) -> Option<u32> {
    match key {
        // `HotkeyKey::Letter` is guaranteed uppercase by the parser; accept a
        // manually built lowercase value too.
        HotkeyKey::Letter(letter) => Some(letter.to_ascii_uppercase() as u32),
        HotkeyKey::Digit(digit) if digit <= 9 => Some(u32::from(b'0') + u32::from(digit)),
        HotkeyKey::NumPad(digit) if digit <= 9 => Some(0x60 + u32::from(digit)),
        HotkeyKey::Function(number) if (1..=24).contains(&number) => {
            Some(0x70 + u32::from(number) - 1)
        }
        _ => None,
    }
}
