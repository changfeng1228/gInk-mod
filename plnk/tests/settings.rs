use std::fs;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use plnk::settings::{CloseOnSnap, Hotkey, Rgb, Settings};
use plnk::settings_store::SettingsStore;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy")
}

fn fixture(subdir: &str) -> PathBuf {
    fixtures_root().join(subdir)
}

fn temp_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "plnk-settings-{}-{}-{}",
        std::process::id(),
        label,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

/// Builds a store that imports from `fixture_dir` and would save to a fresh
/// temporary path, never to the fixture directory.
fn store_for(fixture_dir: &Path, label: &str) -> SettingsStore {
    let settings_path = temp_dir(label).join("settings.ini");
    SettingsStore::new(settings_path, vec![fixture_dir.to_path_buf()])
}

fn snapshot_dir(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            entries.push((
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            ));
        }
    }
    entries.sort();
    entries
}

fn open_legacy(path: &Path) -> BufReader<fs::File> {
    BufReader::new(fs::File::open(path).unwrap())
}

#[test]
fn legacy_defaults_match_gink() {
    let dir = fixture("default");
    let before = snapshot_dir(&dir);
    let mut store = store_for(&dir, "defaults");
    let import = store.load_or_import().unwrap();
    assert!(
        import.warnings.is_empty(),
        "unexpected warnings: {:?}",
        import.warnings
    );
    let settings = &import.settings;

    assert_eq!(settings.language_file, "en-us");
    assert!(!settings.white_tray_icon);
    assert_eq!(settings.snapshot_path, "%USERPROFILE%/Pictures/gInk/");
    assert!(settings.eraser_icon);
    assert!(settings.pan_icon);
    assert!(settings.pointer_icon);
    assert!(!settings.pen_width_icon);
    assert!(settings.ink_visible_icon);
    assert!(settings.snapshot_icon);
    assert!(settings.undo_icon);
    assert!(settings.clear_icon);
    assert_eq!(settings.canvas_cursor, plnk::settings::CanvasCursor::Arrow);
    assert!(!settings.allow_dragging_toolbar);
    assert!(!settings.allow_hotkey_in_pointer_mode);
    assert_eq!(settings.toolbar_left, 0);
    assert_eq!(settings.toolbar_top, 0);
    assert_eq!(settings.close_on_snap, CloseOnSnap::BlankOnly);
    assert!(!settings.always_hide_toolbar);
    assert_eq!(settings.toolbar_height, 0.05);

    // enabled, R, G, B, alpha (opacity), width -- docs/acceptance/legacy-baseline.md 4.1
    let expected_pens = [
        (false, 80, 80, 80, 255, 80),
        (true, 225, 60, 60, 255, 80),
        (true, 30, 110, 200, 255, 80),
        (true, 250, 180, 55, 255, 80),
        (true, 120, 175, 70, 255, 80),
        (true, 235, 125, 15, 80, 500),
        (false, 230, 230, 230, 255, 80),
        (false, 240, 140, 200, 255, 80),
        (false, 25, 180, 175, 255, 80),
        (false, 145, 70, 160, 80, 500),
    ];
    for (index, expected) in expected_pens.iter().enumerate() {
        let pen = &settings.pens[index];
        assert_eq!(pen.enabled, expected.0, "pen {index} enabled");
        assert_eq!(
            pen.color,
            Rgb::new(expected.1, expected.2, expected.3),
            "pen {index} colour"
        );
        assert_eq!(pen.alpha, expected.4, "pen {index} alpha");
        assert_eq!(pen.width, expected.5, "pen {index} width");
    }

    let hotkeys = &settings.hotkeys;
    assert_eq!(hotkeys.global.to_string(), "Ctrl + Alt + G");
    assert_eq!(hotkeys.eraser.to_string(), "E");
    assert_eq!(hotkeys.ink_visible.to_string(), "V");
    assert_eq!(hotkeys.pointer.to_string(), "None");
    assert_eq!(hotkeys.pan.to_string(), "None");
    assert_eq!(hotkeys.undo.to_string(), "Ctrl + Z");
    assert_eq!(hotkeys.redo.to_string(), "Ctrl + Y");
    assert_eq!(hotkeys.snapshot.to_string(), "Ctrl + Alt + S");
    assert_eq!(hotkeys.snap_pin.to_string(), "Ctrl + Alt + Shift + P");
    assert_eq!(hotkeys.clear.to_string(), "None");
    for (index, hotkey) in hotkeys.pens.iter().enumerate() {
        assert_eq!(hotkey.to_string(), index.to_string(), "pen {index} hotkey");
    }

    assert_eq!(
        snapshot_dir(&dir),
        before,
        "import must not write legacy files"
    );
}

#[test]
fn shipped_and_missing_file_defaults_are_distinct() {
    // The "no file at all" scenario: empty readers must yield Settings::defaults().
    let mut config = Cursor::new(Vec::<u8>::new());
    let mut pens = Cursor::new(Vec::<u8>::new());
    let mut hotkeys = Cursor::new(Vec::<u8>::new());
    let missing = Settings::from_legacy(&mut config, &mut pens, &mut hotkeys).unwrap();
    assert_eq!(missing.settings, Settings::defaults());
    assert!(missing.warnings.is_empty());

    let missing_dir = fixture("malformed/missing-file");
    let mut missing_store = store_for(&missing_dir, "missing-files");
    let missing_import = missing_store.load_or_import().unwrap();
    assert_eq!(missing_import.settings, Settings::defaults());
    assert!(missing_import.warnings.is_empty());

    // The shipped factory files.
    let dir = fixture("default");
    let mut config = open_legacy(&dir.join("config.ini"));
    let mut pens = open_legacy(&dir.join("pens.ini"));
    let mut hotkeys = open_legacy(&dir.join("hotkeys.ini"));
    let shipped = Settings::from_legacy(&mut config, &mut pens, &mut hotkeys).unwrap();
    assert!(shipped.warnings.is_empty());
    let shipped = shipped.settings;
    let defaults = Settings::defaults();

    // Pen 3 and Pen 7 colours differ between the factory files and the code defaults.
    assert_eq!(shipped.pens[3].color, Rgb::new(250, 180, 55));
    assert_eq!(defaults.pens[3].color, Rgb::new(235, 180, 55));
    assert_eq!(shipped.pens[7].color, Rgb::new(240, 140, 200));
    assert_eq!(defaults.pens[7].color, Rgb::new(250, 140, 200));
    assert_ne!(shipped.pens[3], defaults.pens[3]);
    assert_ne!(shipped.pens[7], defaults.pens[7]);

    assert_eq!(shipped.toolbar_height, 0.05);
    assert_eq!(defaults.toolbar_height, 0.06);
    assert!(!shipped.allow_dragging_toolbar);
    assert!(defaults.allow_dragging_toolbar);
    assert!(!shipped.allow_hotkey_in_pointer_mode);
    assert!(defaults.allow_hotkey_in_pointer_mode);

    assert_eq!(shipped.hotkeys.eraser.to_string(), "E");
    assert_eq!(defaults.hotkeys.eraser.to_string(), "None");
    assert_eq!(shipped.hotkeys.ink_visible.to_string(), "V");
    assert_eq!(defaults.hotkeys.ink_visible.to_string(), "None");
    assert_eq!(shipped.hotkeys.undo.to_string(), "Ctrl + Z");
    assert_eq!(defaults.hotkeys.undo.to_string(), "None");
    assert_eq!(shipped.hotkeys.redo.to_string(), "Ctrl + Y");
    assert_eq!(defaults.hotkeys.redo.to_string(), "None");
    assert_eq!(shipped.hotkeys.snapshot.to_string(), "Ctrl + Alt + S");
    assert_eq!(defaults.hotkeys.snapshot.to_string(), "None");
    assert_eq!(shipped.hotkeys.pens[0].to_string(), "0");
    assert_eq!(defaults.hotkeys.pens[0].to_string(), "None");
    assert_eq!(shipped.hotkeys.pens[9].to_string(), "9");
    assert_eq!(defaults.hotkeys.pens[9].to_string(), "None");

    // Both scenarios agree on the always-registered global hotkeys.
    assert_eq!(shipped.hotkeys.global, defaults.hotkeys.global);
    assert_eq!(shipped.hotkeys.snap_pin, defaults.hotkeys.snap_pin);
    assert_eq!(shipped.hotkeys.global.to_string(), "Ctrl + Alt + G");
    assert_eq!(
        shipped.hotkeys.snap_pin.to_string(),
        "Ctrl + Alt + Shift + P"
    );
}

#[test]
fn current_pen_and_hotkey_values_survive_import() {
    let dir = fixture("current");
    let before = snapshot_dir(&dir);
    let mut store = store_for(&dir, "current");
    let import = store.load_or_import().unwrap();
    assert!(
        import.warnings.is_empty(),
        "unexpected warnings: {:?}",
        import.warnings
    );
    let settings = &import.settings;

    assert_eq!(settings.language_file, "zh-cn");
    assert_eq!(settings.snapshot_path, "%USERPROFILE%/Pictures/gInk/");
    assert!(!settings.white_tray_icon);
    assert_eq!(settings.close_on_snap, CloseOnSnap::BlankOnly);
    assert_eq!(settings.toolbar_height, 0.05);

    assert_eq!(settings.pens[3].color, Rgb::new(235, 180, 55));
    assert_eq!(settings.pens[7].color, Rgb::new(240, 140, 200));

    let hotkeys = &settings.hotkeys;
    assert_eq!(hotkeys.global.to_string(), "Ctrl + Alt + Z");
    assert_eq!(hotkeys.eraser.to_string(), "E");
    assert_eq!(hotkeys.ink_visible.to_string(), "V");
    assert_eq!(hotkeys.pointer.to_string(), "None");
    assert_eq!(hotkeys.pan.to_string(), "None");
    assert_eq!(hotkeys.undo.to_string(), "Ctrl + Z");
    assert_eq!(hotkeys.redo.to_string(), "Ctrl + Y");
    assert_eq!(hotkeys.snapshot.to_string(), "Ctrl + Alt + S");
    assert_eq!(hotkeys.snap_pin.to_string(), "Ctrl + Alt + A");
    assert_eq!(hotkeys.clear.to_string(), "Ctrl + Q");
    for (index, hotkey) in hotkeys.pens.iter().enumerate() {
        assert_eq!(hotkey.to_string(), index.to_string(), "pen {index} hotkey");
    }

    assert_eq!(
        snapshot_dir(&dir),
        before,
        "import must not write legacy files"
    );
}

#[test]
fn comments_and_unknown_lines_survive_save() {
    let dir = fixture("malformed/syntax");
    let before = snapshot_dir(&dir);
    let temp = temp_dir("syntax-save");
    let settings_path = temp.join("settings.ini");
    let mut store = SettingsStore::new(settings_path.clone(), vec![dir.clone()]);

    let mut import = store.load_or_import().unwrap();
    assert!(
        import.warnings.is_empty(),
        "unexpected warnings: {:?}",
        import.warnings
    );
    assert_eq!(import.settings.language_file, "zh-cn");
    assert!(!import.settings.eraser_icon);
    assert!(!import.settings.white_tray_icon);
    assert_eq!(import.settings.toolbar_top, 42);

    import.settings.white_tray_icon = true;
    import.settings.toolbar_top = 99;
    store.save(&import.settings).unwrap();

    assert_eq!(
        snapshot_dir(&dir),
        before,
        "save must not write legacy files"
    );

    let saved = fs::read_to_string(&settings_path).unwrap();
    for preserved in [
        "; semicolon is not a legacy comment marker, but has no '=' so it is skipped",
        "# hash comment",
        "- dash comment",
        "% percent comment",
        "' quote comment",
        "/ slash comment",
        "! bang comment",
        "[SECTION]",
        "Custom_Key = keep me",
        "Weird = a=b",
    ] {
        assert!(
            saved.contains(preserved),
            "missing line {preserved:?} in saved file:\n{saved}"
        );
    }
    assert!(
        saved.contains("White_Tray_Icon = True"),
        "saved file:\n{saved}"
    );
    assert!(saved.contains("Toolbar_Top = 99"), "saved file:\n{saved}");

    let reloaded = store.load_or_import().unwrap();
    assert_eq!(reloaded.settings, import.settings);
    assert!(
        reloaded.warnings.is_empty(),
        "saved file must re-import cleanly"
    );
    assert!(
        !settings_path.starts_with(&dir),
        "save target must be outside the legacy directory"
    );
}

#[test]
fn malformed_values_fall_back_without_writing_legacy() {
    let dir = fixture("malformed/invalid-values");
    let before = snapshot_dir(&dir);
    let mut store = store_for(&dir, "invalid-values");
    let import = store.load_or_import().unwrap();
    let settings = &import.settings;

    // White_Tray_Icon = True followed by "maybe": every value other than True/1/ON is false.
    assert!(!settings.white_tray_icon);
    assert!(settings.eraser_icon);
    assert!(!settings.pan_icon);
    assert!(!settings.pointer_icon);
    assert!(settings.pen_width_icon);
    assert!(!settings.ink_visible_icon);
    assert!(settings.snapshot_icon);
    assert!(settings.undo_icon);
    assert!(!settings.clear_icon);
    assert_eq!(settings.canvas_cursor, plnk::settings::CanvasCursor::Arrow);
    assert!(settings.allow_dragging_toolbar);
    assert!(settings.allow_hotkey_in_pointer_mode);
    assert_eq!(settings.toolbar_left, 0);
    assert_eq!(settings.toolbar_top, 0);
    assert_eq!(settings.toolbar_height, 0.06);
    assert_eq!(settings.close_on_snap, CloseOnSnap::BlankOnly);
    assert!(!settings.always_hide_toolbar);
    assert_eq!(settings.snapshot_path, "%USERPROFILE%/Pictures/gInk/");

    assert_eq!(settings.pens[0].color, Rgb::new(80, 80, 80));
    assert_eq!(settings.pens[1].color, Rgb::new(225, 60, 200));
    assert_eq!(settings.pens[1].alpha, 255);
    assert_eq!(settings.pens[2].width, 80);
    assert_eq!(settings.pens[3].width, 80);
    // D-023: PEN10_RED is rejected, so Pen1 keeps its own red.
    assert_eq!(settings.pens[1].color.red, 225);

    let mut warned: Vec<&str> = import
        .warnings
        .iter()
        .map(|warning| warning.key.as_str())
        .collect();
    warned.sort_unstable();
    let mut expected = vec![
        "Allow_Dragging_Toolbar",
        "Allow_Hotkey_In_Pointer_Mode",
        "Always_Hide_Toolbar",
        "Canvas_Cursor",
        "Close_On_Snap",
        "Eraser_Icon",
        "PEN10_RED",
        "Pen0_Green",
        "Pen0_Red",
        "Pen1_Alpha",
        "Pen2_Width",
        "Pen3_Width",
        "Snapshot_Icon",
        "Toolbar_Height",
        "Toolbar_Left",
        "Toolbar_Top",
        "White_Tray_Icon",
    ];
    expected.sort_unstable();
    assert_eq!(warned, expected);
    assert_eq!(
        snapshot_dir(&dir),
        before,
        "import must not write legacy files"
    );

    // Files that exist but omit keys: present values win, missing keys use code defaults.
    let dir = fixture("malformed/missing-keys");
    let before = snapshot_dir(&dir);
    let mut store = store_for(&dir, "missing-keys");
    let import = store.load_or_import().unwrap();
    assert!(
        import.warnings.is_empty(),
        "unexpected warnings: {:?}",
        import.warnings
    );
    let settings = &import.settings;
    assert_eq!(settings.language_file, "zh-cn");
    assert!(!settings.pens[1].enabled);
    assert_eq!(settings.pens[1].color, Rgb::new(200, 10, 20));
    assert_eq!(settings.pens[1].alpha, 128);
    assert_eq!(settings.pens[1].width, 300);
    assert_eq!(settings.pens[0], Settings::defaults().pens[0]);
    assert_eq!(settings.toolbar_height, 0.06);
    assert!(settings.allow_dragging_toolbar);
    assert_eq!(settings.hotkeys.global.to_string(), "Ctrl + Alt + G");
    assert_eq!(settings.hotkeys.eraser.to_string(), "None");
    assert_eq!(
        snapshot_dir(&dir),
        before,
        "import must not write legacy files"
    );
}

#[test]
fn all_hotkey_families_roundtrip() {
    for number in 1..=12u8 {
        let text = format!("F{number}");
        assert_eq!(Hotkey::parse(&text).unwrap().to_string(), text);
    }
    for number in 0..=9u8 {
        let text = format!("NumPad{number}");
        assert_eq!(Hotkey::parse(&text).unwrap().to_string(), text);
    }
    for letter in 'A'..='Z' {
        let text = letter.to_string();
        assert_eq!(Hotkey::parse(&text).unwrap().to_string(), text);
    }
    for digit in '0'..='9' {
        let text = digit.to_string();
        assert_eq!(Hotkey::parse(&text).unwrap().to_string(), text);
    }
    for (text, canonical) in [
        ("None", "None"),
        ("Ctrl + Alt + G", "Ctrl + Alt + G"),
        ("Ctrl+Shift+Z", "Ctrl + Shift + Z"),
        ("Control + Alt + F10", "Ctrl + Alt + F10"),
        ("Win + F12", "Win + F12"),
        ("Alt + NumPad0", "Alt + NumPad0"),
        ("shift + alt + numpad9", "Alt + Shift + NumPad9"),
    ] {
        let hotkey = Hotkey::parse(text).unwrap();
        assert_eq!(hotkey.to_string(), canonical, "parsing {text:?}");
        assert_eq!(
            Hotkey::parse(&hotkey.to_string()).unwrap(),
            hotkey,
            "round trip {text:?}"
        );
    }
    for invalid in ["Ctrl +", "F13", "F0", "Ctrl + A + B", "", "Shift +", "Ctrl"] {
        assert!(
            Hotkey::parse(invalid).is_none(),
            "{invalid:?} must be rejected"
        );
    }

    let dir = fixture("malformed/hotkeys");
    let before = snapshot_dir(&dir);
    let mut store = store_for(&dir, "hotkeys");
    let import = store.load_or_import().unwrap();
    let settings = &import.settings;
    let hotkeys = &settings.hotkeys;

    assert_eq!(hotkeys.global.to_string(), "Ctrl + Alt + G");
    assert_eq!(hotkeys.eraser.to_string(), "F1");
    assert_eq!(hotkeys.ink_visible.to_string(), "F2");
    assert_eq!(hotkeys.pointer.to_string(), "F3");
    assert_eq!(hotkeys.pan.to_string(), "F4");
    assert_eq!(hotkeys.undo.to_string(), "F5");
    assert_eq!(hotkeys.redo.to_string(), "F6");
    assert_eq!(hotkeys.snapshot.to_string(), "F7");
    assert_eq!(hotkeys.snap_pin.to_string(), "Ctrl + Alt + Shift + P");
    assert_eq!(hotkeys.clear.to_string(), "None");
    let expected_pens = [
        "F8", "F9", "F10", "F11", "F12", "NumPad0", "NumPad5", "NumPad9", "None", "None",
    ];
    for (index, expected) in expected_pens.iter().enumerate() {
        assert_eq!(
            hotkeys.pens[index].to_string(),
            *expected,
            "pen {index} hotkey"
        );
    }

    let warned: Vec<&str> = import
        .warnings
        .iter()
        .map(|warning| warning.key.as_str())
        .collect();
    assert_eq!(warned, ["Pen8_Hotkey", "Pen9_Hotkey"]);
    assert_eq!(hotkeys.pens[8], Hotkey::default());
    assert_eq!(hotkeys.pens[9], Hotkey::default());

    store.save(settings).unwrap();
    let reloaded = store.load_or_import().unwrap();
    assert_eq!(reloaded.settings, *settings);
    assert!(reloaded.warnings.is_empty());
    assert_eq!(
        snapshot_dir(&dir),
        before,
        "import and save must not write legacy files"
    );
}

#[test]
fn legacy_lookup_prefers_the_first_search_directory() {
    let first = temp_dir("lookup-first");
    let second = temp_dir("lookup-second");
    fs::write(first.join("config.ini"), "Toolbar_Top = 11\n").unwrap();
    fs::write(second.join("config.ini"), "Toolbar_Top = 22\n").unwrap();

    let mut store = SettingsStore::new(
        temp_dir("lookup-store-a").join("settings.ini"),
        vec![first.clone(), second.clone()],
    );
    assert_eq!(store.load_or_import().unwrap().settings.toolbar_top, 11);

    let mut reversed = SettingsStore::new(
        temp_dir("lookup-store-b").join("settings.ini"),
        vec![second, first],
    );
    assert_eq!(reversed.load_or_import().unwrap().settings.toolbar_top, 22);
}

#[test]
fn existing_settings_file_takes_precedence_over_legacy_import() {
    let dir = temp_dir("settings-precedence");
    let settings_path = dir.join("settings.ini");
    fs::write(&settings_path, "Language_File = de\nToolbar_Top = 5\n").unwrap();

    let mut store = SettingsStore::new(settings_path, vec![fixture("default")]);
    let import = store.load_or_import().unwrap();
    assert!(import.warnings.is_empty());
    assert_eq!(import.settings.language_file, "de");
    assert_eq!(import.settings.toolbar_top, 5);
    // The factory hotkeys must not be imported when the new settings file exists.
    assert_eq!(import.settings.hotkeys.eraser.to_string(), "None");
    assert_eq!(import.settings.toolbar_height, 0.06);
}

#[test]
fn saving_a_fresh_install_writes_a_reloadable_document() {
    let dir = temp_dir("fresh-save");
    let settings_path = dir.join("data").join("settings.ini");
    let mut store = SettingsStore::new(
        settings_path.clone(),
        vec![fixture("malformed/missing-file")],
    );

    let import = store.load_or_import().unwrap();
    assert_eq!(import.settings, Settings::defaults());
    store.save(&import.settings).unwrap();

    assert!(
        settings_path.is_file(),
        "save must create the data directory"
    );
    let reloaded = store.load_or_import().unwrap();
    assert_eq!(reloaded.settings, Settings::defaults());
    assert!(reloaded.warnings.is_empty());
}

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("simulated read failure"))
    }
}

impl BufRead for FailingReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("simulated read failure"))
    }

    fn consume(&mut self, _amount: usize) {}
}

#[test]
fn unreadable_legacy_stream_reports_the_file_name() {
    let mut config = Cursor::new(Vec::<u8>::new());
    let mut hotkeys = Cursor::new(Vec::<u8>::new());
    let error = Settings::from_legacy(&mut config, &mut FailingReader, &mut hotkeys).unwrap_err();
    assert!(error.to_string().contains("pens.ini"), "error was: {error}");
}
