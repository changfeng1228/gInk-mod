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
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetMessageW, GetWindowLongPtrW, MB_ICONERROR, MB_OK, MSG, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SetTimer, SetWindowLongPtrW, TranslateMessage, WM_APP,
    WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WM_NCCREATE,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows::core::{Error as WindowsError, HRESULT, HSTRING, PCWSTR, w};

use crate::error::SettingsError;
use crate::hotkeys::{Command, GlobalHotkeys, HotkeyIssue, HotkeyMap};
use crate::settings::Settings;
use crate::settings_store::StorageRoots;
use crate::telemetry::{LogError, LogSink};
use crate::tray::{MENU_EXIT, TrayError, TrayIcon};

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
/// legacy app's combinations; `--exit-after-ms` lets the desktop smoke test
/// start and stop the real shell without synthetic input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOptions {
    pub no_hotkeys: bool,
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
                        "unknown argument: {argument} (supported: --no-hotkeys, --exit-after-ms <ms>)"
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

    /// Runs with development options; `main` uses this for `--no-hotkeys` and
    /// the desktop smoke exit hook.
    pub fn run_with(settings: Settings, options: RunOptions) -> Result<(), AppError> {
        let instance = InstanceGuard::acquire(INSTANCE_MUTEX_NAME).map_err(AppError::Instance)?;
        let Some(_instance) = instance else {
            // Legacy behaviour: a second copy exits silently and never touches
            // the first instance's tray icon, hotkeys or log.
            eprintln!("plnk: another instance is already running");
            return Ok(());
        };

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
        let mut shell = Box::new(Shell::new(settings, options, log));
        let window = ShellWindow::create(shell.as_ref())?;
        let hwnd = window.hwnd();
        shell.window = Some(window);
        shell.start_services(hwnd)?;
        shell.message_loop()
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
    settings: Settings,
    options: RunOptions,
    mode: AppMode,
    log: LogSink,
    exit_requested: bool,
    menu_open: bool,
    shutdown_done: bool,
    _not_send: PhantomData<Rc<()>>,
}

impl Shell {
    fn new(settings: Settings, options: RunOptions, log: LogSink) -> Self {
        Self {
            window: None,
            tray: None,
            hotkeys: None,
            io: None,
            completions: None,
            settings,
            options,
            mode: AppMode::Tray,
            log,
            exit_requested: false,
            menu_open: false,
            shutdown_done: false,
            _not_send: PhantomData,
        }
    }

    fn start_services(&mut self, hwnd: HWND) -> Result<(), AppError> {
        self.tray = Some(TrayIcon::new(hwnd, TRAY_CALLBACK_MESSAGE)?);
        self.log.info("tray icon added");

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

    fn message_loop(&mut self) -> Result<(), AppError> {
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

    fn on_tray_event(&mut self, event: u32) {
        match event & 0xFFFF {
            WM_LBUTTONUP => self.handle_command(Command::ToggleInk),
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                // TrackPopupMenu runs its own message loop; a second tray click
                // delivered inside it must not open a nested menu.
                if self.menu_open {
                    return;
                }
                self.menu_open = true;
                let result = match &self.tray {
                    Some(tray) => tray.show_context_menu(),
                    None => Ok(()),
                };
                self.menu_open = false;
                if let Err(error) = result {
                    self.log.warn(&format!("{error}"));
                }
            }
            _ => {}
        }
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
        self.mode = previous.after_command(command);
        self.log.info(&format!(
            "command {command:?}: mode {previous:?} -> {:?}",
            self.mode
        ));
    }

    fn request_exit(&mut self, reason: &str) {
        if self.exit_requested {
            return;
        }
        self.exit_requested = true;
        self.log.info(&format!("exit requested: {reason}"));
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

        if let Some(mut tray) = self.tray.take() {
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
    let shell = unsafe { shell_from(hwnd) };
    match message {
        WM_HOTKEY => {
            if let Some(shell) = shell {
                shell.on_hotkey(wparam.0 as i32);
            }
            LRESULT(0)
        }
        TRAY_CALLBACK_MESSAGE => {
            if let Some(shell) = shell {
                shell.on_tray_event(lparam.0 as u32);
            }
            LRESULT(0)
        }
        IO_RESULT_MESSAGE => {
            if let Some(shell) = shell {
                shell.on_io_result(wparam.0 as u64);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            if let Some(shell) = shell
                && (wparam.0 & 0xFFFF) == MENU_EXIT
            {
                shell.request_exit("tray menu Exit");
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            if let Some(shell) = shell {
                shell.request_exit("window close message");
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(shell) = shell
                && wparam.0 == EXIT_TIMER_ID
            {
                shell.request_exit("development exit timer");
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

/// # Safety
/// `hwnd` is only dereferenced when it was created by `ShellWindow::create`,
/// whose lifetime guarantees the pointer stays valid within that window's
/// lifetime; `ShellWindow::destroy` clears the pointer before destruction.
unsafe fn shell_from(hwnd: HWND) -> Option<&'static mut Shell> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Shell;
    unsafe { pointer.as_mut() }
}
