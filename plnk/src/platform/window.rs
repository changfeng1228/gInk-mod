//! Top-level windows for the ink overlay and the toolbar.
//!
//! Both windows are plain `WS_POPUP` windows without a redirection bitmap; all
//! pixels come from a DirectComposition visual tree, which is what makes
//! per-pixel transparency (overlay) and DWM-driven transform animations
//! (toolbar) possible. Windows are owned by the thread that creates them (the
//! structs are `!Send`/`!Sync`), so message handling, rendering and destruction
//! all stay on the UI thread.

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HINSTANCE, HWND, LPARAM,
    LRESULT, POINT, RECT, SetLastError, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_EXSTYLE, GWLP_USERDATA,
    GetCursorPos, GetWindowLongPtrW, GetWindowLongW, GetWindowRect, LWA_ALPHA, RegisterClassExW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowLongW, SetWindowPos,
    ShowWindow, WM_APP, WM_ERASEBKGND, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{HRESULT, PCWSTR, w};

use crate::gfx::Rgba8;
use crate::gfx::composition::{Easing, WindowComposition};
use crate::gfx::device::{GfxError, GraphicsDevice, failed};
use crate::platform::dpi::{PhysicalPoint, PhysicalRect};

/// Message the acceptance test posts to prove its callback meter is live.
///
/// The animation path must never post or receive this (or any other per-frame
/// message); production code does not use it.
pub const APP_CALLBACK_MESSAGE: u32 = WM_APP + 1;

/// Per-window message counters exposed for the "no per-frame callback"
/// acceptance test. `timer` counts WM_TIMER, `paint` counts WM_PAINT and
/// `custom` counts [`APP_CALLBACK_MESSAGE`]. `mouse_move` counts routed
/// WM_MOUSEMOVE messages and is used by the input pass-through test, not by
/// the animation assertions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallbackCounts {
    pub timer: u32,
    pub paint: u32,
    pub custom: u32,
    pub mouse_move: u32,
}

#[derive(Default)]
struct WindowCounters {
    timer: AtomicU32,
    paint: AtomicU32,
    custom: AtomicU32,
    mouse_move: AtomicU32,
}

impl WindowCounters {
    fn snapshot(&self) -> CallbackCounts {
        CallbackCounts {
            timer: self.timer.load(Ordering::Relaxed),
            paint: self.paint.load(Ordering::Relaxed),
            custom: self.custom.load(Ordering::Relaxed),
            mouse_move: self.mouse_move.load(Ordering::Relaxed),
        }
    }
}

/// A monotonic clock backed by `QueryPerformanceCounter`.
///
/// `now()` returns the duration since the first call in this process, which is
/// all animation bookkeeping needs (plan §6.2: store absolute start time,
/// duration, start and end points; no per-frame sampling loop).
///
/// [`Self::from_performance_count`] converts a raw QPC reading from another API
/// — notably `POINTER_INFO::PerformanceCount` — into the same time base, so
/// pointer timestamps and presentation timestamps can be compared directly.
pub struct MonotonicClock;

impl MonotonicClock {
    pub fn now() -> Duration {
        let (epoch, frequency) = clock_base();
        let mut counter = 0i64;
        unsafe {
            let _ = QueryPerformanceCounter(&mut counter);
        }
        duration_since(epoch, frequency, counter)
    }

    /// Converts a raw QPC counter reading (for example the `PerformanceCount`
    /// field of `POINTER_INFO`) into the [`Self::now`] time base. Readings
    /// before the process clock epoch clamp to zero.
    pub fn from_performance_count(counter: i64) -> Duration {
        let (epoch, frequency) = clock_base();
        duration_since(epoch, frequency, counter)
    }
}

/// Process clock epoch (QPC tick of the first use) and frequency.
fn clock_base() -> (i64, i64) {
    static CLOCK: OnceLock<(i64, i64)> = OnceLock::new();
    *CLOCK.get_or_init(|| {
        let mut frequency = 0i64;
        let mut epoch = 0i64;
        // QueryPerformanceCounter/Frequency are documented to always succeed
        // on Windows 10+; on the impossible failure path keep a usable 1 kHz
        // time base instead of panicking.
        unsafe {
            let _ = QueryPerformanceFrequency(&mut frequency);
            let _ = QueryPerformanceCounter(&mut epoch);
        }
        (epoch, frequency.max(1))
    })
}

fn duration_since(epoch: i64, frequency: i64, counter: i64) -> Duration {
    let ticks = (counter - epoch).max(0) as u64;
    let frequency = frequency.max(1) as u64;
    let seconds = ticks / frequency;
    let nanos = (ticks % frequency).saturating_mul(1_000_000_000) / frequency;
    Duration::new(seconds, nanos as u32)
}

/// Transparent, topmost ink overlay window.
///
/// In drawing mode it accepts mouse input; in pointer mode
/// [`OverlayWindow::set_click_through`] adds `WS_EX_TRANSPARENT` so the window
/// manager routes input to the windows below without any per-pixel hit test.
pub struct OverlayWindow {
    // Declared before `window` so the composition target and swap chain are
    // released before the HWND is destroyed.
    composition: WindowComposition,
    window: NativeWindow,
}

impl OverlayWindow {
    pub fn new(device: &GraphicsDevice, bounds: PhysicalRect) -> Result<Self, GfxError> {
        Self::new_with_hook(device, bounds, None, std::ptr::null_mut())
    }

    /// Creates the overlay with a window-procedure hook, used by the drawing
    /// session to receive `WM_POINTER`/mouse input. The data pointer contract
    /// is documented on [`WindowHook`].
    pub(crate) fn new_with_hook(
        device: &GraphicsDevice,
        bounds: PhysicalRect,
        hook: Option<WindowHook>,
        hook_data: *mut core::ffi::c_void,
    ) -> Result<Self, GfxError> {
        let window =
            NativeWindow::create_with_hook(bounds, w!("plnk overlay"), true, hook, hook_data)?;
        let composition = WindowComposition::new(
            device,
            window.hwnd(),
            bounds.width() as u32,
            bounds.height() as u32,
        )?;
        Ok(Self {
            composition,
            window,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    pub fn window_rect(&self) -> Result<PhysicalRect, GfxError> {
        self.window.window_rect()
    }

    /// The swap-chain composition owned by this window (used by the ink
    /// surface to present the static/active caches).
    pub(crate) fn composition_mut(&mut self) -> &mut WindowComposition {
        &mut self.composition
    }

    /// Fills the whole overlay with `color` (alpha is honored by DWM).
    pub fn fill(&mut self, color: Rgba8) -> Result<(), GfxError> {
        self.composition.fill(color)
    }

    pub fn show(&self) {
        self.window.show();
    }

    pub fn hide(&self) {
        self.window.hide();
    }

    /// Toggles input pass-through. `true` = pointer mode (input reaches the
    /// window below), `false` = drawing mode (the overlay accepts input).
    pub fn set_click_through(&mut self, enabled: bool) -> Result<(), GfxError> {
        self.window.set_click_through(enabled)
    }

    pub fn callback_counts(&self) -> CallbackCounts {
        self.window.callback_counts()
    }
}

/// Small topmost toolbar window whose motion is animated by DirectComposition.
pub struct ToolbarWindow {
    composition: WindowComposition,
    window: NativeWindow,
    /// Last offset requested; DirectComposition animations are retargeted from
    /// this bookkeeping value (Task 5 layers the model animation on top).
    offset: PhysicalPoint,
}

impl ToolbarWindow {
    pub fn new(device: &GraphicsDevice, bounds: PhysicalRect) -> Result<Self, GfxError> {
        let window = NativeWindow::create_with_hook(
            bounds,
            w!("plnk toolbar"),
            false,
            None,
            std::ptr::null_mut(),
        )?;
        let composition = WindowComposition::new(
            device,
            window.hwnd(),
            bounds.width() as u32,
            bounds.height() as u32,
        )?;
        Ok(Self {
            composition,
            window,
            offset: PhysicalPoint::new(0, 0),
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    pub fn window_rect(&self) -> Result<PhysicalRect, GfxError> {
        self.window.window_rect()
    }

    /// Fills the visual with `color` (the gate prototype draws one block).
    pub fn fill(&mut self, color: Rgba8) -> Result<(), GfxError> {
        self.composition.fill(color)
    }

    pub fn show(&self) {
        self.window.show();
    }

    /// Places the visual at a fixed physical-pixel offset inside the window.
    pub fn set_offset(&mut self, offset: PhysicalPoint) -> Result<(), GfxError> {
        self.composition.set_offset(offset)?;
        self.offset = offset;
        Ok(())
    }

    /// Animates the visual from the last requested offset to `target`.
    ///
    /// Only start, end, duration and easing are handed to DirectComposition;
    /// DWM evaluates the curve on its composition cadence. This call returns
    /// immediately and the application receives no per-frame callbacks.
    ///
    /// This raw primitive intentionally keeps no animation clock: a second call
    /// starts from the previously requested target, not from where the visual
    /// currently is. The product toolbar (`ui::toolbar::ToolbarWindow`) layers
    /// [`crate::ui::animation::ToolbarAnimator`] on top to retarget from the
    /// live visual position.
    pub fn animate_to(
        &mut self,
        target: PhysicalPoint,
        duration: Duration,
        easing: Easing,
    ) -> Result<(), GfxError> {
        self.composition
            .animate_offset(self.offset, target, duration, easing)?;
        self.offset = target;
        Ok(())
    }

    pub fn callback_counts(&self) -> CallbackCounts {
        self.window.callback_counts()
    }
}

/// A window-procedure hook installed by a higher layer (the toolbar UI).
///
/// The hook is called for every message after the shared callback counters
/// were updated, except `WM_NCCREATE`/`WM_NCDESTROY` (the window back-pointer
/// is not installed yet / has already been cleared). Returning `Some` takes
/// over the message; `None` continues with the default handling.
///
/// # Safety
/// `data` is the pointer registered by the window creator. It must stay valid
/// for as long as the HWND exists; the window clears the hook before the HWND
/// is destroyed.
pub(crate) type WindowHook =
    unsafe fn(*mut core::ffi::c_void, HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>;

struct WindowContext {
    counters: WindowCounters,
    hook: Option<WindowHook>,
    hook_data: *mut core::ffi::c_void,
}

/// Common top-level window plumbing: class registration, creation, extended
/// style management, destruction and the acceptance-test counters.
pub(crate) struct NativeWindow {
    hwnd: HWND,
    // Boxed so the pointer stored in GWLP_USERDATA stays valid when the window
    // object itself is moved (constructors return it by value).
    context: Box<WindowContext>,
    _not_send: PhantomData<Rc<()>>,
}

impl NativeWindow {
    pub(crate) fn create_with_hook(
        bounds: PhysicalRect,
        title: PCWSTR,
        layered: bool,
        hook: Option<WindowHook>,
        hook_data: *mut core::ffi::c_void,
    ) -> Result<Self, GfxError> {
        ensure_process_dpi_awareness()?;
        if bounds.width() <= 0 || bounds.height() <= 0 {
            return Err(GfxError::ZeroSize {
                width: bounds.width().max(0) as u32,
                height: bounds.height().max(0) as u32,
            });
        }

        let instance =
            unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(failed("GetModuleHandleW"))?;
        let class_name = w!("plnk_gate_window");
        let window_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: class_name,
            ..Default::default()
        };
        let atom = unsafe { RegisterClassExW(&window_class) };
        if atom == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(GfxError::Operation {
                    context: "RegisterClassExW",
                    source: windows::core::Error::from_hresult(HRESULT::from_win32(error.0)),
                });
            }
        }

        let context = Box::new(WindowContext {
            counters: WindowCounters::default(),
            hook,
            hook_data,
        });
        // Stored in GWLP_USERDATA for the window procedure. The Box keeps the
        // context alive for at least as long as the HWND (destruction runs in
        // `destroy`/Drop, which also clears the hook, before the Box drops).
        let context_pointer = context.as_ref() as *const WindowContext as *const core::ffi::c_void;
        let mut style =
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
        if layered {
            // Windows only routes mouse input through `WS_EX_TRANSPARENT`
            // across process boundaries on layered windows. A layered window
            // without a redirection bitmap must have layered attributes set or
            // it is not shown; alpha 255 leaves the DirectComposition content's
            // own per-pixel alpha untouched.
            style |= WS_EX_LAYERED;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                style,
                class_name,
                title,
                WS_POPUP,
                bounds.left,
                bounds.top,
                bounds.width(),
                bounds.height(),
                None,
                None,
                Some(HINSTANCE(instance.0)),
                Some(context_pointer),
            )
        }
        .map_err(failed("CreateWindowExW"))?;

        if layered
            && let Err(error) =
                unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) }
        {
            // Do not leak the window if its layered attributes cannot be set.
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(failed("SetLayeredWindowAttributes(LWA_ALPHA)")(error));
        }

        Ok(Self {
            hwnd,
            context,
            _not_send: PhantomData,
        })
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub(crate) fn callback_counts(&self) -> CallbackCounts {
        self.context.counters.snapshot()
    }

    pub(crate) fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub(crate) fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub(crate) fn window_rect(&self) -> Result<PhysicalRect, GfxError> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut rect) }.map_err(failed("GetWindowRect"))?;
        Ok(PhysicalRect::new(
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
        ))
    }

    pub(crate) fn set_click_through(&mut self, enabled: bool) -> Result<(), GfxError> {
        let current = unsafe { GetWindowLongW(self.hwnd, GWL_EXSTYLE) } as u32;
        let mut updated = current & !WS_EX_TRANSPARENT.0;
        if enabled {
            updated |= WS_EX_TRANSPARENT.0;
        }
        if updated == current {
            return Ok(());
        }
        unsafe { SetLastError(ERROR_SUCCESS) };
        let previous = unsafe { SetWindowLongW(self.hwnd, GWL_EXSTYLE, updated as i32) };
        if previous == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_SUCCESS {
                return Err(GfxError::Operation {
                    context: "SetWindowLongW(GWL_EXSTYLE)",
                    source: windows::core::Error::from_hresult(HRESULT::from_win32(error.0)),
                });
            }
        }
        // Flush the cached window style so hit testing takes effect at once.
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(failed("SetWindowPos(SWP_FRAMECHANGED)"))?;
        Ok(())
    }

    fn destroy(&mut self) {
        self.context.hook = None;
        self.context.hook_data = std::ptr::null_mut();
        if !self.hwnd.is_invalid() {
            let _ = unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = HWND::default();
        }
    }
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Physical bounds of the monitor that currently contains the mouse cursor.
///
/// The first release targets the current monitor only (spec §2/§4); a
/// multi-monitor virtual-screen overlay remains 未验证.
pub fn current_monitor_rect() -> Result<PhysicalRect, GfxError> {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        Ok(PhysicalRect::new(
            info.rcMonitor.left,
            info.rcMonitor.top,
            info.rcMonitor.right,
            info.rcMonitor.bottom,
        ))
    } else {
        Err(failed("GetMonitorInfoW")(
            windows::core::Error::from_thread(),
        ))
    }
}

fn ensure_process_dpi_awareness() -> Result<(), GfxError> {
    static STATE: OnceLock<Result<(), HRESULT>> = OnceLock::new();
    let result = STATE.get_or_init(|| {
        match unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } {
            Ok(()) => Ok(()),
            Err(error) => {
                // The API fails when awareness is already set (e.g. by a
                // manifest); that is fine as long as it is per-monitor-v2.
                let current = unsafe { GetThreadDpiAwarenessContext() };
                let already_v2 = unsafe {
                    AreDpiAwarenessContextsEqual(
                        current,
                        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                    )
                }
                .as_bool();
                if already_v2 {
                    Ok(())
                } else {
                    Err(error.code())
                }
            }
        }
    });
    result.map_err(|code| GfxError::Operation {
        context: "SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)",
        source: windows::core::Error::from_hresult(code),
    })
}

fn context(hwnd: HWND) -> Option<&'static WindowContext> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowContext;
    if pointer.is_null() {
        None
    } else {
        // SAFETY: the pointer is installed in WM_NCCREATE from the Box kept
        // alive by NativeWindow and cleared in WM_NCDESTROY, so any message
        // dispatched to this HWND sees a live WindowContext.
        Some(unsafe { &*pointer })
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    if message == WM_NCDESTROY {
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }

    let context = context(hwnd);
    if let Some(context) = context {
        match message {
            WM_PAINT => {
                context.counters.paint.fetch_add(1, Ordering::Relaxed);
                let mut paint = PAINTSTRUCT::default();
                unsafe {
                    BeginPaint(hwnd, &mut paint);
                    let _ = EndPaint(hwnd, &paint);
                }
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_TIMER => {
                context.counters.timer.fetch_add(1, Ordering::Relaxed);
            }
            WM_MOUSEMOVE => {
                context.counters.mouse_move.fetch_add(1, Ordering::Relaxed);
            }
            message if message == APP_CALLBACK_MESSAGE => {
                context.counters.custom.fetch_add(1, Ordering::Relaxed);
                return LRESULT(0);
            }
            _ => {}
        }
        if let Some(hook) = context.hook {
            // SAFETY: the creator guarantees `hook_data` is valid while the
            // HWND exists; `destroy` clears the hook before DestroyWindow.
            if let Some(result) = unsafe { hook(context.hook_data, hwnd, message, wparam, lparam) }
            {
                return result;
            }
        }
    }

    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut paint);
                let _ = EndPaint(hwnd, &paint);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        message if message == APP_CALLBACK_MESSAGE => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
