//! Tray icon and popup menu.
//!
//! The icon lives in the notification area and is owned by the hidden
//! application window created in `app.rs`. Removing the icon is part of the
//! exit path: [`TrayIcon`]'s `Drop` and the explicit [`TrayIcon::remove`] both
//! call `Shell_NotifyIconW(NIM_DELETE)`, so no stale icon survives an exit.
//!
//! The stock application icon is used until the icon assets task provides the
//! real gInk-derived tray icons (including `White_Tray_Icon`).

use std::fmt;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, IDI_APPLICATION, LoadIconW,
    MF_STRING, PostMessageW, SetForegroundWindow, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{Error as WindowsError, w};

/// Command id posted by the tray menu. The same id the user selects in the
/// popup produces the `WM_COMMAND` handled by the shell window.
pub const MENU_EXIT: usize = 1;

const TRAY_ICON_ID: u32 = 1;
const TOOLTIP: &str = "plnk";

#[derive(Debug)]
pub struct TrayError {
    pub context: &'static str,
    pub source: WindowsError,
}

impl TrayError {
    fn from_thread(context: &'static str) -> Self {
        Self {
            context,
            source: WindowsError::from_thread(),
        }
    }
}

impl fmt::Display for TrayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "tray operation failed while {}: {}",
            self.context, self.source
        )
    }
}

impl std::error::Error for TrayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The notification-area icon owned by [`TrayIcon::hwnd`].
pub struct TrayIcon {
    hwnd: HWND,
    added: bool,
}

impl TrayIcon {
    /// Adds the icon to the notification area. `callback_message` is posted to
    /// `hwnd` with mouse events in `lParam`.
    pub fn new(hwnd: HWND, callback_message: u32) -> Result<Self, TrayError> {
        let mut icon = Self { hwnd, added: false };
        let stock_icon =
            unsafe { LoadIconW(None, IDI_APPLICATION) }.map_err(|error| TrayError {
                context: "loading the stock tray icon",
                source: error,
            })?;
        let mut data = icon.base_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = callback_message;
        data.hIcon = stock_icon;
        copy_wide(&mut data.szTip, TOOLTIP);
        if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            return Err(TrayError::from_thread("adding the tray icon (NIM_ADD)"));
        }
        icon.added = true;
        Ok(icon)
    }

    /// Shows the tray context menu for `hwnd` and lets Windows post the
    /// selected command back to it as `WM_COMMAND`.
    ///
    /// An associated function without `self`: `TrackPopupMenu` runs a nested
    /// modal message loop, so the caller must not hold a borrow of the shell
    /// state (which owns this icon) while the menu is open.
    pub fn show_context_menu(hwnd: HWND) -> Result<(), TrayError> {
        let menu = unsafe { CreatePopupMenu() }.map_err(|error| TrayError {
            context: "creating the tray menu (CreatePopupMenu)",
            source: error,
        })?;
        let result = Self::run_context_menu(hwnd, menu);
        let _ = unsafe { DestroyMenu(menu) };
        result
    }

    fn run_context_menu(hwnd: HWND, menu: HMENU) -> Result<(), TrayError> {
        unsafe { AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("Exit")) }.map_err(|error| {
            TrayError {
                context: "adding Exit to the tray menu (AppendMenuW)",
                source: error,
            }
        })?;
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) }.map_err(|error| TrayError {
            context: "reading the cursor position (GetCursorPos)",
            source: error,
        })?;
        // Required so the popup closes when the user clicks elsewhere; a
        // background process is allowed to fail this call, so it is ignored.
        let _ = unsafe { SetForegroundWindow(hwnd) };
        let shown =
            unsafe { TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, Some(0), hwnd, None) };
        if !shown.as_bool() {
            return Err(TrayError::from_thread(
                "showing the tray menu (TrackPopupMenu)",
            ));
        }
        let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
        Ok(())
    }

    /// Shows a warning balloon. Windows 10+ may render this as a toast and the
    /// user's focus-assist setting can suppress it; the full details always go
    /// to the log file.
    pub fn show_warning(&self, title: &str, message: &str) -> Result<(), TrayError> {
        let mut data = self.base_data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_WARNING;
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, message);
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            Ok(())
        } else {
            Err(TrayError::from_thread(
                "showing the hotkey warning balloon (NIM_MODIFY)",
            ))
        }
    }

    /// Removes the icon. Idempotent; returns whether it was actually gone.
    pub fn remove(&mut self) -> bool {
        if !self.added {
            return true;
        }
        self.added = false;
        let data = self.base_data();
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data) }.as_bool()
    }

    fn base_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ICON_ID,
            ..Default::default()
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
    }
}

fn copy_wide(buffer: &mut [u16], text: &str) {
    let encoded: Vec<u16> = text
        .encode_utf16()
        .take(buffer.len().saturating_sub(1))
        .collect();
    buffer[..encoded.len()].copy_from_slice(&encoded);
    if let Some(last) = buffer.get_mut(encoded.len()) {
        *last = 0;
    }
}
