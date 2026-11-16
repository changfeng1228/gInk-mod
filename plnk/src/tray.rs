//! Tray icon and popup menu.
//!
//! The icon lives in the notification area and is owned by the hidden
//! application window created in `app.rs`. Removing the icon is part of the
//! exit path: [`TrayIcon`]'s `Drop` and the explicit [`TrayIcon::remove`] both
//! call `Shell_NotifyIconW(NIM_DELETE)`, so no stale icon survives an exit.
//!
//! The original red/white gInk ICO is loaded relative to the executable and is
//! also assigned to the hidden shell window. Missing files degrade to the stock
//! application icon with a deferred warning for the application log.

use std::fmt;
use std::path::Path;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, HICON, HMENU, ICON_BIG,
    ICON_SMALL, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, LoadIconW,
    LoadImageW, MF_SEPARATOR, MF_STRING, PostMessageW, SendMessageW, SetForegroundWindow,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL, WM_SETICON,
};
use windows::core::{Error as WindowsError, PCWSTR, w};

/// Command id posted by the tray menu. The same id the user selects in the
/// popup produces the `WM_COMMAND` handled by the shell window.
pub const MENU_OPTIONS: usize = 2;
pub const MENU_EXIT: usize = 1;

const TRAY_ICON_ID: u32 = 1;
pub const TRAY_TOOLTIP: &str = concat!("plnk ", env!("CARGO_PKG_VERSION"));

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

/// Owned or shared icon handle. File icons are destroyed; stock icons are not.
struct LoadedIcon {
    handle: HICON,
    owned: bool,
    warning: Option<String>,
}

impl Drop for LoadedIcon {
    fn drop(&mut self) {
        if self.owned && !self.handle.is_invalid() {
            let _ = unsafe { DestroyIcon(self.handle) };
        }
    }
}

/// The notification-area icon owned by [`TrayIcon::hwnd`].
pub struct TrayIcon {
    hwnd: HWND,
    added: bool,
    // Kept alive until after the shell HWND is destroyed (Shell field order).
    _icon: LoadedIcon,
    asset_warning: Option<String>,
}

impl TrayIcon {
    /// Adds the icon to the notification area. `callback_message` is posted to
    /// `hwnd` with mouse events in `lParam`.
    pub fn new(hwnd: HWND, callback_message: u32, white: bool) -> Result<Self, TrayError> {
        let icon = match crate::assets::icon_directory() {
            Ok(directory) => load_tray_icon(&directory, white)?,
            Err(error) => stock_tray_icon(Some(format!(
                "could not locate the executable asset directory: {error}; using stock tray icon"
            )))?,
        };
        let asset_warning = icon.warning.clone();
        let mut tray = Self {
            hwnd,
            added: false,
            _icon: icon,
            asset_warning,
        };
        let mut data = tray.base_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = callback_message;
        data.hIcon = tray._icon.handle;
        copy_wide(&mut data.szTip, TRAY_TOOLTIP);
        set_window_icons(hwnd, tray._icon.handle);
        if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            return Err(TrayError::from_thread("adding the tray icon (NIM_ADD)"));
        }
        tray.added = true;
        Ok(tray)
    }

    pub fn take_asset_warning(&mut self) -> Option<String> {
        self.asset_warning.take()
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
        unsafe { AppendMenuW(menu, MF_STRING, MENU_OPTIONS, w!("Options...")) }.map_err(
            |error| TrayError {
                context: "adding Options to the tray menu (AppendMenuW)",
                source: error,
            },
        )?;
        unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) }.map_err(|error| {
            TrayError {
                context: "adding a separator to the tray menu (AppendMenuW)",
                source: error,
            }
        })?;
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

fn load_tray_icon(directory: &Path, white: bool) -> Result<LoadedIcon, TrayError> {
    use std::os::windows::ffi::OsStrExt;

    let filename = crate::assets::tray_icon_filename(white);
    let path = directory.join(filename);
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    match unsafe {
        LoadImageW(
            None,
            PCWSTR(wide.as_ptr()),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
    } {
        Ok(handle) => Ok(LoadedIcon {
            handle: HICON(handle.0),
            owned: true,
            warning: None,
        }),
        Err(error) => stock_tray_icon(Some(format!(
            "legacy tray icon {} could not be loaded: {error}; using stock tray icon",
            path.display()
        ))),
    }
}

fn stock_tray_icon(warning: Option<String>) -> Result<LoadedIcon, TrayError> {
    let handle = unsafe { LoadIconW(None, IDI_APPLICATION) }.map_err(|error| TrayError {
        context: "loading the stock tray icon fallback",
        source: error,
    })?;
    Ok(LoadedIcon {
        handle,
        owned: false,
        warning,
    })
}

fn set_window_icons(hwnd: HWND, icon: HICON) {
    for kind in [ICON_SMALL, ICON_BIG] {
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(kind as usize)),
                Some(LPARAM(icon.0 as isize)),
            )
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_red_and_white_icons_load_and_missing_files_use_stock_fallback() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        for white in [false, true] {
            let loaded = load_tray_icon(&root, white).expect("load original ICO");
            assert!(loaded.owned, "the packaged ICO must own its HICON");
            assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
            assert!(!loaded.handle.is_invalid());
        }

        let missing =
            std::env::temp_dir().join(format!("plnk-missing-tray-assets-{}", std::process::id()));
        let fallback = load_tray_icon(&missing, false).expect("stock icon fallback");
        assert!(
            !fallback.owned,
            "the shared stock icon must not be destroyed"
        );
        assert!(!fallback.handle.is_invalid());
        assert!(
            fallback
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("icon_red.ico")),
            "{:?}",
            fallback.warning
        );
    }
}
