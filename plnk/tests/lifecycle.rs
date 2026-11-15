//! Application-shell lifecycle tests (Task 4).
//!
//! These tests never touch the running legacy gInk instance: the single-instance
//! names are test-local, global hotkey registrations use an obscure combination
//! and hold it only for the duration of one test, and the log files live in a
//! temporary directory that is removed at the end.
//!
//! The IO-worker test deliberately closes the worker while a job is still
//! running: `close` must return immediately and the late result must never be
//! delivered to the UI thread.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use plnk::app::{
    AppMode, IO_RESULT_MESSAGE, InstanceGuard, IoCompletion, IoResult, IoSubmitError, IoWorker,
    PostMessageCompletion, RunOptions,
};
use plnk::hotkeys::{Command, GlobalHotkeys, HotkeyIssueKind, HotkeyMap, KeyboardEvent};
use plnk::settings::{Hotkey, HotkeyKey, Settings};
use plnk::telemetry::{LogPolicy, LogSink};

fn unique_suffix() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Required test 1: a second instance must not replace the first; ownership is
/// released only when the first guard drops.
#[test]
fn second_instance_does_not_replace_first() {
    let name = format!(
        "Local\\plnk-lifecycle-instance-{}-{}",
        std::process::id(),
        unique_suffix()
    );

    let first = InstanceGuard::acquire(&name)
        .expect("creating the instance mutex must not fail")
        .expect("the first instance acquires the mutex");

    // A Windows mutex is recursive for its owning thread, so the "second
    // instance" is acquired from another thread, exactly like a second process.
    std::thread::scope(|scope| {
        let second = scope.spawn(|| {
            InstanceGuard::acquire(&name)
                .expect("second CreateMutexW must not fail")
                .is_none()
        });
        assert!(
            second.join().expect("second instance thread"),
            "a second instance must not replace the running first instance"
        );
        let third = scope.spawn(|| {
            InstanceGuard::acquire(&name)
                .expect("third CreateMutexW must not fail")
                .is_none()
        });
        assert!(
            third.join().expect("third instance thread"),
            "the first instance must keep ownership while its guard is alive"
        );
    });

    drop(first);
    let after = InstanceGuard::acquire(&name).expect("CreateMutexW after the first exited");
    assert!(
        after.is_some(),
        "dropping the first guard must release the singleton mutex"
    );
}

/// Required test 2: invalid and conflicting hotkeys are reported, never
/// silently ignored; a Windows registration refusal is also reported.
#[test]
fn invalid_or_conflicting_hotkey_is_reported() {
    // (a) A modifier without a key is invalid and must be reported by name.
    let mut invalid_settings = Settings::defaults();
    invalid_settings.hotkeys.global = Hotkey {
        control: true,
        alt: false,
        shift: false,
        win: false,
        key: None,
    };
    // Disable the second global hotkey so this case registers nothing real.
    invalid_settings.hotkeys.snap_pin = Hotkey::default();
    let invalid_map = HotkeyMap::from_settings(&invalid_settings);
    let (invalid_guard, invalid_report) = GlobalHotkeys::register(&invalid_map, None, true);
    assert!(
        invalid_report
            .issues
            .iter()
            .any(|issue| issue.name == "Hotkey_Global"
                && matches!(issue.kind, HotkeyIssueKind::Invalid { .. })),
        "invalid global hotkey must be reported: {:?}",
        invalid_report.issues
    );
    assert!(
        invalid_report.registered.is_empty(),
        "an invalid hotkey must not be registered"
    );
    drop(invalid_guard);

    // (b) Two settings entries with the same combination are a configured
    // conflict; only one may be registered and the other is reported.
    let mut conflicting_settings = Settings::defaults();
    let combination = Hotkey::parse("Ctrl + Alt + Shift + F9").expect("parse test combination");
    conflicting_settings.hotkeys.global = combination;
    conflicting_settings.hotkeys.snap_pin = combination;
    let conflicting_map = HotkeyMap::from_settings(&conflicting_settings);
    let (first_guard, first_report) = GlobalHotkeys::register(&conflicting_map, None, true);
    assert!(
        first_report
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, HotkeyIssueKind::Conflict { .. })),
        "configured duplicate hotkeys must be reported: {:?}",
        first_report.issues
    );

    // (c) While the first registration is alive, an identical registration is
    // refused by Windows; the refusal must be surfaced with the hotkey name.
    let (second_guard, second_report) = GlobalHotkeys::register(&conflicting_map, None, true);
    assert!(
        second_report
            .issues
            .iter()
            .any(|issue| issue.name == "Hotkey_Global"
                && matches!(issue.kind, HotkeyIssueKind::RegistrationFailed { .. })),
        "a combination already owned by another registration must be reported: {:?}",
        second_report.issues
    );

    drop(second_guard);
    drop(first_guard);
}

/// Required test 3: closing the app while a bounded-IO worker still runs drops
/// the late result and does not block the UI thread.
#[test]
fn close_during_pending_io_drops_result() {
    struct RecordingCompletion {
        delivered: Arc<Mutex<Vec<IoResult>>>,
    }

    impl IoCompletion for RecordingCompletion {
        fn complete(&self, result: IoResult) {
            self.delivered
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(result);
        }
    }

    let delivered = Arc::new(Mutex::new(Vec::new()));
    let sink: Arc<dyn IoCompletion> = Arc::new(RecordingCompletion {
        delivered: Arc::clone(&delivered),
    });
    let mut worker = IoWorker::start(4, sink).expect("start the bounded IO worker");

    let (started_sender, started_receiver) = std::sync::mpsc::channel();
    worker
        .submit("slow-job", move || {
            let _ = started_sender.send(());
            std::thread::sleep(Duration::from_millis(250));
            Ok("late result".to_owned())
        })
        .expect("submit the slow job");
    started_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("the slow job must start");

    worker
        .submit("queued-job", || Ok("queued result".to_owned()))
        .expect("submit the queued job");

    let close_started = Instant::now();
    worker.close();
    assert!(
        close_started.elapsed() < Duration::from_millis(100),
        "close must not wait for the in-flight job (took {:?})",
        close_started.elapsed()
    );
    assert_eq!(
        worker.submit("after-close", || Ok("never".to_owned())),
        Err(IoSubmitError::Closed),
        "submitting after close must be refused"
    );

    let deadline = Instant::now() + Duration::from_secs(3);
    while !worker.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        worker.is_finished(),
        "the worker thread must exit after the in-flight job finishes"
    );
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        delivered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty(),
        "results produced after close must be dropped, not delivered"
    );
}

/// The production delivery path: completed IO work wakes the UI thread with
/// `PostMessageW`, and the result is fetched by the message's id.
#[test]
fn io_results_are_posted_to_the_ui_thread() {
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, HWND_MESSAGE, MSG,
        PM_REMOVE, PeekMessageW, RegisterClassExW, TranslateMessage, WNDCLASSEXW, WS_OVERLAPPED,
    };
    use windows::core::{PCWSTR, w};

    unsafe extern "system" fn message_only_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }.expect("GetModuleHandleW");
    let class = w!("plnk_lifecycle_message_window");
    let window_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(message_only_proc),
        hInstance: HINSTANCE(instance.0),
        lpszClassName: class,
        ..Default::default()
    };
    assert_ne!(
        unsafe { RegisterClassExW(&window_class) },
        0,
        "RegisterClassExW"
    );
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            class,
            w!("plnk lifecycle test"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
    }
    .expect("create the message-only window");

    let completion = Arc::new(PostMessageCompletion::new(hwnd, IO_RESULT_MESSAGE));
    let worker = IoWorker::start(2, completion.clone()).expect("start the worker");
    worker
        .submit("posted-result", || Ok("delivered".to_owned()))
        .expect("submit the job");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut delivered = None;
    let mut message = MSG::default();
    while Instant::now() < deadline && delivered.is_none() {
        while unsafe { PeekMessageW(&mut message, Some(hwnd), 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == IO_RESULT_MESSAGE {
                delivered = completion.take(message.wParam.0 as u64);
            } else {
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let delivered = delivered.expect("the IO result must arrive through PostMessageW");
    assert_eq!(delivered.label, "posted-result");
    match delivered.outcome {
        Ok(payload) => assert_eq!(payload, "delivered"),
        Err(error) => panic!("IO job failed: {error}"),
    }

    drop(worker);
    let _ = unsafe { DestroyWindow(hwnd) };
}

/// Required test 4: the log stays within a fixed total size and rolls.
#[test]
fn log_rotation_is_bounded() {
    let directory = std::env::temp_dir().join(format!(
        "plnk-log-rotation-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&directory).expect("create the log test directory");
    let path = directory.join("plnk.log");
    let policy = LogPolicy {
        max_file_bytes: 512,
        max_files: 3,
    };
    let log = LogSink::with_policy(path.clone(), policy).expect("open the bounded log");

    for index in 0..200 {
        log.info(&format!("line {index} {}", "x".repeat(40)));
    }

    let files = log_files(&directory);
    let total: u64 = files.iter().map(|(_, size)| *size).sum();
    assert!(
        files.len() <= policy.max_files,
        "log must keep at most {} files, found {}",
        policy.max_files,
        files.len()
    );
    assert!(
        total <= policy.max_file_bytes * policy.max_files as u64,
        "rolled log exceeds its fixed bound: {total} bytes in {files:?}"
    );
    let current = std::fs::read_to_string(&path).expect("read the current log file");
    assert!(
        current.contains("line 199"),
        "the newest line must be in the current log file"
    );

    // A pathological single line is truncated instead of blowing the bound.
    log.info(&"y".repeat(10_000));
    let total: u64 = log_files(&directory).iter().map(|(_, size)| *size).sum();
    assert!(
        total <= policy.max_file_bytes * policy.max_files as u64,
        "a single oversized line must not break the log bound: {total} bytes"
    );

    drop(log);
    let _ = std::fs::remove_dir_all(&directory);
}

fn log_files(directory: &std::path::Path) -> Vec<(std::path::PathBuf, u64)> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).expect("list the log directory") {
        let entry = entry.expect("read a log directory entry");
        let metadata = entry.metadata().expect("read log metadata");
        if metadata.is_file() {
            files.push((entry.path(), metadata.len()));
        }
    }
    files
}

/// The mode must be one explicit state that later drawing/pointer/snapping
/// tasks extend; transitions are testable without any window.
#[test]
fn app_mode_transitions_are_explicit() {
    assert_eq!(
        AppMode::Tray.after_command(Command::ToggleInk),
        AppMode::Drawing
    );
    assert_eq!(
        AppMode::Drawing.after_command(Command::ToggleInk),
        AppMode::Pointer
    );
    assert_eq!(
        AppMode::Pointer.after_command(Command::ToggleInk),
        AppMode::Drawing
    );
    assert_eq!(
        AppMode::Snapping.after_command(Command::ToggleInk),
        AppMode::Drawing
    );
    assert_eq!(
        AppMode::Tray.after_command(Command::SnapPin),
        AppMode::Snapping
    );
    assert_eq!(
        AppMode::Drawing.after_command(Command::Snapshot),
        AppMode::Snapping
    );
    assert_eq!(
        AppMode::Drawing.after_command(Command::Pointer),
        AppMode::Pointer
    );
    assert_eq!(
        AppMode::Pointer.after_command(Command::Pointer),
        AppMode::Drawing
    );
    assert_eq!(
        AppMode::Drawing.after_command(Command::Undo),
        AppMode::Drawing
    );
}

/// `HotkeyMap::matches` is the keyboard-event entry point later tasks use for
/// non-system-wide tool hotkeys.
#[test]
fn hotkey_map_matches_keyboard_events() {
    let map = HotkeyMap::from_settings(&Settings::defaults());
    assert_eq!(
        map.matches(KeyboardEvent {
            control: true,
            alt: true,
            shift: false,
            win: false,
            key: HotkeyKey::Letter('G'),
        }),
        Some(Command::ToggleInk)
    );
    assert_eq!(
        map.matches(KeyboardEvent {
            control: true,
            alt: false,
            shift: false,
            win: false,
            key: HotkeyKey::Letter('G'),
        }),
        None,
        "modifiers must match exactly"
    );
    assert_eq!(
        map.matches(KeyboardEvent {
            control: false,
            alt: false,
            shift: false,
            win: false,
            key: HotkeyKey::Function(12),
        }),
        None,
        "unconfigured keys must not produce a command"
    );
}

/// The development flags are part of the required isolation from the legacy
/// app: `--no-hotkeys` must be understood, and the smoke exit hook is opt-in.
#[test]
fn run_options_parse_development_flags() {
    let options = RunOptions::from_args(["--no-hotkeys", "--exit-after-ms", "250"])
        .expect("parse development flags");
    assert!(options.no_hotkeys);
    assert_eq!(options.exit_after, Some(Duration::from_millis(250)));

    let defaults = RunOptions::from_args(std::iter::empty::<&str>()).expect("parse no flags");
    assert!(!defaults.no_hotkeys);
    assert_eq!(defaults.exit_after, None);

    let error = RunOptions::from_args(["--unknown-flag"]).expect_err("unknown flags are errors");
    assert!(
        error.to_string().contains("--unknown-flag"),
        "the error must name the offending argument: {error}"
    );
}
