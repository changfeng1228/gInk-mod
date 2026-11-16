//! Application lifecycle for plnk.
//!
//! One UI thread owns the hidden shell window, the tray icon, the global hotkey
//! registrations and the single [`AppMode`] state. File work is handed to a
//! small bounded worker; results come back to the UI thread through
//! `PostMessageW`, and closing the application drops pending results instead of
//! waiting for the worker (plan §6.2).
//!
//! Isolation from the legacy gInk instance is deliberate: a different mutex
//! name, a different data directory, no startup-item changes, and the
//! `--no-hotkeys` development switch so a test run never competes with the
//! legacy app's system-wide combinations.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_CLASS_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM,
    LRESULT, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, RegisterHotKey, ReleaseCapture, SetCapture, UnregisterHotKey, VK_ESCAPE,
};
use windows::Win32::UI::Input::Pointer::{GetPointerInfo, POINTER_INFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetMessageW, GetWindowLongPtrW, HCURSOR, HTCLIENT, MB_ICONERROR, MB_OK, MSG, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassExW, SetCursor, SetTimer, SetWindowLongPtrW,
    TranslateMessage, WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY,
    WM_HOTKEY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_POINTERCAPTURECHANGED,
    WM_POINTERDOWN, WM_POINTERLEAVE, WM_POINTERUP, WM_POINTERUPDATE, WM_RBUTTONUP, WM_SETCURSOR,
    WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{Error as WindowsError, HRESULT, HSTRING, PCWSTR, w};

use crate::error::SettingsError;
use crate::gfx::Rgba8;
use crate::gfx::device::{GfxError, GraphicsDevice};
use crate::gfx::ink_surface::InkSurface;
use crate::hotkeys::{Command, GlobalHotkeys, HotkeyIssue, HotkeyMap};
use crate::ink::controller::InkController;
use crate::ink::dirty::DirtyRegion;
use crate::ink::geometry::PhysicalPointF;
use crate::ink::model::{Edit, PenStyle};
use crate::input::{MouseNormalizer, PointerKind, PointerNormalizer};
use crate::platform::cursor::{CanvasCursors, default_class_cursor};
use crate::platform::dpi::PhysicalRect;
use crate::platform::window::{MonotonicClock, current_monitor_rect, ensure_process_dpi_awareness};
use crate::settings::{PenSettings, Settings};
use crate::settings_store::{SettingsStore, StorageRoots};
use crate::telemetry::{LogError, LogSink};
use crate::tray::{MENU_EXIT, MENU_OPTIONS, TrayError, TrayIcon};
use crate::ui::pen_width::{DEFAULT_WIDTH_HIMETRIC, PenWidthPanel};
use crate::ui::toolbar::{ButtonId, ToolbarWindow as ToolbarUiWindow, pen_width_visible};

/// Singleton mutex of the new app. Deliberately different from the legacy
/// gInk GUID, so the legacy instance and a development build can coexist.
pub const INSTANCE_MUTEX_NAME: &str = "Global\\plnk-6f2c7a14-90b8-4e3d-8f6a-2c1d5b7e9a03";

/// Hidden window class used by the shell and by the desktop smoke test.
pub const SHELL_WINDOW_CLASS: &str = "plnk_app_shell_window";
pub const SHELL_WINDOW_TITLE: &str = "plnk";

/// Tray mouse callbacks posted by `Shell_NotifyIconW`.
pub const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 20;
/// Wake-up message posted by the bounded IO worker when a result is ready.
pub const IO_RESULT_MESSAGE: u32 = WM_APP + 21;
/// Posted by the toolbar when the user clicks a button; `wparam` carries
/// [`ButtonId::code`].
pub const TOOLBAR_CLICK_MESSAGE: u32 = WM_APP + 22;
/// Live pen-width updates posted by the popup slider (`wparam` = HIMETRIC).
pub const PEN_WIDTH_CHANGE_MESSAGE: u32 = WM_APP + 23;

/// Drawing-scoped Escape registration. Kept separate from the configurable
/// global hotkeys so `--no-hotkeys` development runs still support Esc.
const ANNOTATION_ESCAPE_HOTKEY_ID: i32 = 0x4553; // "ES"

const EXIT_TIMER_ID: usize = 0x504C; // "PL"
const IO_QUEUE_CAPACITY: usize = 8;

/// The single application state. Later tasks add real drawing/pointer/snapping
/// windows and input; Task 4 keeps the transitions explicit and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Tray,
    Drawing,
    Pointer,
    Snapping,
}

impl AppMode {
    /// Mode after a command. The mapping mirrors the legacy global-hotkey
    /// cycle: no ink -> start drawing; drawing -> pointer; pointer -> drawing.
    /// Snapshot commands arm the snapping state; the actual capture UI arrives
    /// in a later task.
    pub fn after_command(self, command: Command) -> Self {
        match command {
            Command::ToggleInk => match self {
                Self::Tray => Self::Drawing,
                Self::Drawing => Self::Pointer,
                Self::Pointer | Self::Snapping => Self::Drawing,
            },
            Command::Pointer => match self {
                Self::Drawing => Self::Pointer,
                Self::Pointer => Self::Drawing,
                Self::Tray | Self::Snapping => self,
            },
            Command::Snapshot | Command::SnapPin => Self::Snapping,
            _ => self,
        }
    }
}

/// Development options. `--no-hotkeys` keeps a test run from claiming the
/// legacy app's combinations; `--draw` enters drawing mode without the tray or
/// hotkey; `--pen-width` is retained for compatibility and is now a no-op,
/// because the MVP always shows the pen-width button; `--exit-after-ms` lets
/// the desktop smoke test start and stop the real shell without synthetic
/// input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOptions {
    pub no_hotkeys: bool,
    pub draw_mode: bool,
    pub pen_width: bool,
    pub exit_after: Option<Duration>,
}

impl RunOptions {
    pub fn from_args<I, S>(args: I) -> Result<Self, AppError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            let argument = argument.as_ref().to_string_lossy().into_owned();
            match argument.as_str() {
                "--no-hotkeys" => options.no_hotkeys = true,
                "--draw" => options.draw_mode = true,
                "--pen-width" => options.pen_width = true,
                "--exit-after-ms" => {
                    let value = args.next().ok_or_else(|| {
                        AppError::Usage(
                            "--exit-after-ms requires a value in milliseconds".to_owned(),
                        )
                    })?;
                    let text = value.as_ref().to_string_lossy().into_owned();
                    let milliseconds: u64 = text.parse().map_err(|_| {
                        AppError::Usage(format!("invalid --exit-after-ms value: {text}"))
                    })?;
                    options.exit_after = Some(Duration::from_millis(milliseconds));
                }
                _ => {
                    return Err(AppError::Usage(format!(
                        "unknown argument: {argument} (supported: --no-hotkeys, --draw, --pen-width, --exit-after-ms <ms>)"
                    )));
                }
            }
        }
        Ok(options)
    }
}

/// Whole-application failures. Every variant carries the context the user
/// needs to act (a path, a hotkey name, or the failing operation).
#[derive(Debug)]
pub enum AppError {
    Usage(String),
    Settings(SettingsError),
    Logging(LogError),
    Instance(io::Error),
    Tray(TrayError),
    Window {
        context: &'static str,
        source: WindowsError,
    },
    Graphics {
        context: &'static str,
        source: GfxError,
    },
    /// An internal invariant that should be impossible to violate.
    Runtime(String),
    IoWorker(io::Error),
    MessageLoop(WindowsError),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => write!(formatter, "invalid command line: {message}"),
            Self::Settings(error) => write!(formatter, "{error}"),
            Self::Logging(error) => write!(formatter, "{error}"),
            Self::Instance(error) => {
                write!(formatter, "failed to enforce single instance: {error}")
            }
            Self::Tray(error) => write!(formatter, "{error}"),
            Self::Window { context, source } => {
                write!(
                    formatter,
                    "application window failed while {context}: {source}"
                )
            }
            Self::Graphics { context, source } => {
                write!(formatter, "drawing stack failed while {context}: {source}")
            }
            Self::Runtime(message) => write!(formatter, "internal error: {message}"),
            Self::IoWorker(error) => {
                write!(
                    formatter,
                    "failed to start the bounded file IO worker: {error}"
                )
            }
            Self::MessageLoop(error) => {
                write!(formatter, "the application message loop failed: {error}")
            }
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Usage(_) => None,
            Self::Settings(error) => Some(error),
            Self::Logging(error) => Some(error),
            Self::Instance(error) => Some(error),
            Self::Tray(error) => Some(error),
            Self::Window { source, .. } | Self::MessageLoop(source) => Some(source),
            Self::Graphics { source, .. } => Some(source),
            Self::Runtime(_) => None,
            Self::IoWorker(error) => Some(error),
        }
    }
}

impl From<SettingsError> for AppError {
    fn from(error: SettingsError) -> Self {
        Self::Settings(error)
    }
}

impl From<LogError> for AppError {
    fn from(error: LogError) -> Self {
        Self::Logging(error)
    }
}

impl From<TrayError> for AppError {
    fn from(error: TrayError) -> Self {
        Self::Tray(error)
    }
}

impl From<GfxError> for AppError {
    fn from(source: GfxError) -> Self {
        Self::Graphics {
            context: "drawing the annotation overlay",
            source,
        }
    }
}

impl AppError {
    /// Top-level reporting: always to stderr (development console), and as a
    /// message box for fatal runtime failures so a double-clicked build does not
    /// fail silently. Command-line usage errors stay console-only.
    pub fn report(&self) {
        eprintln!("plnk: {self}");
        if matches!(self, Self::Usage(_)) {
            return;
        }
        let text = HSTRING::from(self.to_string());
        unsafe {
            MessageBoxW(None, &text, w!("plnk"), MB_OK | MB_ICONERROR);
        }
    }
}

/// The application entry point required by the task interface.
pub struct App;

impl App {
    /// Runs the application with default options (hotkeys enabled, no exit
    /// timer). Returns only when the shell window has been destroyed and all
    /// resources (tray icon, hotkeys, IO worker) are released.
    pub fn run(settings: Settings) -> Result<(), AppError> {
        Self::run_with(settings, RunOptions::default())
    }

    /// Runs with development options. Acquires the singleton guard itself,
    /// which is fine for embedders; `main` uses [`Self::run_with_instance`] so
    /// the guard is acquired before the settings import does any filesystem
    /// work.
    pub fn run_with(settings: Settings, options: RunOptions) -> Result<(), AppError> {
        let instance = InstanceGuard::acquire(INSTANCE_MUTEX_NAME).map_err(AppError::Instance)?;
        let Some(instance) = instance else {
            // A second copy never touches the first instance's tray icon,
            // hotkeys or log; it only reports why it is exiting.
            report_already_running();
            return Ok(());
        };
        Self::run_with_instance(settings, options, instance)
    }

    /// Runs with an already acquired singleton guard, which must stay alive for
    /// the whole process. This lets `main` enforce single-instance before it
    /// reads or imports any settings file.
    pub fn run_with_instance(
        settings: Settings,
        options: RunOptions,
        instance: InstanceGuard,
    ) -> Result<(), AppError> {
        let _instance = instance;

        // Physical-pixel coordinates everywhere (windows, overlay bounds, ink,
        // screenshot) require per-monitor-v2 awareness before the first
        // monitor/rect query; an unaware process gets virtualized rectangles.
        ensure_process_dpi_awareness().map_err(|source| AppError::Graphics {
            context: "enabling per-monitor-v2 DPI awareness",
            source,
        })?;

        let storage_root = StorageRoots::from_environment()
            .resolve()
            .map_err(AppError::Settings)?;
        let log = LogSink::open(storage_root.join("plnk.log"))?;
        log.info(&format!(
            "plnk {} starting (pid {})",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        ));
        if options.no_hotkeys {
            log.info("global hotkeys disabled by --no-hotkeys");
        }

        let result = Self::run_shell(settings, options, log.clone());
        match &result {
            Ok(()) => log.info("plnk exited normally"),
            Err(error) => log.error(&format!("fatal: {error}")),
        }
        result
    }

    fn run_shell(settings: Settings, options: RunOptions, log: LogSink) -> Result<(), AppError> {
        // The Shell must be boxed before the window exists: the window
        // procedure stores a raw pointer to it, and the address must never move
        // while Windows owns that pointer.
        let mut settings = settings;
        if options.pen_width {
            // Retained for compatibility with existing test invocations; the
            // MVP policy in `ui::toolbar` already exposes the button.
            settings.pen_width_icon = true;
        }
        let mut shell = Box::new(Shell::new(settings, options, log));
        let window = ShellWindow::create(shell.as_ref())?;
        let hwnd = window.hwnd();
        shell.window = Some(window);
        shell.start_services(hwnd)?;
        if shell.options.draw_mode {
            // Development switch: enter drawing mode without the tray/hotkey.
            shell.handle_command(Command::ToggleInk);
        }
        // Free function: no `&mut Shell` may be live while the pump dispatches
        // messages, because a nested modal loop (tray menu) can re-enter the
        // window procedure and borrow the shell again.
        pump_messages()
    }
}

/// Process-wide single-instance guard backed by a named mutex. `None` from
/// [`Self::acquire`] means another instance already owns the name; the caller
/// must not replace or disturb it.
pub struct InstanceGuard {
    handle: HANDLE,
    _not_send: PhantomData<Rc<()>>,
}

impl InstanceGuard {
    pub fn acquire(name: &str) -> io::Result<Option<Self>> {
        let name = HSTRING::from(name);
        let handle = unsafe { CreateMutexW(None, false, &name) }
            .map_err(|error| io::Error::other(format!("CreateMutexW: {error}")))?;
        let wait = unsafe { WaitForSingleObject(handle, 0) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            // WAIT_ABANDONED means the previous owner crashed; the mutex is
            // ours now and `Drop` releases it cleanly.
            Ok(Some(Self {
                handle,
                _not_send: PhantomData,
            }))
        } else if wait == WAIT_TIMEOUT {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Ok(None)
        } else {
            let error = unsafe { GetLastError() };
            unsafe {
                let _ = CloseHandle(handle);
            }
            Err(io::Error::other(format!(
                "WaitForSingleObject failed with Windows error {}",
                error.0
            )))
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Outcome of the single-instance check.
///
/// Deliberately UI-free so tests can assert the mapping without opening a
/// modal message box; `main` renders [`Self::AlreadyRunning`] visibly.
pub enum InstanceDecision {
    /// This process owns the singleton; the guard must stay alive.
    Primary(InstanceGuard),
    /// Another instance is already running; this process must exit
    /// successfully without touching settings or the first instance's log.
    AlreadyRunning,
}

/// Maps a singleton acquisition result to the process decision.
pub fn decide_instance(acquired: Option<InstanceGuard>) -> InstanceDecision {
    match acquired {
        Some(guard) => InstanceDecision::Primary(guard),
        None => InstanceDecision::AlreadyRunning,
    }
}

/// User-visible feedback for the second-instance exit path.
///
/// The second instance performs no settings work, so this is its only visible
/// trace: one line on stderr, one best-effort crash-log entry, and — outside
/// test builds — an information box explaining that plnk is already running.
/// The box is compiled out of test binaries so no automated test can block on
/// a modal window.
pub fn report_already_running() {
    eprintln!("plnk: another instance is already running");
    crate::telemetry::log_second_instance_exit();
    #[cfg(not(test))]
    {
        let text = HSTRING::from("plnk 已在运行，请查看系统托盘图标。");
        unsafe {
            MessageBoxW(
                None,
                &text,
                w!("plnk"),
                MB_OK | windows::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
            );
        }
    }
}

/// Result of one bounded IO job. The string payload is for the UI thread's use
/// (for example a saved file path); the shell logs only the label and error, so
/// screenshot pixels or file contents never enter the log.
#[derive(Debug)]
pub struct IoResult {
    pub label: &'static str,
    pub outcome: Result<String, String>,
}

/// Delivery target for IO results. The production implementation posts to the
/// UI thread; tests can record deliveries in-process.
pub trait IoCompletion: Send + Sync + 'static {
    fn complete(&self, result: IoResult);
}

/// Production delivery: store the result in a UI-thread-visible table and post
/// a wake-up message carrying only its numeric id.
pub struct PostMessageCompletion {
    /// The HWND is stored as an integer because the completion is used from the
    /// worker thread and `HWND` itself is not `Send`. The value is only passed
    /// to `PostMessageW`, which is thread-safe.
    hwnd: isize,
    message: u32,
    pending: Mutex<BTreeMap<u64, IoResult>>,
}

impl PostMessageCompletion {
    pub fn new(hwnd: HWND, message: u32) -> Self {
        Self {
            hwnd: hwnd.0 as isize,
            message,
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    /// Called by the UI thread when it receives the wake-up message.
    pub fn take(&self, id: u64) -> Option<IoResult> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id)
    }

    fn discard(&self, id: u64) {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
    }
}

impl IoCompletion for PostMessageCompletion {
    fn complete(&self, result: IoResult) {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, result);
        let posted = unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut core::ffi::c_void)),
                self.message,
                WPARAM(id as usize),
                LPARAM(0),
            )
        };
        if posted.is_err() {
            // The window is gone (shutdown race): drop the result instead of
            // leaving it in the table.
            self.discard(id);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoSubmitError {
    /// The bounded queue is full; the UI thread should retry later.
    Busy,
    /// The worker has been closed and will not run new work.
    Closed,
}

impl std::fmt::Display for IoSubmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(formatter, "the file IO queue is full"),
            Self::Closed => write!(formatter, "the file IO worker is closed"),
        }
    }
}

impl std::error::Error for IoSubmitError {}

struct IoTask {
    label: &'static str,
    work: Box<dyn FnOnce() -> Result<String, String> + Send + 'static>,
}

/// Bounded std-only file-IO worker. `submit` never blocks the UI thread; a full
/// queue is reported as [`IoSubmitError::Busy`]. `close` detaches immediately:
/// the in-flight job runs to completion on the worker thread, its result is
/// dropped, and the thread exits.
pub struct IoWorker {
    sender: Option<SyncSender<IoTask>>,
    closed: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl IoWorker {
    pub fn start(capacity: usize, completion: Arc<dyn IoCompletion>) -> io::Result<Self> {
        let capacity = capacity.max(1);
        let (sender, receiver) = sync_channel::<IoTask>(capacity);
        let closed = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let thread_closed = Arc::clone(&closed);
        let thread_finished = Arc::clone(&finished);
        let thread = thread::Builder::new()
            .name("plnk-io".to_owned())
            .spawn(move || {
                while !thread_closed.load(Ordering::Acquire) {
                    let Ok(task) = receiver.recv() else {
                        break;
                    };
                    if thread_closed.load(Ordering::Acquire) {
                        break;
                    }
                    let outcome = (task.work)();
                    if thread_closed.load(Ordering::Acquire) {
                        break;
                    }
                    completion.complete(IoResult {
                        label: task.label,
                        outcome,
                    });
                }
                thread_finished.store(true, Ordering::Release);
            })?;
        Ok(Self {
            sender: Some(sender),
            closed,
            finished,
            thread: Some(thread),
        })
    }

    pub fn submit<F>(&self, label: &'static str, work: F) -> Result<(), IoSubmitError>
    where
        F: FnOnce() -> Result<String, String> + Send + 'static,
    {
        if self.closed.load(Ordering::Acquire) {
            return Err(IoSubmitError::Closed);
        }
        let Some(sender) = &self.sender else {
            return Err(IoSubmitError::Closed);
        };
        sender
            .try_send(IoTask {
                label,
                work: Box::new(work),
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => IoSubmitError::Busy,
                TrySendError::Disconnected(_) => IoSubmitError::Closed,
            })
    }

    /// Stops accepting work and detaches the worker thread without waiting for
    /// an in-flight job. This is the "closing during pending IO" path: it must
    /// return immediately so the UI thread can exit.
    pub fn close(&mut self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.sender = None;
        // Deliberately no join: joining would block the UI thread on the
        // file operation that is being abandoned.
        self.thread = None;
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

impl Drop for IoWorker {
    fn drop(&mut self) {
        self.close();
    }
}

/// Operations a drawing-session message produces that must run after the
/// session borrow ends (`SetCapture`/`ReleaseCapture` synchronously send
/// `WM_CAPTURECHANGED` back into the window procedure).
enum SessionOp {
    SetCapture,
    ReleaseCapture,
    SetCursor(HCURSOR),
}

/// The live drawing session: ink surface, controller and input normalizers.
///
/// The overlay window created by [`InkSurface`] routes its messages to
/// [`session_hook`] with a pointer to this struct, so a `Box<InkSession>` must
/// never move its contents (the `Box` itself may move freely). The surface is
/// the first field so the window — and its hook back-pointer — is destroyed
/// before the rest of the session state.
pub struct InkSession {
    surface: Option<InkSurface>,
    controller: InkController,
    pen_style: PenStyle,
    pointer: PointerNormalizer,
    mouse: MouseNormalizer,
    cursors: CanvasCursors,
    active_pointer: Option<u32>,
    mouse_down: bool,
    committed: Option<usize>,
    last_error: Option<String>,
}

impl InkSession {
    /// Creates the overlay and starts routing its input to the session.
    pub fn new(device: &GraphicsDevice, bounds: PhysicalRect) -> Result<Box<Self>, GfxError> {
        let cursors = match crate::assets::icon_directory() {
            Ok(directory) => CanvasCursors::load(&directory),
            Err(error) => {
                // No asset directory means no original cursors; keep the app
                // usable with the standard arrow and record the reason.
                let mut cursors = CanvasCursors::load(std::path::Path::new(""));
                cursors.record_warning(format!("asset directory is unavailable: {error}"));
                cursors
            }
        };
        let mut session = Box::new(Self {
            surface: None,
            controller: InkController::new(),
            pen_style: PenStyle::fixed(Rgba8::new(0, 0, 0, 255), 80.0),
            pointer: PointerNormalizer::new(),
            mouse: MouseNormalizer::new(),
            cursors,
            active_pointer: None,
            mouse_down: false,
            committed: None,
            last_error: None,
        });
        let data = &mut *session as *mut Self as *mut core::ffi::c_void;
        let surface = InkSurface::new_with_hook(device, bounds, session_hook, data)?;
        session.surface = Some(surface);
        Ok(session)
    }

    /// Number of original pen-cursor files that loaded (0-4).
    pub fn pen_cursor_count(&self) -> usize {
        self.cursors.loaded_count()
    }

    /// Drains cursor-loading warnings for the shell log.
    pub fn take_cursor_warnings(&mut self) -> Vec<String> {
        self.cursors.take_warnings()
    }

    pub fn surface(&self) -> Option<&InkSurface> {
        self.surface.as_ref()
    }

    pub fn surface_mut(&mut self) -> Option<&mut InkSurface> {
        self.surface.as_mut()
    }

    pub fn controller(&self) -> &InkController {
        &self.controller
    }

    /// Style for the next stroke (color, width, highlighter alpha). The
    /// pressure channel is enabled per contact from the pointer capability.
    pub fn set_pen_style(&mut self, style: PenStyle) {
        self.pen_style = PenStyle::new(style.color, style.width, style.pressure_enabled);
    }

    /// Number of document strokes after the latest commit, drained by the
    /// shell for bounded acceptance diagnostics.
    pub fn take_committed(&mut self) -> Option<usize> {
        self.committed.take()
    }

    /// First error since the last call; the shell logs it on its next command
    /// or toolbar event (a drawing failure must not panic or kill the app).
    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// Cancels any live contact. Mode changes call this before flipping input
    /// pass-through so a partial stroke never commits.
    pub fn cancel_active(&mut self) -> Result<(), GfxError> {
        self.drop_active()
    }

    /// Undoes the last edit and repaints only its pixels.
    pub fn undo(&mut self) -> Result<(), GfxError> {
        self.drop_active()?;
        let Some(region) = self.controller.document_mut().undo() else {
            return Ok(());
        };
        self.redraw_and_present(&region)
    }

    /// Clears the canvas as an undoable edit.
    pub fn clear(&mut self) -> Result<(), GfxError> {
        self.drop_active()?;
        let region = self.controller.document_mut().apply(Edit::Clear);
        if region.is_empty() {
            return Ok(());
        }
        self.redraw_and_present(&region)
    }

    /// Leaving annotation: drop a live contact and clear the canvas through the
    /// command history (legacy `RetreatAndExit`; undo after reopening restores
    /// the pre-exit strokes, same process only).
    pub fn close_annotation(&mut self) -> Result<(), GfxError> {
        self.drop_active()?;
        let region = self.controller.document_mut().close_annotation();
        if region.is_empty() {
            return Ok(());
        }
        self.redraw_and_present(&region)
    }

    /// Re-entering annotation without touching the in-memory history.
    pub fn reopen_annotation(&mut self) {
        self.controller.document_mut().reopen_annotation();
    }

    fn on_message(
        &mut self,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> (bool, Vec<SessionOp>) {
        match message {
            WM_SETCURSOR => {
                // Only the client area gets the pen cursor; anything else is
                // the shared window procedure's arrow. Returning TRUE keeps
                // DefWindowProc from restoring the class cursor.
                if (lparam.0 & 0xFFFF) as u32 != HTCLIENT {
                    return (false, Vec::new());
                }
                let color = self.pen_style.color;
                let cursor = self.cursors.for_pen(color.r, color.g, color.b);
                (true, vec![SessionOp::SetCursor(cursor)])
            }
            WM_LBUTTONDOWN => {
                let began = self.mouse_down(lparam);
                let ops = if began {
                    vec![SessionOp::SetCapture]
                } else {
                    Vec::new()
                };
                (began, ops)
            }
            WM_MOUSEMOVE => {
                self.mouse_move(lparam);
                (self.mouse_down, Vec::new())
            }
            WM_LBUTTONUP => {
                let releasing = self.mouse_up(lparam);
                let ops = if releasing {
                    vec![SessionOp::ReleaseCapture]
                } else {
                    Vec::new()
                };
                (releasing, ops)
            }
            WM_CAPTURECHANGED => {
                if self.mouse_down {
                    self.cancel_contact();
                }
                (false, Vec::new())
            }
            // Pointer messages are always consumed so `DefWindowProcW` does not
            // promote an unhandled touch/pen contact into a duplicate mouse
            // stroke; the "no pointer messages" case is a physical mouse, whose
            // WM_MOUSE messages arrive on the fallback path above.
            WM_POINTERDOWN => {
                self.pointer_down(wparam);
                (true, Vec::new())
            }
            WM_POINTERUPDATE => {
                self.pointer_update(wparam);
                (true, Vec::new())
            }
            WM_POINTERUP => {
                self.pointer_up(wparam);
                (true, Vec::new())
            }
            WM_POINTERLEAVE => {
                if self.active_pointer.is_some() {
                    self.cancel_contact();
                }
                (true, Vec::new())
            }
            WM_POINTERCAPTURECHANGED => {
                if self.active_pointer.is_some() {
                    self.cancel_contact();
                }
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        }
    }

    fn mouse_down(&mut self, lparam: LPARAM) -> bool {
        if self.controller.is_drawing() || self.active_pointer.is_some() {
            return false;
        }
        let Some(position) = self.client_position(lparam) else {
            return false;
        };
        self.apply_style(false);
        let point = self.mouse.sample(position, MonotonicClock::now());
        let dirty = self.controller.begin(point);
        self.mouse_down = true;
        self.after_append(dirty);
        true
    }

    fn mouse_move(&mut self, lparam: LPARAM) {
        if !self.mouse_down || self.active_pointer.is_some() {
            return;
        }
        let Some(position) = self.client_position(lparam) else {
            return;
        };
        let point = self.mouse.sample(position, MonotonicClock::now());
        let dirty = self.controller.append(&[point]);
        self.after_append(dirty);
    }

    fn mouse_up(&mut self, lparam: LPARAM) -> bool {
        if !self.mouse_down {
            return false;
        }
        self.mouse_down = false;
        if let Some(position) = self.client_position(lparam) {
            let point = self.mouse.sample(position, MonotonicClock::now());
            let _ = self.controller.append(&[point]);
        }
        if let Err(error) = self.commit_active() {
            self.record_error(error);
        }
        true
    }

    fn pointer_down(&mut self, wparam: WPARAM) {
        if self.controller.is_drawing() || self.mouse_down || self.active_pointer.is_some() {
            return;
        }
        let Some((id, info)) = pointer_info(wparam) else {
            return;
        };
        let points = self.pointer.next(info);
        if self.pointer.is_eraser() {
            // The inverted pen end is the eraser tool; tool behavior is Task 8.
            // Drawing ink with the eraser end would be plainly wrong, so the
            // contact is ignored (recorded as 未验证 in the stylus notes).
            return;
        }
        if points.is_empty() {
            return;
        }
        let pressure = self.pointer.kind() == PointerKind::Pen && self.pointer.pressure_supported();
        self.apply_style(pressure);
        let mut dirty = self.controller.begin(points[0]);
        dirty.union(&self.controller.append(&points[1..]));
        self.active_pointer = Some(id);
        self.after_append(dirty);
    }

    fn pointer_update(&mut self, wparam: WPARAM) {
        let Some(active) = self.active_pointer else {
            return;
        };
        let Some((id, info)) = pointer_info(wparam) else {
            return;
        };
        if id != active {
            return;
        }
        let points = self.pointer.next(info);
        if points.is_empty() {
            return;
        }
        let dirty = self.controller.append(&points);
        self.after_append(dirty);
    }

    fn pointer_up(&mut self, wparam: WPARAM) {
        let Some(active) = self.active_pointer else {
            return;
        };
        let Some((id, info)) = pointer_info(wparam) else {
            return;
        };
        if id != active {
            return;
        }
        let points = self.pointer.next(info);
        if !points.is_empty() {
            let _ = self.controller.append(&points);
        }
        self.active_pointer = None;
        if let Err(error) = self.commit_active() {
            self.record_error(error);
        }
    }

    /// A second contact, a lost capture or a mode change: discard the partial
    /// stroke and clear the active layer — never commit it.
    fn cancel_contact(&mut self) {
        if let Err(error) = self.drop_active() {
            self.record_error(error);
        }
    }

    fn drop_active(&mut self) -> Result<(), GfxError> {
        let region = self.controller.cancel();
        self.active_pointer = None;
        self.mouse_down = false;
        if region.is_empty() {
            return Ok(());
        }
        let Self { surface, .. } = self;
        let Some(surface) = surface.as_mut() else {
            return Ok(());
        };
        surface.clear_active(&region)?;
        surface.present(region).map(|_| ())
    }

    fn commit_active(&mut self) -> Result<(), GfxError> {
        let Some(region) = self.controller.end() else {
            return Ok(());
        };
        let Self {
            controller,
            surface,
            ..
        } = self;
        let Some(surface) = surface.as_mut() else {
            return Ok(());
        };
        // The static cache is additive on commit: draw the new stroke on top,
        // then clear its copy from the active layer.
        let Some(stroke) = controller.document().strokes().last() else {
            return Ok(());
        };
        surface.draw_committed(stroke, &region)?;
        surface.clear_active(&region)?;
        surface.present(region)?;
        self.committed = Some(controller.document().len());
        Ok(())
    }

    fn redraw_and_present(&mut self, region: &DirtyRegion) -> Result<(), GfxError> {
        let Self {
            controller,
            surface,
            ..
        } = self;
        let Some(surface) = surface.as_mut() else {
            return Ok(());
        };
        surface.redraw_static(controller.document(), region)?;
        surface.present(region.clone()).map(|_| ())
    }

    fn after_append(&mut self, region: DirtyRegion) {
        if region.is_empty() {
            return;
        }
        let result = {
            let Self {
                controller,
                surface,
                ..
            } = self;
            match surface.as_mut() {
                Some(surface) => match controller.active() {
                    Some(active) => surface
                        .draw_active(active, &region)
                        .and_then(|()| surface.present(region).map(|_| ())),
                    None => Ok(()),
                },
                None => Ok(()),
            }
        };
        if let Err(error) = result {
            self.record_error(error);
        }
    }

    fn apply_style(&mut self, pressure_enabled: bool) {
        let mut style = self.pen_style;
        style.pressure_enabled = pressure_enabled;
        self.controller.set_style(style);
    }

    fn client_position(&self, lparam: LPARAM) -> Option<PhysicalPointF> {
        let x = i32::from((lparam.0 & 0xFFFF) as u16 as i16);
        let y = i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16);
        let rect = self.surface.as_ref()?.window_rect().ok()?;
        Some(PhysicalPointF::new(
            (rect.left + x) as f32,
            (rect.top + y) as f32,
        ))
    }

    fn record_error(&mut self, error: GfxError) {
        if self.last_error.is_none() {
            self.last_error = Some(error.to_string());
        }
    }
}

fn pointer_info(wparam: WPARAM) -> Option<(u32, POINTER_INFO)> {
    let pointer_id = (wparam.0 & 0xFFFF) as u32;
    let mut info = POINTER_INFO::default();
    unsafe { GetPointerInfo(pointer_id, &mut info) }
        .ok()
        .map(|()| (pointer_id, info))
}

/// Window-procedure hook of the ink overlay.
///
/// # Safety
/// `data` is the live `Box<InkSession>` registered by [`InkSession::new`]; the
/// overlay clears the hook before the session (and its window) is destroyed.
unsafe fn session_hook(
    data: *mut core::ffi::c_void,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (handled, ops) = {
        // The borrow ends before the capture operations below: SetCapture and
        // ReleaseCapture synchronously send WM_CAPTURECHANGED back into this
        // procedure, which must be free to borrow the session itself.
        let session = unsafe { &mut *(data as *mut InkSession) };
        session.on_message(message, wparam, lparam)
    };
    for op in ops {
        match op {
            SessionOp::SetCapture => {
                unsafe { SetCapture(hwnd) };
            }
            SessionOp::ReleaseCapture => unsafe {
                let _ = ReleaseCapture();
            },
            SessionOp::SetCursor(cursor) => unsafe {
                SetCursor(Some(cursor));
            },
        }
    }
    // WM_SETCURSOR must return TRUE (1) to tell Windows the cursor was set;
    // every other handled message uses the conventional zero result.
    handled.then_some(if message == WM_SETCURSOR {
        LRESULT(1)
    } else {
        LRESULT(0)
    })
}

/// Converts a configured legacy pen width from HIMETRIC to physical pixels.
///
/// Microsoft.Ink `DrawingAttributes.Width` is 0.01 mm, not a pixel count.
/// Settings and the width slider stay in legacy HIMETRIC units; only rendering
/// converts them (80 -> 4.54 physical px at 144 DPI).
pub fn pen_style_for_dpi(pen: &PenSettings, dpi: f32) -> PenStyle {
    PenStyle::himetric(
        Rgba8::new(pen.color.red, pen.color.green, pen.color.blue, pen.alpha),
        f32::from(pen.width),
        dpi,
        false,
    )
}

fn display_dpi(session: &InkSession) -> f32 {
    session
        .surface()
        .map(|surface| crate::platform::dpi::DpiMap::from_window(surface.hwnd()).dpi() as f32)
        .unwrap_or(96.0)
}

fn default_pen_index(settings: &Settings) -> usize {
    if settings.pens[1].enabled {
        1
    } else {
        settings
            .pens
            .iter()
            .position(|pen| pen.enabled)
            .unwrap_or(1)
    }
}

/// The hidden shell window. `Drop` clears the window-procedure back-pointer
/// before destroying the HWND, so no message can reach a half-dropped shell.
struct ShellWindow {
    hwnd: HWND,
}

impl ShellWindow {
    fn create(shell: &Shell) -> Result<Self, AppError> {
        let instance =
            unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|source| AppError::Window {
                context: "getting the module instance (GetModuleHandleW)",
                source,
            })?;
        let class_name = HSTRING::from(SHELL_WINDOW_CLASS);
        let title = HSTRING::from(SHELL_WINDOW_TITLE);
        let window_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(shell_window_proc),
            hInstance: HINSTANCE(instance.0),
            hCursor: default_class_cursor(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassExW(&window_class) };
        if atom == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(AppError::Window {
                    context: "registering the shell window class (RegisterClassExW)",
                    source: WindowsError::from_hresult(HRESULT::from_win32(error.0)),
                });
            }
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                &class_name,
                &title,
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                Some(shell as *const Shell as *const core::ffi::c_void),
            )
        }
        .map_err(|source| AppError::Window {
            context: "creating the shell window (CreateWindowExW)",
            source,
        })?;
        Ok(Self { hwnd })
    }

    fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn destroy(&mut self) {
        if self.hwnd.is_invalid() {
            return;
        }
        let hwnd = self.hwnd;
        self.hwnd = HWND::default();
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
        }
    }
}

impl Drop for ShellWindow {
    fn drop(&mut self) {
        self.destroy();
    }
}

struct Shell {
    // Declared first so the window is destroyed before the state its message
    // procedure may touch is dropped.
    window: Option<ShellWindow>,
    tray: Option<TrayIcon>,
    hotkeys: Option<GlobalHotkeys>,
    io: Option<IoWorker>,
    completions: Option<Arc<PostMessageCompletion>>,
    // Drawing stack, in dependency order: the toolbar is an owned popup of
    // the overlay and must therefore be destroyed before its owner. Both use
    // the graphics device. They are created lazily on the first drawing
    // session and stay alive until shutdown.
    pen_width: Option<PenWidthPanel>,
    toolbar: Option<ToolbarUiWindow>,
    ink: Option<Box<InkSession>>,
    gfx: Option<GraphicsDevice>,
    annotation_escape: bool,
    settings: Settings,
    options: RunOptions,
    mode: AppMode,
    pen_index: usize,
    global_pen_width: f32,
    log: LogSink,
    exit_requested: bool,
    menu_open: bool,
    shutdown_done: bool,
    _not_send: PhantomData<Rc<()>>,
}

impl Shell {
    fn new(settings: Settings, options: RunOptions, log: LogSink) -> Self {
        let pen_index = default_pen_index(&settings);
        Self {
            window: None,
            tray: None,
            hotkeys: None,
            io: None,
            completions: None,
            pen_width: None,
            ink: None,
            toolbar: None,
            gfx: None,
            annotation_escape: false,
            settings,
            options,
            mode: AppMode::Tray,
            pen_index,
            global_pen_width: DEFAULT_WIDTH_HIMETRIC,
            log,
            exit_requested: false,
            menu_open: false,
            shutdown_done: false,
            _not_send: PhantomData,
        }
    }

    fn start_services(&mut self, hwnd: HWND) -> Result<(), AppError> {
        let mut tray = TrayIcon::new(hwnd, TRAY_CALLBACK_MESSAGE, self.settings.white_tray_icon)?;
        if let Some(warning) = tray.take_asset_warning() {
            self.log.warn(&warning);
        }
        self.tray = Some(tray);
        self.log.info(&format!(
            "tray icon added (legacy {} ICO)",
            crate::assets::tray_icon_filename(self.settings.white_tray_icon)
        ));

        let map = HotkeyMap::from_settings(&self.settings);
        let (hotkeys, report) = GlobalHotkeys::register(&map, Some(hwnd), !self.options.no_hotkeys);
        if self.options.no_hotkeys {
            self.log
                .info("global hotkeys not registered (development switch)");
        } else {
            self.log.info(&format!(
                "registered {} global hotkey(s)",
                report.registered.len()
            ));
            for issue in &report.issues {
                self.log.warn(&format!("hotkey issue: {issue}"));
            }
            self.report_hotkey_issues(&report.issues);
        }
        self.hotkeys = Some(hotkeys);

        let completions = Arc::new(PostMessageCompletion::new(hwnd, IO_RESULT_MESSAGE));
        let worker =
            IoWorker::start(IO_QUEUE_CAPACITY, completions.clone()).map_err(AppError::IoWorker)?;
        self.completions = Some(completions);
        self.io = Some(worker);

        if let Some(delay) = self.options.exit_after {
            let milliseconds = delay.as_millis().clamp(1, u128::from(u32::MAX)) as u32;
            if unsafe { SetTimer(Some(hwnd), EXIT_TIMER_ID, milliseconds, None) } == 0 {
                return Err(AppError::Window {
                    context: "arming the development exit timer (SetTimer)",
                    source: WindowsError::from_thread(),
                });
            }
            self.log.info(&format!(
                "development exit timer armed for {milliseconds} ms"
            ));
        }
        Ok(())
    }

    fn report_hotkey_issues(&self, issues: &[HotkeyIssue]) {
        if issues.is_empty() {
            return;
        }
        let mut names: Vec<&str> = issues.iter().map(|issue| issue.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        let message = format!(
            "{} hotkey(s) could not be used: {}. Details were written to plnk.log.",
            issues.len(),
            names.join(", ")
        );
        if let Some(tray) = &self.tray
            && let Err(error) = tray.show_warning("plnk hotkeys", &message)
        {
            self.log
                .warn(&format!("could not show the hotkey warning: {error}"));
        }
    }

    fn on_hotkey(&mut self, id: i32) {
        let Some(command) = self
            .hotkeys
            .as_ref()
            .and_then(|hotkeys| hotkeys.command_for_id(id))
        else {
            self.log.warn(&format!("unknown WM_HOTKEY id {id}"));
            return;
        };
        self.handle_command(command);
    }

    fn on_io_result(&mut self, id: u64) {
        let Some(completions) = &self.completions else {
            return;
        };
        let Some(result) = completions.take(id) else {
            return;
        };
        match result.outcome {
            Ok(_) => self
                .log
                .info(&format!("file task '{}' completed", result.label)),
            Err(message) => self
                .log
                .warn(&format!("file task '{}' failed: {message}", result.label)),
        }
    }

    fn handle_command(&mut self, command: Command) {
        let previous = self.mode;
        let next = previous.after_command(command);
        if next == previous {
            return;
        }
        match self.apply_mode(previous, next) {
            Ok(()) => {
                self.mode = next;
                self.log.info(&format!(
                    "command {command:?}: mode {previous:?} -> {next:?}"
                ));
            }
            Err(error) => {
                self.log.warn(&format!(
                    "command {command:?}: could not enter {next:?} ({error}); staying in {previous:?}"
                ));
            }
        }
        self.log_ink_error();
    }

    /// Applies the side effects of a mode transition. On failure the caller
    /// keeps the previous mode so the state machine never claims a mode whose
    /// windows failed to appear.
    fn apply_mode(&mut self, previous: AppMode, next: AppMode) -> Result<(), AppError> {
        match (previous, next) {
            (AppMode::Tray, AppMode::Drawing) => self.enter_annotation(),
            (AppMode::Drawing, AppMode::Pointer) => self.set_ink_click_through(true),
            (AppMode::Pointer, AppMode::Drawing) => self.set_ink_click_through(false),
            _ => Ok(()),
        }
    }

    /// Creates the graphics device, drawing session and toolbar on first use.
    fn ensure_drawing_stack(&mut self) -> Result<(), AppError> {
        if self.gfx.is_none() {
            let device = GraphicsDevice::new().map_err(|source| AppError::Graphics {
                context: "creating the graphics device",
                source,
            })?;
            self.gfx = Some(device);
            self.log.info("graphics device created");
        }
        if self.ink.is_some() && self.toolbar.is_some() && self.pen_width.is_some() {
            return Ok(());
        }
        let bounds = current_monitor_rect().map_err(|source| AppError::Graphics {
            context: "querying the current monitor",
            source,
        })?;
        if self.ink.is_none() {
            let Some(device) = self.gfx.as_ref() else {
                return Err(AppError::Runtime(
                    "graphics device missing after initialization".to_owned(),
                ));
            };
            let mut session =
                InkSession::new(device, bounds).map_err(|source| AppError::Graphics {
                    context: "creating the ink overlay",
                    source,
                })?;
            for warning in session.take_cursor_warnings() {
                self.log.warn(&warning);
            }
            self.log.info(&format!(
                "original pen cursors loaded: {}/4",
                session.pen_cursor_count()
            ));
            self.ink = Some(session);
            self.refresh_pen_style();
            self.log.info(&format!(
                "ink overlay created at {}x{} physical pixels",
                bounds.width(),
                bounds.height()
            ));
        }
        if self.toolbar.is_none() {
            let Some(device) = self.gfx.as_ref() else {
                return Err(AppError::Runtime(
                    "graphics device missing after initialization".to_owned(),
                ));
            };
            let mut toolbar = ToolbarUiWindow::new(device, &self.settings).map_err(|source| {
                AppError::Graphics {
                    context: "creating the toolbar window",
                    source,
                }
            })?;
            if let Some(surface) = self.ink.as_ref().and_then(|session| session.surface()) {
                toolbar
                    .set_owner(surface.hwnd())
                    .map_err(|source| AppError::Graphics {
                        context: "keeping the toolbar above the ink overlay",
                        source,
                    })?;
            }
            if let Some(window) = self.window.as_ref() {
                toolbar.set_click_notifier(window.hwnd(), TOOLBAR_CLICK_MESSAGE);
            }
            for warning in toolbar.take_asset_warnings() {
                self.log.warn(&warning);
            }
            self.log.info(&format!(
                "original toolbar bitmaps loaded: {}/19",
                toolbar.legacy_bitmap_count()
            ));
            self.toolbar = Some(toolbar);
            self.log.info("toolbar window created");
        }
        if self.pen_width.is_none() {
            let Some(device) = self.gfx.as_ref() else {
                return Err(AppError::Runtime(
                    "graphics device missing after initialization".to_owned(),
                ));
            };
            let owner = self
                .toolbar
                .as_ref()
                .map_or(HWND::default(), |toolbar| toolbar.hwnd());
            let mut panel =
                PenWidthPanel::new(device, owner).map_err(|source| AppError::Graphics {
                    context: "creating the pen-width popup",
                    source,
                })?;
            if let Some(window) = self.window.as_ref() {
                panel.set_change_notifier(window.hwnd(), PEN_WIDTH_CHANGE_MESSAGE);
            }
            for warning in panel.take_asset_warnings() {
                self.log.warn(&warning);
            }
            self.log.info(&format!(
                "original pen-width panel bitmap loaded: {}",
                panel.legacy_panel_bitmap_loaded()
            ));
            self.pen_width = Some(panel);
            self.log.info("pen-width popup created");
        }
        Ok(())
    }

    fn enter_annotation(&mut self) -> Result<(), AppError> {
        self.ensure_drawing_stack()?;
        if let Some(session) = self.ink.as_mut() {
            session.reopen_annotation();
            if let Some(surface) = session.surface_mut() {
                surface.set_click_through(false)?;
                surface.show();
            }
        }
        if let Some(toolbar) = self.toolbar.as_mut()
            && let Err(error) = toolbar.enter()
        {
            // Do not leave an interactive, inkless overlay behind when the
            // toolbar cannot appear; the mode machine stays in Tray.
            if let Some(session) = self.ink.as_mut() {
                if let Some(surface) = session.surface_mut() {
                    let _ = surface.set_click_through(true);
                }
                if let Some(surface) = session.surface() {
                    surface.hide();
                }
            }
            return Err(AppError::from(error));
        }
        if let Err(error) = self.register_annotation_escape() {
            self.exit_annotation("Escape registration failed");
            return Err(error);
        }
        Ok(())
    }

    fn register_annotation_escape(&mut self) -> Result<(), AppError> {
        if self.annotation_escape {
            return Ok(());
        }
        let Some(window) = self.window.as_ref() else {
            return Err(AppError::Runtime(
                "shell window missing while registering Escape".to_owned(),
            ));
        };
        unsafe {
            RegisterHotKey(
                Some(window.hwnd()),
                ANNOTATION_ESCAPE_HOTKEY_ID,
                HOT_KEY_MODIFIERS(0),
                u32::from(VK_ESCAPE.0),
            )
        }
        .map_err(|source| AppError::Window {
            context: "registering the annotation Escape hotkey",
            source,
        })?;
        self.annotation_escape = true;
        self.log.info("annotation Escape hotkey registered");
        Ok(())
    }

    fn unregister_annotation_escape(&mut self) {
        if !self.annotation_escape {
            return;
        }
        self.annotation_escape = false;
        if let Some(window) = self.window.as_ref() {
            let result =
                unsafe { UnregisterHotKey(Some(window.hwnd()), ANNOTATION_ESCAPE_HOTKEY_ID) };
            if let Err(error) = result {
                self.log
                    .warn(&format!("could not unregister Escape: {error}"));
            }
        }
        self.log.info("annotation Escape hotkey unregistered");
    }

    fn on_escape(&mut self) {
        if self
            .pen_width
            .as_ref()
            .is_some_and(|panel| panel.is_visible())
        {
            self.hide_pen_width_panel();
            self.log.info("pen width panel closed by Escape");
        } else {
            self.exit_annotation("Escape");
        }
    }

    /// Back to tray mode: drop a live contact, clear through the history, hide
    /// the overlay and slide the toolbar out. The graphics stack stays alive for
    /// the next session (undo restoring pre-exit strokes is same-process only).
    fn exit_annotation(&mut self, reason: &str) {
        self.log.info(&format!("leaving annotation: {reason}"));
        self.unregister_annotation_escape();
        self.hide_pen_width_panel();
        if let Some(session) = self.ink.as_mut() {
            if let Err(error) = session.close_annotation() {
                self.log.warn(&format!("could not clear ink: {error}"));
            }
            if let Some(surface) = session.surface_mut()
                && let Err(error) = surface.set_click_through(true)
            {
                self.log
                    .warn(&format!("could not enable click-through: {error}"));
            }
            if let Some(surface) = session.surface() {
                surface.hide();
            }
        }
        if let Some(toolbar) = self.toolbar.as_mut()
            && let Err(error) = toolbar.fast_exit()
        {
            self.log
                .warn(&format!("could not hide the toolbar: {error}"));
        }
        self.mode = AppMode::Tray;
        self.log_ink_error();
    }

    fn set_ink_click_through(&mut self, enabled: bool) -> Result<(), AppError> {
        if let Some(session) = self.ink.as_mut() {
            // A mode flip is a cancellation point: a partial stroke must never
            // commit when input pass-through changes mid-contact.
            if let Err(error) = session.cancel_active() {
                self.log
                    .warn(&format!("could not cancel the live stroke: {error}"));
            }
            if let Some(surface) = session.surface_mut() {
                surface.set_click_through(enabled)?;
            }
            self.log.info(if enabled {
                "ink overlay is click-through (pointer mode)"
            } else {
                "ink overlay accepts input (drawing mode)"
            });
        }
        Ok(())
    }

    fn select_pen(&mut self, index: usize) {
        if index >= self.settings.pens.len() {
            self.log.warn(&format!("pen index {index} is out of range"));
            return;
        }
        self.hide_pen_width_panel();
        self.pen_index = index;
        self.refresh_pen_style();
        if let Some(toolbar) = self.toolbar.as_mut() {
            let mut model = toolbar.model().clone();
            model.set_active(Some(ButtonId::Pen(index as u8)));
            if let Err(error) = toolbar.set_model(model) {
                self.log
                    .warn(&format!("could not refresh the toolbar: {error}"));
            }
        }
        self.log.info(&format!("pen {index} selected"));
        // Selecting a pen leaves pointer mode (legacy `btPen_Click`).
        if self.mode == AppMode::Pointer {
            self.handle_command(Command::Pointer);
        }
    }

    fn refresh_pen_style(&mut self) {
        let index = self.pen_index;
        let global_width = self.global_pen_width;
        // The MVP always exposes the width slider, so its session-global width
        // must apply even when the imported legacy config says
        // `Pen_Width_Icon = False`; otherwise dragging the visible slider
        // would not change the pen.
        let override_width = pen_width_visible(&self.settings);
        if let Some(session) = self.ink.as_mut() {
            let dpi = display_dpi(session);
            let mut style = pen_style_for_dpi(&self.settings.pens[index], dpi);
            if override_width {
                // Legacy PenWidthEnabled overrides every pen's width globally;
                // color/alpha stay per-pen and the HIMETRIC value is session-only.
                style.width = PenStyle::himetric(style.color, global_width, dpi, false).width;
            }
            session.set_pen_style(style);
            self.log
                .info(&format!("active pen width: {:.3} physical px", style.width));
        }
    }

    fn toggle_pen_width(&mut self) {
        if self.mode != AppMode::Drawing {
            return;
        }
        if self
            .pen_width
            .as_ref()
            .is_some_and(|panel| panel.is_visible())
        {
            self.hide_pen_width_panel();
            return;
        }
        let Some(toolbar) = self.toolbar.as_ref() else {
            return;
        };
        let Some(button) = toolbar.button_screen_rect(ButtonId::PenWidth) else {
            return;
        };
        let toolbar_rect = toolbar.panel_rect();
        if let Some(panel) = self.pen_width.as_mut() {
            panel.show_above(toolbar_rect, button);
            self.log.info("pen width panel shown");
        }
    }

    fn hide_pen_width_panel(&mut self) {
        if let Some(panel) = self.pen_width.as_mut()
            && panel.is_visible()
        {
            panel.hide();
        }
    }

    fn on_pen_width_change(&mut self, width: f32) {
        if !width.is_finite() {
            return;
        }
        self.global_pen_width = width.clamp(30.0, 3000.0);
        self.refresh_pen_style();
        self.log.info(&format!(
            "global pen width: {:.0} HIMETRIC",
            self.global_pen_width
        ));
    }

    fn ink_undo(&mut self) {
        let result = self.ink.as_mut().map(|session| session.undo());
        match result {
            Some(Ok(())) => self.log.info("ink undone"),
            Some(Err(error)) => self.log.warn(&format!("undo failed: {error}")),
            None => {}
        }
        self.log_ink_error();
    }

    fn ink_clear(&mut self) {
        let result = self.ink.as_mut().map(|session| session.clear());
        match result {
            Some(Ok(())) => self.log.info("ink cleared"),
            Some(Err(error)) => self.log.warn(&format!("clear failed: {error}")),
            None => {}
        }
        self.log_ink_error();
    }

    /// Toolbar click delivered asynchronously by the toolbar window.
    fn on_toolbar_click(&mut self, code: usize) {
        let Some(button) = ButtonId::from_code(code as u32) else {
            self.log
                .warn(&format!("unknown toolbar button code {code}"));
            return;
        };
        self.log.info(&format!("toolbar click: {button:?}"));
        match button {
            ButtonId::Pen(index) => self.select_pen(usize::from(index)),
            ButtonId::Undo => self.ink_undo(),
            ButtonId::Clear => self.ink_clear(),
            ButtonId::PenWidth => self.toggle_pen_width(),
            ButtonId::Exit => self.exit_annotation("toolbar exit button"),
            ButtonId::Pointer => self.handle_command(Command::Pointer),
            // The dock slide is handled inside the toolbar window itself.
            ButtonId::Dock => {}
            other => self.log.info(&format!(
                "toolbar button {other:?} is not wired in the drawing MVP (Task 8)"
            )),
        }
        self.log_ink_error();
    }

    /// Surfaces deferred drawing errors (the input hook cannot log directly
    /// because it runs while the shell is not borrowed).
    fn log_ink_error(&mut self) {
        if let Some(count) = self
            .ink
            .as_mut()
            .and_then(|session| session.take_committed())
        {
            self.log
                .info(&format!("ink committed: {count} document strokes"));
        }
        if let Some(message) = self.ink.as_mut().and_then(|session| session.take_error()) {
            self.log.warn(&format!("drawing session: {message}"));
        }
    }

    /// Opens the settings window (tray menu "Options...").
    ///
    /// Only reachable from tray mode: during an annotation session the two
    /// system-wide hotkeys are registered and the overlay owns the pointer, so a
    /// modal settings window would fight the drawing session.
    fn open_options(&mut self) {
        if !crate::options::can_open_in(self.mode) {
            self.log.warn(&format!(
                "options requested while in {:?}; only tray mode may open them",
                self.mode
            ));
            if let Some(tray) = &self.tray {
                let _ = tray.show_warning("plnk", "请先退出批注（按 Esc），再打开设置。");
            }
            return;
        }
        match crate::ui::options_window::show(
            self.window.as_ref().map(|w| w.hwnd()),
            &self.settings,
        ) {
            Ok(Some(updated)) => {
                // Hotkeys changed: re-register so the new combinations take
                // effect without a restart, then persist.
                let changed = updated != self.settings;
                self.settings = updated;
                if changed {
                    self.reregister_hotkeys();
                    self.save_settings();
                }
            }
            Ok(None) => self.log.info("options cancelled"),
            Err(error) => self.log.warn(&format!("options window failed: {error}")),
        }
    }

    /// Re-registers the global hotkeys after a settings change.
    fn reregister_hotkeys(&mut self) {
        if let Some(mut hotkeys) = self.hotkeys.take() {
            let released = hotkeys.unregister_all();
            self.log.info(&format!(
                "{released} hotkey(s) released for re-registration"
            ));
        }
        let map = HotkeyMap::from_settings(&self.settings);
        let (hotkeys, report) = GlobalHotkeys::register(
            &map,
            self.window.as_ref().map(|w| w.hwnd()),
            !self.options.no_hotkeys,
        );
        if !self.options.no_hotkeys {
            self.log.info(&format!(
                "re-registered {} global hotkey(s)",
                report.registered.len()
            ));
            for issue in &report.issues {
                self.log.warn(&format!("hotkey issue: {issue}"));
            }
            self.report_hotkey_issues(&report.issues);
        }
        self.hotkeys = Some(hotkeys);
    }

    /// Persists the current settings. Failures are logged, not fatal: the user
    /// keeps a working session and the next successful save picks up the change.
    fn save_settings(&mut self) {
        let mut store = SettingsStore::default();
        match store.load_or_import() {
            Ok(_) => {}
            Err(error) => self
                .log
                .warn(&format!("settings reload before save failed: {error}")),
        }
        if let Err(error) = store.save(&self.settings) {
            self.log.warn(&format!("cannot save settings: {error}"));
            if let Some(tray) = &self.tray {
                let _ = tray.show_warning("plnk", &format!("设置保存失败：{error}"));
            }
        } else {
            self.log.info("settings saved");
        }
    }

    fn request_exit(&mut self, reason: &str) {
        if !self.exit_requested {
            self.exit_requested = true;
            self.log.info(&format!("exit requested: {reason}"));
        }
        // Post unconditionally: a nested modal loop (the tray menu) can consume
        // an earlier WM_QUIT, so a repeated request must still reach the pump.
        unsafe { PostQuitMessage(0) };
    }

    fn shutdown(&mut self) {
        if self.shutdown_done {
            return;
        }
        self.shutdown_done = true;
        self.log.info("shutting down");

        if let Some(mut io) = self.io.take() {
            io.close();
            self.log.info("file IO worker closed");
        }
        self.completions = None;

        if let Some(tray) = self.tray.as_mut() {
            let removed = tray.remove();
            self.log.info(if removed {
                "tray icon removed"
            } else {
                "tray icon removal failed"
            });
        }
        if let Some(mut hotkeys) = self.hotkeys.take() {
            let released = hotkeys.unregister_all();
            self.log
                .info(&format!("{released} global hotkey(s) unregistered"));
        }
        self.unregister_annotation_escape();
        if let Some(panel) = self.pen_width.take() {
            drop(panel);
            self.log.info("pen-width popup released");
        }
        if let Some(mut toolbar) = self.toolbar.take() {
            let _ = toolbar.fast_exit();
            let _ = toolbar.settle_now();
            drop(toolbar);
            self.log.info("toolbar released");
        }
        if let Some(mut ink) = self.ink.take() {
            if let Err(error) = ink.cancel_active() {
                self.log
                    .warn(&format!("could not cancel the live stroke: {error}"));
            }
            if let Some(surface) = ink.surface() {
                surface.hide();
            }
            drop(ink);
            self.log.info("ink surface released");
        }
        self.gfx = None;
        if let Some(mut window) = self.window.take() {
            window.destroy();
        }
        self.log.info("shutdown complete");
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.shutdown();
    }
}

unsafe extern "system" fn shell_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    match message {
        WM_HOTKEY => {
            if wparam.0 as i32 == ANNOTATION_ESCAPE_HOTKEY_ID {
                unsafe {
                    with_shell(hwnd, |shell| shell.on_escape());
                }
            } else {
                unsafe {
                    with_shell(hwnd, |shell| shell.on_hotkey(wparam.0 as i32));
                }
            }
            LRESULT(0)
        }
        TRAY_CALLBACK_MESSAGE => {
            unsafe { handle_tray_event(hwnd, lparam.0 as u32) };
            LRESULT(0)
        }
        IO_RESULT_MESSAGE => {
            unsafe {
                with_shell(hwnd, |shell| shell.on_io_result(wparam.0 as u64));
            }
            LRESULT(0)
        }
        TOOLBAR_CLICK_MESSAGE => {
            unsafe {
                with_shell(hwnd, |shell| shell.on_toolbar_click(wparam.0));
            }
            LRESULT(0)
        }
        PEN_WIDTH_CHANGE_MESSAGE => {
            unsafe {
                with_shell(hwnd, |shell| shell.on_pen_width_change(wparam.0 as f32));
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xFFFF {
                MENU_EXIT => unsafe {
                    with_shell(hwnd, |shell| shell.request_exit("tray menu Exit"));
                },
                MENU_OPTIONS => unsafe {
                    with_shell(hwnd, |shell| shell.open_options());
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                with_shell(hwnd, |shell| shell.request_exit("window close message"));
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == EXIT_TIMER_ID {
                unsafe {
                    with_shell(hwnd, |shell| shell.request_exit("development exit timer"));
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Runs the UI message pump. A free function so no `&mut Shell` is alive while
/// `DispatchMessageW` runs the window procedure.
fn pump_messages() -> Result<(), AppError> {
    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 == 0 {
            return Ok(());
        }
        if result.0 == -1 {
            return Err(AppError::MessageLoop(WindowsError::from_thread()));
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Calls `action` with a short-lived mutable reference to the shell. The borrow
/// ends before this function returns, so a handler that pumps messages (the
/// tray menu) cannot alias a second borrow created by re-entrant dispatch.
///
/// # Safety
/// `hwnd` must be the live shell window created by `ShellWindow::create`;
/// `action` must not dispatch messages or retain the reference.
unsafe fn with_shell(hwnd: HWND, action: impl FnOnce(&mut Shell)) {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Shell;
    if let Some(shell) = unsafe { pointer.as_mut() } {
        action(shell);
    }
}

/// # Safety
/// `hwnd` must be the live shell window.
unsafe fn handle_tray_event(hwnd: HWND, event: u32) {
    match event & 0xFFFF {
        WM_LBUTTONUP => unsafe {
            with_shell(hwnd, |shell| shell.handle_command(Command::ToggleInk));
        },
        WM_RBUTTONUP | WM_CONTEXTMENU => unsafe { show_tray_menu(hwnd) },
        _ => {}
    }
}

/// Displays the real popup menu. No shell reference is alive across
/// `TrackPopupMenu`: the menu's nested modal loop re-enters the window
/// procedure (for example for the Exit `WM_COMMAND`), which must be free to
/// borrow the shell itself.
///
/// # Safety
/// `hwnd` must be the live shell window.
unsafe fn show_tray_menu(hwnd: HWND) {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Shell;
    if pointer.is_null() {
        return;
    }
    if unsafe { (*pointer).menu_open } {
        return;
    }
    unsafe { (*pointer).menu_open = true };
    let result = TrayIcon::show_context_menu(hwnd);
    unsafe {
        (*pointer).menu_open = false;
        if let Err(error) = result {
            (*pointer).log.warn(&format!("{error}"));
        }
        // The menu's modal loop may have consumed the WM_QUIT posted by the
        // Exit command; post it again now that control is back in the pump.
        if (*pointer).exit_requested {
            PostQuitMessage(0);
        }
    }
}
