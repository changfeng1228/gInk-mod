//! The settings window.
//!
//! Scope for this step is deliberately small -- the two things that cannot be
//! edited anywhere else yet:
//!
//! 1. Hotkey rebinding: every action plus the ten pens.
//! 2. Autostart: a checkbox backed by a Startup-folder shortcut.
//!
//! Everything else (pen colours, alpha, widths, language) already round-trips
//! through the INI layer and keeps working, so it is not part of this window.
//!
//! # Shape
//!
//! `show` creates a modal top-level window with two Win32 child controls per
//! hotkey row (a label and an edit box) plus the autostart checkbox. Editing a
//! box parses the text immediately and reports a conflict or an invalid
//! combination in the status line, without committing anything until OK is
//! pressed -- so Cancel and the close box both leave the configuration exactly
//! as it was.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject, HFONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetDlgItem, GetMessageW, GetWindowTextW, IsDialogMessageW, IsWindow, MSG, RegisterClassExW,
    SW_SHOW, SendMessageW, SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage,
    WM_COMMAND, WM_CREATE, WM_DESTROY, WM_SETFONT, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR};

use crate::hotkeys::{self, Command};
use crate::options;
use crate::settings::{Hotkey, Settings};
use crate::startup;

const ID_OK: i32 = 1;
const ID_CANCEL: i32 = 2;
const ID_AUTOSTART: i32 = 3;
const ID_STATUS: i32 = 4;
const ROW_BASE: i32 = 100;
const ROWS_PER_ENTRY: i32 = 2;

/// The message the window procedure builds its children on.
///
/// Exposed so a regression test can assert it is `WM_CREATE`: using
/// `WM_INITDIALOG` here silently produces an empty window, because
/// `CreateWindowExW` does not send it.
pub fn control_message_for_creation() -> u32 {
    WM_CREATE
}

const WINDOW_CLASS: &str = "plnk_options_window";
const WINDOW_TITLE: &str = "plnk 设置";

const DIALOG_WIDTH: i32 = 780;
const DIALOG_HEIGHT: i32 = 660;
const ROW_HEIGHT: i32 = 26;
const LABEL_WIDTH: i32 = 260;
const EDIT_WIDTH: i32 = 200;
const MARGIN: i32 = 16;

/// One editable row plus the id of its edit control.
struct RowState {
    command: Command,
    edit_id: i32,
}

/// Everything the window procedure needs.
struct DialogState {
    working: Settings,
    rows: Vec<RowState>,
    status: String,
    autostart: bool,
    accepted: bool,
}

thread_local! {
    /// The window procedure is a bare `extern "system"` function with no
    /// closure environment, so the state is reached through a thread-local
    /// slot. Only one settings window exists at a time (the shell is
    /// single-instance and the tray menu is modal) and the slot is cleared when
    /// `show` returns.
    static STATE: RefCell<Option<Box<DialogState>>> = const { RefCell::new(None) };
}

/// Shows the modal settings window.
///
/// Returns `Ok(Some(settings))` when the user pressed OK with a valid
/// configuration, `Ok(None)` when the window was cancelled or closed, and
/// `Err` when the window could not be created at all.
pub fn show(owner: Option<HWND>, settings: &Settings) -> Result<Option<Settings>, String> {
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|error| format!("cannot get the module instance: {error}"))?;

    let rows = options::rebindable_hotkeys(settings)
        .into_iter()
        .enumerate()
        .map(|(index, row)| RowState {
            command: row.command,
            edit_id: ROW_BASE + index as i32 * ROWS_PER_ENTRY,
        })
        .collect::<Vec<_>>();

    STATE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(DialogState {
            working: settings.clone(),
            rows,
            status: String::new(),
            autostart: startup::is_enabled(),
            accepted: false,
        }));
    });

    let class = HSTRING::from(WINDOW_CLASS);
    let title = HSTRING::from(WINDOW_TITLE);
    let outcome = run_window(instance.0, &class, &title, owner);

    let accepted = STATE
        .with(|slot| slot.borrow_mut().take())
        .and_then(|state| {
            if state.accepted {
                Some(state.working)
            } else {
                None
            }
        });
    outcome.map(|()| accepted)
}

fn run_window(
    instance: *mut core::ffi::c_void,
    class: &HSTRING,
    title: &HSTRING,
    owner: Option<HWND>,
) -> Result<(), String> {
    let window_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: windows::Win32::Foundation::HINSTANCE(instance),
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassExW(&window_class) };
    if atom == 0 {
        let error = unsafe { windows::Win32::Foundation::GetLastError() };
        if error != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!(
                "cannot register the settings window class: {error:?}"
            ));
        }
    }

    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            class,
            title,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            220,
            160,
            DIALOG_WIDTH,
            DIALOG_HEIGHT,
            owner,
            None,
            Some(windows::Win32::Foundation::HINSTANCE(instance)),
            None,
        )
    }
    .map_err(|error| format!("cannot create the settings window: {error}"))?;

    // Modal against the shell: the owner is disabled for the lifetime of this
    // loop so the tray menu cannot re-enter the shell while settings are open.
    if let Some(owner) = owner {
        let _ = unsafe { EnableWindow(owner, false) };
    }
    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 == 0 || result.0 == -1 {
            break;
        }
        let handled = unsafe { IsDialogMessageW(hwnd, &message).as_bool() };
        if !handled {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            break;
        }
    }

    if let Some(owner) = owner {
        unsafe {
            let _ = EnableWindow(owner, true);
            let _ = SetForegroundWindow(owner);
        }
    }
    Ok(())
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // A plain `CreateWindowExW` window never receives `WM_INITDIALOG` (that
        // message belongs to the `DialogBox*` family), so building the children
        // there left the frame empty. `WM_CREATE` is the creation message every
        // window gets, right before it is first shown.
        WM_CREATE => {
            unsafe { build_controls(hwnd) };
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            unsafe { handle_command(hwnd, id) };
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe fn build_controls(hwnd: HWND) {
    let font = unsafe { default_ui_font() };
    let snapshot = STATE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|state| (state.working.clone(), state.autostart, state.rows.len()))
    });
    let Some((working, autostart, row_count)) = snapshot else {
        return;
    };
    let rows = options::rebindable_hotkeys(&working);
    let pen_start = rows.len().saturating_sub(10);

    let mut y = MARGIN;
    for (index, row) in rows.iter().enumerate().take(row_count) {
        let label_text = if row.label.is_empty() {
            format!("笔 {} (Pen {})", index - pen_start, index - pen_start)
        } else {
            row.label.to_owned()
        };
        unsafe {
            create_control(
                hwnd,
                ControlSpec {
                    class: "STATIC",
                    text: &label_text,
                    x: MARGIN,
                    y,
                    width: LABEL_WIDTH,
                    height: 20,
                    id: -1,
                },
                font,
            );
        }
        let edit_id = ROW_BASE + index as i32 * ROWS_PER_ENTRY;
        let edit = unsafe {
            create_control(
                hwnd,
                ControlSpec {
                    class: "EDIT",
                    text: "",
                    x: MARGIN + LABEL_WIDTH + 8,
                    y: y - 3,
                    width: EDIT_WIDTH,
                    height: 22,
                    id: edit_id,
                },
                font,
            )
        };
        let current = options::current_hotkey(&working, row.command);
        let text = HSTRING::from(hotkeys::format_hotkey(&current));
        unsafe {
            let _ = SetWindowTextW(edit, &text);
            let _ = SendMessageW(
                edit,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        y += ROW_HEIGHT;
    }

    y += 10;
    let check = unsafe {
        create_control(
            hwnd,
            ControlSpec {
                class: "BUTTON",
                text: "开机自启动 (Start with Windows)",
                x: MARGIN,
                y,
                width: 300,
                height: 24,
                id: ID_AUTOSTART,
            },
            font,
        )
    };
    let checked = if autostart {
        BST_CHECKED
    } else {
        BST_UNCHECKED
    };
    unsafe {
        SendMessageW(
            check,
            BM_SETCHECK,
            Some(WPARAM(checked.0 as usize)),
            Some(LPARAM(0)),
        )
    };
    y += 36;

    unsafe {
        create_control(
            hwnd,
            ControlSpec {
                class: "STATIC",
                text: "",
                x: MARGIN,
                y,
                width: DIALOG_WIDTH - 2 * MARGIN - 40,
                height: 22,
                id: ID_STATUS,
            },
            font,
        );
    }
    y += 40;

    unsafe {
        create_control(
            hwnd,
            ControlSpec {
                class: "BUTTON",
                text: "OK",
                x: DIALOG_WIDTH - 2 * MARGIN - 200,
                y,
                width: 90,
                height: 28,
                id: ID_OK,
            },
            font,
        );
        create_control(
            hwnd,
            ControlSpec {
                class: "BUTTON",
                text: "Cancel",
                x: DIALOG_WIDTH - 2 * MARGIN - 100,
                y,
                width: 90,
                height: 28,
                id: ID_CANCEL,
            },
            font,
        );
    }
}

/// Placement and identity of one child control. Grouped into a struct so the
/// builder does not take eight positional coordinates.
struct ControlSpec<'a> {
    class: &'a str,
    text: &'a str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
}

unsafe fn create_control(parent: HWND, spec: ControlSpec<'_>, font: HFONT) -> HWND {
    let ControlSpec {
        class,
        text,
        x,
        y,
        width,
        height,
        id,
    } = spec;
    let class = HSTRING::from(class);
    let text = HSTRING::from(text);
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            &class,
            &text,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER,
            x,
            y,
            width,
            height,
            Some(parent),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU(
                id as isize as *mut core::ffi::c_void,
            )),
            None,
            None,
        )
    };
    let hwnd = hwnd.unwrap_or_default();
    if id >= 0 {
        let _ = unsafe {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            )
        };
    }
    hwnd
}

unsafe fn default_ui_font() -> HFONT {
    HFONT(unsafe { GetStockObject(DEFAULT_GUI_FONT) }.0)
}

unsafe fn handle_command(hwnd: HWND, id: i32) {
    match id {
        ID_OK => unsafe { accept(hwnd) },
        ID_CANCEL => {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        ID_AUTOSTART => unsafe { toggle_autostart(hwnd) },
        edit_id if edit_id >= ROW_BASE => unsafe { read_edited_row(hwnd, edit_id) },
        _ => {}
    }
}

unsafe fn toggle_autostart(hwnd: HWND) {
    let Some(check) = (unsafe { GetDlgItem(Some(hwnd), ID_AUTOSTART) }).ok() else {
        return;
    };
    let checked = unsafe { SendMessageW(check, BM_GETCHECK, None, None) }.0 as u32;
    let enable = checked == BST_CHECKED.0;

    if let Err(error) = startup::set_enabled(enable) {
        // Roll the checkbox back: the UI must never claim a state the file
        // system refused to store.
        let revert = if enable { BST_UNCHECKED } else { BST_CHECKED };
        unsafe {
            SendMessageW(
                check,
                BM_SETCHECK,
                Some(WPARAM(revert.0 as usize)),
                Some(LPARAM(0)),
            )
        };
        set_status(&format!("自启动设置失败：{error}"));
    } else {
        set_status("");
    }
    STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.autostart = enable && startup::is_enabled();
        }
    });
    unsafe { refresh_status(hwnd) };
}

unsafe fn read_edited_row(hwnd: HWND, edit_id: i32) {
    let Some(edit) = (unsafe { GetDlgItem(Some(hwnd), edit_id) }).ok() else {
        return;
    };
    let mut buffer = [0u16; 128];
    let length = unsafe { GetWindowTextW(edit, &mut buffer) };
    let text = String::from_utf16_lossy(&buffer[..length as usize]);
    let message = STATE.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(state) = guard.as_mut() else {
            return String::new();
        };
        let Some(command) = state
            .rows
            .iter()
            .find(|row| row.edit_id == edit_id)
            .map(|row| row.command)
        else {
            return String::new();
        };
        match Hotkey::parse(text.trim()) {
            None => format!("无法识别的组合：{text}"),
            Some(hotkey) => match options::apply_hotkey(&mut state.working, command, hotkey) {
                Ok(()) => String::new(),
                Err(error) => error.message,
            },
        }
    });
    set_status(&message);
    unsafe { refresh_status(hwnd) };
}

fn set_status(message: &str) {
    STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.status = message.to_owned();
        }
    });
}

unsafe fn refresh_status(hwnd: HWND) {
    let status = STATE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|state| state.status.clone())
            .unwrap_or_default()
    });
    if let Ok(control) = unsafe { GetDlgItem(Some(hwnd), ID_STATUS) } {
        let text = HSTRING::from(status);
        let _ = unsafe { SetWindowTextW(control, &text) };
    }
}

unsafe fn accept(hwnd: HWND) {
    STATE.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.accepted = true;
        }
    });
    let _ = unsafe { DestroyWindow(hwnd) };
}
