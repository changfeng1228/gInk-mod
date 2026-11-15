//! Strongly typed settings plus the legacy gInk INI importer.
//!
//! The legacy program has two different default sets:
//!
//! * the **factory INI files** shipped in `bin/`, and
//! * the **code defaults** used when a file (or a key) is absent.
//!
//! They intentionally disagree for the Pen3/Pen7 colours, `Toolbar_Height`,
//! `Allow_Dragging_Toolbar`, `Allow_Hotkey_In_Pointer_Mode` and the non-global
//! hotkeys; see `docs/acceptance/legacy-baseline.md` sections 4.1-4.3.
//! [`Settings::defaults`] returns the code defaults, and importing the factory
//! files is expected to produce different values for those items.
//!
//! Known legacy defects are not reproduced: F10-F12 hotkeys parse (D-001),
//! NumPad hotkeys use stable names (D-002), an invalid hotkey rolls back the
//! whole hotkey (D-003) and a two-digit pen number is rejected (D-023).

use std::fmt;
use std::io::BufRead;
use std::path::PathBuf;

use crate::error::SettingsError;

pub const MAX_PEN_COUNT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasCursor {
    Arrow,
    PenTip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOnSnap {
    True,
    False,
    BlankOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PenSettings {
    /// Legacy `PenN_Enabled`: whether the pen's toolbar button is shown. A
    /// hidden pen can still be selected with its hotkey.
    pub enabled: bool,
    pub color: Rgb,
    /// Opacity 0-255 from the legacy `PenN_Alpha` key. The legacy drawing code
    /// stores `Transparency = 255 - alpha` internally.
    pub alpha: u8,
    pub width: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyKey {
    Letter(char),
    Digit(u8),
    NumPad(u8),
    Function(u8),
}

impl fmt::Display for HotkeyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Letter(letter) => write!(formatter, "{letter}"),
            Self::Digit(digit) => write!(formatter, "{digit}"),
            Self::NumPad(digit) => write!(formatter, "NumPad{digit}"),
            Self::Function(number) => write!(formatter, "F{number}"),
        }
    }
}

/// A hotkey in the legacy textual form, e.g. `Ctrl + Alt + Shift + P`, `F12`,
/// `NumPad0`, `1` or `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hotkey {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Option<HotkeyKey>,
}

impl Hotkey {
    /// Parses the legacy hotkey string. Modifier names are case-insensitive and
    /// may be written as `Ctrl` or `Control`; the key itself must appear once.
    pub fn parse(text: &str) -> Option<Self> {
        if text == "None" {
            return Some(Self::default());
        }

        let mut hotkey = Self::default();
        let mut key = None;
        for token in text.split('+').map(str::trim) {
            if token.is_empty() {
                return None;
            }
            if token.eq_ignore_ascii_case("ctrl") || token.eq_ignore_ascii_case("control") {
                hotkey.control = true;
            } else if token.eq_ignore_ascii_case("alt") {
                hotkey.alt = true;
            } else if token.eq_ignore_ascii_case("shift") {
                hotkey.shift = true;
            } else if token.eq_ignore_ascii_case("win") {
                hotkey.win = true;
            } else if key.is_none() {
                key = Some(parse_hotkey_key(token)?);
            } else {
                return None;
            }
        }
        // A modifier on its own is not a hotkey.
        hotkey.key = Some(key?);
        Some(hotkey)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(key) = self.key else {
            return formatter.write_str("None");
        };

        let mut first = true;
        for (enabled, name) in [
            (self.control, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if enabled {
                if !first {
                    formatter.write_str(" + ")?;
                }
                formatter.write_str(name)?;
                first = false;
            }
        }
        if !first {
            formatter.write_str(" + ")?;
        }
        write!(formatter, "{key}")
    }
}

fn parse_hotkey_key(token: &str) -> Option<HotkeyKey> {
    let upper = token.to_ascii_uppercase();

    if let Some(number) = upper.strip_prefix('F')
        && !number.is_empty()
    {
        return match number.parse::<u8>() {
            Ok(number) if (1..=12).contains(&number) => Some(HotkeyKey::Function(number)),
            _ => None,
        };
    }
    if let Some(number) = upper.strip_prefix("NUMPAD") {
        return match number.parse::<u8>() {
            Ok(number) if number <= 9 => Some(HotkeyKey::NumPad(number)),
            _ => None,
        };
    }

    let mut characters = upper.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    match character {
        'A'..='Z' => Some(HotkeyKey::Letter(character)),
        '0'..='9' => Some(HotkeyKey::Digit(character as u8 - b'0')),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkeys {
    pub global: Hotkey,
    pub eraser: Hotkey,
    pub ink_visible: Hotkey,
    pub pointer: Hotkey,
    pub pan: Hotkey,
    pub undo: Hotkey,
    pub redo: Hotkey,
    pub snapshot: Hotkey,
    pub snap_pin: Hotkey,
    pub clear: Hotkey,
    pub pens: [Hotkey; MAX_PEN_COUNT],
}

/// The single strongly typed configuration entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub language_file: String,
    pub white_tray_icon: bool,
    pub snapshot_path: String,
    pub eraser_icon: bool,
    pub pan_icon: bool,
    pub pointer_icon: bool,
    pub pen_width_icon: bool,
    pub ink_visible_icon: bool,
    pub snapshot_icon: bool,
    pub undo_icon: bool,
    pub clear_icon: bool,
    pub canvas_cursor: CanvasCursor,
    pub allow_dragging_toolbar: bool,
    pub allow_hotkey_in_pointer_mode: bool,
    pub toolbar_left: i32,
    pub toolbar_top: i32,
    pub close_on_snap: CloseOnSnap,
    pub always_hide_toolbar: bool,
    pub toolbar_height: f32,
    pub pens: [PenSettings; MAX_PEN_COUNT],
    pub hotkeys: Hotkeys,
}

impl Settings {
    /// Code defaults, i.e. the legacy state when none of the three INI files
    /// can be found. Deliberately different from the shipped factory files for
    /// the items listed in the module documentation.
    pub fn defaults() -> Self {
        Self {
            language_file: "en-us".to_owned(),
            white_tray_icon: false,
            snapshot_path: "%USERPROFILE%/Pictures/gInk/".to_owned(),
            eraser_icon: true,
            pan_icon: true,
            pointer_icon: true,
            pen_width_icon: false,
            ink_visible_icon: true,
            snapshot_icon: true,
            undo_icon: true,
            clear_icon: true,
            canvas_cursor: CanvasCursor::Arrow,
            allow_dragging_toolbar: true,
            allow_hotkey_in_pointer_mode: true,
            toolbar_left: 0,
            toolbar_top: 0,
            close_on_snap: CloseOnSnap::BlankOnly,
            always_hide_toolbar: false,
            toolbar_height: 0.06,
            pens: default_pens(),
            hotkeys: default_hotkeys(),
        }
    }

    /// Imports the three legacy documents.
    ///
    /// The documents are applied in the legacy read order (`pens.ini`,
    /// `config.ini`, `hotkeys.ini`). Missing keys keep the code defaults;
    /// invalid values keep that key's default and append a warning carrying the
    /// key name. Only a whole-document read failure returns an error.
    pub fn from_legacy(
        config: &mut dyn BufRead,
        pens: &mut dyn BufRead,
        hotkeys: &mut dyn BufRead,
    ) -> Result<SettingsImport, SettingsError> {
        let mut settings = Self::defaults();
        let mut warnings = Vec::new();
        // Legacy read order: pens.ini, then config.ini, then hotkeys.ini.
        let text = read_document(pens, "pens.ini")?;
        apply_document(&mut settings, &text, &mut warnings);
        let text = read_document(config, "config.ini")?;
        apply_document(&mut settings, &text, &mut warnings);
        let text = read_document(hotkeys, "hotkeys.ini")?;
        apply_document(&mut settings, &text, &mut warnings);
        Ok(SettingsImport { settings, warnings })
    }
}

fn default_pens() -> [PenSettings; MAX_PEN_COUNT] {
    [
        PenSettings {
            enabled: false,
            color: Rgb::new(80, 80, 80),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: true,
            color: Rgb::new(225, 60, 60),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: true,
            color: Rgb::new(30, 110, 200),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: true,
            color: Rgb::new(235, 180, 55),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: true,
            color: Rgb::new(120, 175, 70),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: true,
            color: Rgb::new(235, 125, 15),
            alpha: 80,
            width: 500,
        },
        PenSettings {
            enabled: false,
            color: Rgb::new(230, 230, 230),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: false,
            color: Rgb::new(250, 140, 200),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: false,
            color: Rgb::new(25, 180, 175),
            alpha: 255,
            width: 80,
        },
        PenSettings {
            enabled: false,
            color: Rgb::new(145, 70, 160),
            alpha: 80,
            width: 500,
        },
    ]
}

const fn hotkey(
    control: bool,
    alt: bool,
    shift: bool,
    win: bool,
    key: Option<HotkeyKey>,
) -> Hotkey {
    Hotkey {
        control,
        alt,
        shift,
        win,
        key,
    }
}

fn default_hotkeys() -> Hotkeys {
    Hotkeys {
        global: hotkey(true, true, false, false, Some(HotkeyKey::Letter('G'))),
        eraser: Hotkey::default(),
        ink_visible: Hotkey::default(),
        pointer: Hotkey::default(),
        pan: Hotkey::default(),
        undo: Hotkey::default(),
        redo: Hotkey::default(),
        snapshot: Hotkey::default(),
        snap_pin: hotkey(true, true, true, false, Some(HotkeyKey::Letter('P'))),
        clear: Hotkey::default(),
        pens: [Hotkey::default(); MAX_PEN_COUNT],
    }
}

/// One imported settings snapshot plus the per-key warnings it produced.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsImport {
    pub settings: Settings,
    pub warnings: Vec<SettingsWarning>,
}

/// A single invalid legacy value. The key is spelled exactly as it appeared in
/// the file so the user can find it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsWarning {
    pub key: String,
    pub value: String,
    pub message: String,
}

impl SettingsWarning {
    fn new(key: &str, value: &str, message: &str) -> Self {
        Self {
            key: key.to_owned(),
            value: value.to_owned(),
            message: message.to_owned(),
        }
    }
}

impl fmt::Display for SettingsWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid value {:?} for {}: {}",
            self.value, self.key, self.message
        )
    }
}

#[derive(Clone, Copy)]
enum PenField {
    Enabled,
    Red,
    Green,
    Blue,
    Alpha,
    Width,
    Hotkey,
}

#[derive(Clone, Copy)]
enum SettingKey {
    LanguageFile,
    WhiteTrayIcon,
    SnapshotPath,
    EraserIcon,
    PanIcon,
    PointerIcon,
    PenWidthIcon,
    InkVisibleIcon,
    SnapshotIcon,
    UndoIcon,
    ClearIcon,
    CanvasCursor,
    AllowDraggingToolbar,
    AllowHotkeyInPointerMode,
    ToolbarLeft,
    ToolbarTop,
    CloseOnSnap,
    AlwaysHideToolbar,
    ToolbarHeight,
    Pen { index: usize, field: PenField },
    HotkeyGlobal,
    HotkeyEraser,
    HotkeyInkVisible,
    HotkeyPointer,
    HotkeyPan,
    HotkeyUndo,
    HotkeyRedo,
    HotkeySnapshot,
    HotkeySnapPin,
    HotkeyClear,
}

enum KeyLookup {
    Known(SettingKey),
    InvalidPenId,
    Unknown,
}

fn classify(upper_key: &str) -> KeyLookup {
    let known = match upper_key {
        "LANGUAGE_FILE" => Some(SettingKey::LanguageFile),
        "WHITE_TRAY_ICON" => Some(SettingKey::WhiteTrayIcon),
        "SNAPSHOT_PATH" => Some(SettingKey::SnapshotPath),
        "ERASER_ICON" => Some(SettingKey::EraserIcon),
        "PAN_ICON" => Some(SettingKey::PanIcon),
        "POINTER_ICON" => Some(SettingKey::PointerIcon),
        "PEN_WIDTH_ICON" => Some(SettingKey::PenWidthIcon),
        "INKVISIBLE_ICON" => Some(SettingKey::InkVisibleIcon),
        "SNAPSHOT_ICON" => Some(SettingKey::SnapshotIcon),
        "UNDO_ICON" => Some(SettingKey::UndoIcon),
        "CLEAR_ICON" => Some(SettingKey::ClearIcon),
        "CANVAS_CURSOR" => Some(SettingKey::CanvasCursor),
        "ALLOW_DRAGGING_TOOLBAR" => Some(SettingKey::AllowDraggingToolbar),
        "ALLOW_HOTKEY_IN_POINTER_MODE" => Some(SettingKey::AllowHotkeyInPointerMode),
        "TOOLBAR_LEFT" => Some(SettingKey::ToolbarLeft),
        "TOOLBAR_TOP" => Some(SettingKey::ToolbarTop),
        "CLOSE_ON_SNAP" => Some(SettingKey::CloseOnSnap),
        "ALWAYS_HIDE_TOOLBAR" => Some(SettingKey::AlwaysHideToolbar),
        "TOOLBAR_HEIGHT" => Some(SettingKey::ToolbarHeight),
        "HOTKEY_GLOBAL" => Some(SettingKey::HotkeyGlobal),
        "HOTKEY_ERASER" => Some(SettingKey::HotkeyEraser),
        "HOTKEY_INKVISIBLE" => Some(SettingKey::HotkeyInkVisible),
        "HOTKEY_POINTER" => Some(SettingKey::HotkeyPointer),
        "HOTKEY_PAN" => Some(SettingKey::HotkeyPan),
        "HOTKEY_UNDO" => Some(SettingKey::HotkeyUndo),
        "HOTKEY_REDO" => Some(SettingKey::HotkeyRedo),
        "HOTKEY_SNAPSHOT" => Some(SettingKey::HotkeySnapshot),
        "HOTKEY_SNAPPIN" => Some(SettingKey::HotkeySnapPin),
        "HOTKEY_CLEAR" => Some(SettingKey::HotkeyClear),
        _ => None,
    };
    if let Some(key) = known {
        return KeyLookup::Known(key);
    }
    classify_pen_key(upper_key)
}

fn classify_pen_key(upper_key: &str) -> KeyLookup {
    let Some(rest) = upper_key.strip_prefix("PEN") else {
        return KeyLookup::Unknown;
    };
    let Some((digits, field)) = rest.split_once('_') else {
        return KeyLookup::Unknown;
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return KeyLookup::Unknown;
    }
    let Ok(index) = digits.parse::<usize>() else {
        // All-digit but too large for `usize`: still an invalid pen number.
        return KeyLookup::InvalidPenId;
    };
    let field = match field {
        "ENABLED" => PenField::Enabled,
        "RED" => PenField::Red,
        "GREEN" => PenField::Green,
        "BLUE" => PenField::Blue,
        "ALPHA" => PenField::Alpha,
        "WIDTH" => PenField::Width,
        "HOTKEY" => PenField::Hotkey,
        // Unknown suffix: keep the line as an unknown key, like the legacy
        // parser which only inspects known suffixes.
        _ => return KeyLookup::Unknown,
    };
    if index >= MAX_PEN_COUNT {
        // D-023: `PEN10_RED` used to be applied to Pen1.
        return KeyLookup::InvalidPenId;
    }
    KeyLookup::Known(SettingKey::Pen { index, field })
}

fn read_document(reader: &mut dyn BufRead, file_name: &str) -> Result<String, SettingsError> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| SettingsError::Read {
            path: PathBuf::from(file_name),
            source,
        })?;
    Ok(decode_text(&bytes))
}

/// Decodes an INI document like the legacy `StreamReader`: UTF-8, with a
/// leading BOM removed and invalid bytes replaced rather than rejected.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
    let bytes = if bytes.starts_with(&BOM) {
        &bytes[BOM.len()..]
    } else {
        bytes
    };
    String::from_utf8_lossy(bytes).into_owned()
}

fn split_ini_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                lines.push(&text[start..index]);
                index += if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = index;
            }
            b'\n' => {
                lines.push(&text[start..index]);
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Returns the raw key text before the first `=` and the trimmed value, or
/// `None` when the legacy parser would skip the line.
fn parse_ini_line(line: &str) -> Option<(&str, &str)> {
    if line.is_empty() {
        return None;
    }
    let first = line.chars().next()?;
    if matches!(first, '-' | '%' | '\'' | '/' | '!' | '[' | '#') {
        return None;
    }
    let (raw_key, rest) = line.split_once('=')?;
    if rest.contains('=') {
        return None;
    }
    Some((raw_key, rest.trim()))
}

fn apply_document(settings: &mut Settings, text: &str, warnings: &mut Vec<SettingsWarning>) {
    for line in split_ini_lines(text) {
        let Some((raw_key, value)) = parse_ini_line(line) else {
            continue;
        };
        let key_name = raw_key.trim();
        let upper_key = key_name.to_uppercase();
        match classify(&upper_key) {
            KeyLookup::Known(key) => apply_setting(settings, key, key_name, value, warnings),
            KeyLookup::InvalidPenId => warnings.push(SettingsWarning::new(
                key_name,
                value,
                "pen number must be between 0 and 9",
            )),
            KeyLookup::Unknown => {}
        }
    }
}

fn apply_setting(
    settings: &mut Settings,
    key: SettingKey,
    key_name: &str,
    value: &str,
    warnings: &mut Vec<SettingsWarning>,
) {
    match key {
        SettingKey::LanguageFile => settings.language_file = value.to_owned(),
        SettingKey::WhiteTrayIcon => match parse_bool(value) {
            Some(enabled) => settings.white_tray_icon = enabled,
            None => {
                // Legacy behaviour: every value other than True/1/ON is false.
                warnings.push(SettingsWarning::new(
                    key_name,
                    value,
                    "only True/1/ON enable the white tray icon; other values are read as False",
                ));
                settings.white_tray_icon = false;
            }
        },
        SettingKey::SnapshotPath => {
            let mut path = value.to_owned();
            if !path.ends_with('/') && !path.ends_with('\\') {
                path.push('/');
            }
            settings.snapshot_path = path;
        }
        SettingKey::EraserIcon => set_bool(&mut settings.eraser_icon, value, key_name, warnings),
        SettingKey::PanIcon => set_bool(&mut settings.pan_icon, value, key_name, warnings),
        SettingKey::PointerIcon => {
            set_bool(&mut settings.pointer_icon, value, key_name, warnings);
        }
        SettingKey::PenWidthIcon => {
            set_bool(&mut settings.pen_width_icon, value, key_name, warnings);
        }
        SettingKey::InkVisibleIcon => {
            set_bool(&mut settings.ink_visible_icon, value, key_name, warnings);
        }
        SettingKey::SnapshotIcon => {
            set_bool(&mut settings.snapshot_icon, value, key_name, warnings);
        }
        SettingKey::UndoIcon => set_bool(&mut settings.undo_icon, value, key_name, warnings),
        SettingKey::ClearIcon => set_bool(&mut settings.clear_icon, value, key_name, warnings),
        SettingKey::CanvasCursor => match value {
            "0" => settings.canvas_cursor = CanvasCursor::Arrow,
            "1" => settings.canvas_cursor = CanvasCursor::PenTip,
            _ => warnings.push(SettingsWarning::new(
                key_name,
                value,
                "expected 0 (arrow) or 1 (pen tip)",
            )),
        },
        SettingKey::AllowDraggingToolbar => {
            set_bool(
                &mut settings.allow_dragging_toolbar,
                value,
                key_name,
                warnings,
            );
        }
        SettingKey::AllowHotkeyInPointerMode => {
            set_bool(
                &mut settings.allow_hotkey_in_pointer_mode,
                value,
                key_name,
                warnings,
            );
        }
        SettingKey::ToolbarLeft => match value.parse::<i32>() {
            Ok(left) => settings.toolbar_left = left,
            Err(_) => warnings.push(SettingsWarning::new(key_name, value, "expected an integer")),
        },
        SettingKey::ToolbarTop => match value.parse::<i32>() {
            Ok(top) => settings.toolbar_top = top,
            Err(_) => warnings.push(SettingsWarning::new(key_name, value, "expected an integer")),
        },
        SettingKey::CloseOnSnap => {
            if let Some(enabled) = parse_bool(value) {
                settings.close_on_snap = if enabled {
                    CloseOnSnap::True
                } else {
                    CloseOnSnap::False
                };
            } else if value.eq_ignore_ascii_case("blankonly") {
                settings.close_on_snap = CloseOnSnap::BlankOnly;
            } else {
                warnings.push(SettingsWarning::new(
                    key_name,
                    value,
                    "expected True, False or BlankOnly",
                ));
            }
        }
        SettingKey::AlwaysHideToolbar => {
            set_bool(&mut settings.always_hide_toolbar, value, key_name, warnings);
        }
        SettingKey::ToolbarHeight => match value.parse::<f32>() {
            Ok(height) if height.is_finite() => settings.toolbar_height = height,
            _ => warnings.push(SettingsWarning::new(
                key_name,
                value,
                "expected a finite number",
            )),
        },
        SettingKey::Pen { index, field } => {
            if let PenField::Hotkey = field {
                set_hotkey(&mut settings.hotkeys.pens[index], value, key_name, warnings);
            } else {
                let pen = &mut settings.pens[index];
                match field {
                    PenField::Enabled => set_bool(&mut pen.enabled, value, key_name, warnings),
                    PenField::Red => {
                        set_color_channel(&mut pen.color.red, value, key_name, warnings);
                    }
                    PenField::Green => {
                        set_color_channel(&mut pen.color.green, value, key_name, warnings);
                    }
                    PenField::Blue => {
                        set_color_channel(&mut pen.color.blue, value, key_name, warnings);
                    }
                    PenField::Alpha => match value.parse::<i32>() {
                        Ok(alpha) if (0..=255).contains(&alpha) => pen.alpha = alpha as u8,
                        _ => warnings.push(SettingsWarning::new(
                            key_name,
                            value,
                            "alpha must be an integer between 0 and 255",
                        )),
                    },
                    PenField::Width => match value.parse::<i32>() {
                        Ok(width) if (30..=3000).contains(&width) => pen.width = width as u16,
                        _ => warnings.push(SettingsWarning::new(
                            key_name,
                            value,
                            "width must be an integer between 30 and 3000",
                        )),
                    },
                    PenField::Hotkey => {}
                }
            }
        }
        SettingKey::HotkeyGlobal => {
            set_hotkey(&mut settings.hotkeys.global, value, key_name, warnings);
        }
        SettingKey::HotkeyEraser => {
            set_hotkey(&mut settings.hotkeys.eraser, value, key_name, warnings);
        }
        SettingKey::HotkeyInkVisible => {
            set_hotkey(&mut settings.hotkeys.ink_visible, value, key_name, warnings);
        }
        SettingKey::HotkeyPointer => {
            set_hotkey(&mut settings.hotkeys.pointer, value, key_name, warnings);
        }
        SettingKey::HotkeyPan => {
            set_hotkey(&mut settings.hotkeys.pan, value, key_name, warnings);
        }
        SettingKey::HotkeyUndo => {
            set_hotkey(&mut settings.hotkeys.undo, value, key_name, warnings);
        }
        SettingKey::HotkeyRedo => {
            set_hotkey(&mut settings.hotkeys.redo, value, key_name, warnings);
        }
        SettingKey::HotkeySnapshot => {
            set_hotkey(&mut settings.hotkeys.snapshot, value, key_name, warnings);
        }
        SettingKey::HotkeySnapPin => {
            set_hotkey(&mut settings.hotkeys.snap_pin, value, key_name, warnings);
        }
        SettingKey::HotkeyClear => {
            set_hotkey(&mut settings.hotkeys.clear, value, key_name, warnings);
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    if value.eq_ignore_ascii_case("true") || value == "1" || value.eq_ignore_ascii_case("on") {
        Some(true)
    } else if value.eq_ignore_ascii_case("false")
        || value == "0"
        || value.eq_ignore_ascii_case("off")
    {
        Some(false)
    } else {
        None
    }
}

fn set_bool(target: &mut bool, value: &str, key_name: &str, warnings: &mut Vec<SettingsWarning>) {
    match parse_bool(value) {
        Some(parsed) => *target = parsed,
        None => warnings.push(SettingsWarning::new(
            key_name,
            value,
            "expected True/False/1/0/ON/OFF",
        )),
    }
}

fn set_color_channel(
    target: &mut u8,
    value: &str,
    key_name: &str,
    warnings: &mut Vec<SettingsWarning>,
) {
    match value.parse::<i32>() {
        Ok(channel) if (0..=255).contains(&channel) => *target = channel as u8,
        _ => warnings.push(SettingsWarning::new(
            key_name,
            value,
            "colour channel must be an integer between 0 and 255",
        )),
    }
}

fn set_hotkey(
    target: &mut Hotkey,
    value: &str,
    key_name: &str,
    warnings: &mut Vec<SettingsWarning>,
) {
    match Hotkey::parse(value) {
        Some(hotkey) => *target = hotkey,
        None => warnings.push(SettingsWarning::new(key_name, value, "not a valid hotkey")),
    }
}

const CANONICAL_CONFIG: [(SettingKey, &str); 19] = [
    (SettingKey::LanguageFile, "Language_File"),
    (SettingKey::WhiteTrayIcon, "White_Tray_Icon"),
    (SettingKey::SnapshotPath, "Snapshot_Path"),
    (SettingKey::EraserIcon, "Eraser_Icon"),
    (SettingKey::PanIcon, "Pan_Icon"),
    (SettingKey::PointerIcon, "Pointer_Icon"),
    (SettingKey::PenWidthIcon, "Pen_Width_Icon"),
    (SettingKey::InkVisibleIcon, "InkVisible_Icon"),
    (SettingKey::SnapshotIcon, "Snapshot_Icon"),
    (SettingKey::UndoIcon, "Undo_Icon"),
    (SettingKey::ClearIcon, "Clear_Icon"),
    (SettingKey::CanvasCursor, "Canvas_Cursor"),
    (SettingKey::AllowDraggingToolbar, "Allow_Dragging_Toolbar"),
    (
        SettingKey::AllowHotkeyInPointerMode,
        "Allow_Hotkey_In_Pointer_Mode",
    ),
    (SettingKey::ToolbarLeft, "Toolbar_Left"),
    (SettingKey::ToolbarTop, "Toolbar_Top"),
    (SettingKey::CloseOnSnap, "Close_On_Snap"),
    (SettingKey::AlwaysHideToolbar, "Always_Hide_Toolbar"),
    (SettingKey::ToolbarHeight, "Toolbar_Height"),
];

const CANONICAL_HOTKEYS: [(SettingKey, &str); 10] = [
    (SettingKey::HotkeyGlobal, "Hotkey_Global"),
    (SettingKey::HotkeyEraser, "Hotkey_Eraser"),
    (SettingKey::HotkeyInkVisible, "Hotkey_InkVisible"),
    (SettingKey::HotkeyPointer, "Hotkey_Pointer"),
    (SettingKey::HotkeyPan, "Hotkey_Pan"),
    (SettingKey::HotkeyUndo, "Hotkey_Undo"),
    (SettingKey::HotkeyRedo, "Hotkey_Redo"),
    (SettingKey::HotkeySnapshot, "Hotkey_Snapshot"),
    (SettingKey::HotkeySnapPin, "Hotkey_SnapPin"),
    (SettingKey::HotkeyClear, "Hotkey_Clear"),
];

const PEN_FIELDS: [(PenField, &str); 6] = [
    (PenField::Enabled, "Enabled"),
    (PenField::Red, "Red"),
    (PenField::Green, "Green"),
    (PenField::Blue, "Blue"),
    (PenField::Alpha, "Alpha"),
    (PenField::Width, "Width"),
];

/// Rewrites every recognised key in `text` from `settings` while preserving
/// comments, blank lines, section headers, unknown keys and line order.
pub(crate) fn rewrite_ini_document(text: &str, settings: &Settings) -> String {
    let mut output = String::new();
    for line in split_ini_lines(text) {
        let Some((raw_key, _)) = parse_ini_line(line) else {
            output.push_str(line);
            output.push_str("\r\n");
            continue;
        };
        let key_name = raw_key.trim();
        let upper_key = key_name.to_uppercase();
        match classify(&upper_key) {
            KeyLookup::Known(key) => {
                output.push_str(raw_key);
                output.push_str("= ");
                output.push_str(&known_value(key, settings));
                output.push_str("\r\n");
            }
            KeyLookup::InvalidPenId | KeyLookup::Unknown => {
                output.push_str(line);
                output.push_str("\r\n");
            }
        }
    }
    output
}

/// Renders a complete settings document, used when there was no legacy source
/// document to preserve.
pub(crate) fn render_settings_document(settings: &Settings) -> String {
    let mut output = String::new();
    for (key, name) in CANONICAL_CONFIG {
        push_setting(&mut output, key, name, settings);
    }
    for index in 0..MAX_PEN_COUNT {
        for (field, name) in PEN_FIELDS {
            push_setting(
                &mut output,
                SettingKey::Pen { index, field },
                &format!("Pen{index}_{name}"),
                settings,
            );
        }
    }
    for (key, name) in CANONICAL_HOTKEYS {
        push_setting(&mut output, key, name, settings);
    }
    for index in 0..MAX_PEN_COUNT {
        push_setting(
            &mut output,
            SettingKey::Pen {
                index,
                field: PenField::Hotkey,
            },
            &format!("Pen{index}_Hotkey"),
            settings,
        );
    }
    output
}

fn push_setting(output: &mut String, key: SettingKey, name: &str, settings: &Settings) {
    output.push_str(name);
    output.push_str("= ");
    output.push_str(&known_value(key, settings));
    output.push_str("\r\n");
}

fn known_value(key: SettingKey, settings: &Settings) -> String {
    match key {
        SettingKey::LanguageFile => settings.language_file.clone(),
        SettingKey::WhiteTrayIcon => bool_value(settings.white_tray_icon),
        SettingKey::SnapshotPath => settings.snapshot_path.clone(),
        SettingKey::EraserIcon => bool_value(settings.eraser_icon),
        SettingKey::PanIcon => bool_value(settings.pan_icon),
        SettingKey::PointerIcon => bool_value(settings.pointer_icon),
        SettingKey::PenWidthIcon => bool_value(settings.pen_width_icon),
        SettingKey::InkVisibleIcon => bool_value(settings.ink_visible_icon),
        SettingKey::SnapshotIcon => bool_value(settings.snapshot_icon),
        SettingKey::UndoIcon => bool_value(settings.undo_icon),
        SettingKey::ClearIcon => bool_value(settings.clear_icon),
        SettingKey::CanvasCursor => match settings.canvas_cursor {
            CanvasCursor::Arrow => "0".to_owned(),
            CanvasCursor::PenTip => "1".to_owned(),
        },
        SettingKey::AllowDraggingToolbar => bool_value(settings.allow_dragging_toolbar),
        SettingKey::AllowHotkeyInPointerMode => bool_value(settings.allow_hotkey_in_pointer_mode),
        SettingKey::ToolbarLeft => settings.toolbar_left.to_string(),
        SettingKey::ToolbarTop => settings.toolbar_top.to_string(),
        SettingKey::CloseOnSnap => match settings.close_on_snap {
            CloseOnSnap::True => "True".to_owned(),
            CloseOnSnap::False => "False".to_owned(),
            CloseOnSnap::BlankOnly => "BlankOnly".to_owned(),
        },
        SettingKey::AlwaysHideToolbar => bool_value(settings.always_hide_toolbar),
        SettingKey::ToolbarHeight => settings.toolbar_height.to_string(),
        SettingKey::Pen { index, field } => {
            let pen = &settings.pens[index];
            match field {
                PenField::Enabled => bool_value(pen.enabled),
                PenField::Red => pen.color.red.to_string(),
                PenField::Green => pen.color.green.to_string(),
                PenField::Blue => pen.color.blue.to_string(),
                PenField::Alpha => pen.alpha.to_string(),
                PenField::Width => pen.width.to_string(),
                PenField::Hotkey => settings.hotkeys.pens[index].to_string(),
            }
        }
        SettingKey::HotkeyGlobal => settings.hotkeys.global.to_string(),
        SettingKey::HotkeyEraser => settings.hotkeys.eraser.to_string(),
        SettingKey::HotkeyInkVisible => settings.hotkeys.ink_visible.to_string(),
        SettingKey::HotkeyPointer => settings.hotkeys.pointer.to_string(),
        SettingKey::HotkeyPan => settings.hotkeys.pan.to_string(),
        SettingKey::HotkeyUndo => settings.hotkeys.undo.to_string(),
        SettingKey::HotkeyRedo => settings.hotkeys.redo.to_string(),
        SettingKey::HotkeySnapshot => settings.hotkeys.snapshot.to_string(),
        SettingKey::HotkeySnapPin => settings.hotkeys.snap_pin.to_string(),
        SettingKey::HotkeyClear => settings.hotkeys.clear.to_string(),
    }
}

fn bool_value(value: bool) -> String {
    if value { "True" } else { "False" }.to_owned()
}
