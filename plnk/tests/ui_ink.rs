//! Ink input and presentation acceptance (plan Task 7).
//!
//! Headless tests (run by default):
//! - `coalesced_history_preserves_order`: `GetPointer*InfoHistory` returns
//!   coalesced samples in *reverse chronological* order (newest first); the
//!   normalizer must emit them chronologically (oldest first) so a stroke
//!   follows the physical pen path.
//! - `contact_cancel_does_not_commit_partial_history`: a cancelled contact
//!   (pointer capture lost, window left, Esc) must drop the active stroke
//!   without touching the document, its undo/redo history or the static layer.
//!
//! Desktop tests (`#[ignore]`, run with `--ignored --test-threads=1`):
//! - `present_only_submits_dirty_region`: a real DirectComposition overlay
//!   presents from static/active caches; the frame reports exactly the dirty
//!   region it was given, rasterizes no pixels outside it, and transfers no CPU
//!   pixel data (no full-screen bitmap upload).
//! - `pointer_timestamp_to_submit_is_monotonic`: pointer QPC timestamps,
//!   normalized stroke timestamps and submit timestamps are non-decreasing and
//!   input never appears to arrive after it is presented.
//! - `mvp_end_to_end_mouse_writing_selects_undo_clear_and_exits`: real routed
//!   mouse input (SendInput) writes on the overlay, toolbar clicks select a
//!   pen, undo and clear work, and the session shuts down cleanly.
//! - `ink_mvp_probe`: Release-only measurement of private memory after 1000
//!   mixed strokes and of pointer-to-submit CPU latency (spec §4.2).
//!
//! Run the desktop tests with:
//! `cargo test --manifest-path plnk/Cargo.toml --test ui_ink -- --ignored --test-threads=1`
//! Run the probe in Release with:
//! `cargo test --release --manifest-path plnk/Cargo.toml --test ui_ink -- --ignored --test-threads=1 ink_mvp_probe --nocapture`

use std::process::Command;
use std::time::{Duration, Instant};

use plnk::app::InkSession;
use plnk::gfx::Rgba8;
use plnk::gfx::device::GraphicsDevice;
use plnk::gfx::ink_surface::InkSurface;
use plnk::ink::{DirtyRect, DirtyRegion, InkController, PenPoint, PenStyle, PhysicalPointF};
use plnk::input::{PointerKind, PointerNormalizer, PointerSample};
use plnk::platform::dpi::{PhysicalPoint, PhysicalRect};
use plnk::platform::window::{
    MonotonicClock, ToolbarWindow as BackdropWindow, current_monitor_rect,
};
use plnk::settings::Settings;
use plnk::ui::toolbar::{ButtonId, ToolbarModel, ToolbarWindow};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DEVMODEW, DIB_RGB_COLORS, DeleteDC, DeleteObject, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW,
    GetDC, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, mouse_event,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, GetCursorPos, MSG, PM_REMOVE, PeekMessageW, PostMessageW,
    SetCursorPos, TranslateMessage, WM_CLOSE,
};
use windows::core::{PCWSTR, w};

// ---------------------------------------------------------------------------
// Step 1 tests: normalization and cancel semantics (headless)
// ---------------------------------------------------------------------------

#[test]
fn coalesced_history_preserves_order() {
    // Win32 hands back pointer history newest-first; a burst of coalesced
    // samples from one WM_POINTER message must reach the stroke in physical
    // order, not reversed.
    let frequency = qpc_frequency();
    let millisecond = (frequency / 1000).max(1);
    // Establish the process clock before the samples: the normalizer converts
    // QPC readings into this same time base. The clock epoch is the first-use
    // reading, so leave a margin before the burst.
    let _ = MonotonicClock::now();
    std::thread::sleep(Duration::from_millis(6));
    let newest = qpc_now();
    let mut normalizer = PointerNormalizer::new();
    let newest_first = [
        PointerSample::new(14, 4, None, newest),
        PointerSample::new(12, 3, None, newest - millisecond),
        PointerSample::new(9, 2, None, newest - 2 * millisecond),
        PointerSample::new(5, 1, None, newest - 3 * millisecond),
        PointerSample::new(0, 0, None, newest - 4 * millisecond),
    ];
    let points = normalizer.normalize(&newest_first);
    let xs: Vec<f32> = points.iter().map(|point| point.position.x).collect();
    assert_eq!(
        xs,
        vec![0.0, 5.0, 9.0, 12.0, 14.0],
        "coalesced samples must be emitted oldest first"
    );
    let ys: Vec<f32> = points.iter().map(|point| point.position.y).collect();
    assert_eq!(ys, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    assert!(
        points
            .windows(2)
            .all(|pair| pair[0].timestamp <= pair[1].timestamp),
        "timestamps must be non-decreasing within one coalesced frame"
    );
    let span = points[4].timestamp - points[0].timestamp;
    assert!(
        span >= Duration::from_millis(3) && span <= Duration::from_millis(5),
        "four QPC milliseconds must survive the conversion to the process clock, got {span:?}"
    );
    assert!(
        points
            .iter()
            .all(|point| (point.pressure - 1.0).abs() < 1e-6),
        "a device that reports no pressure draws at full pressure"
    );

    // A later frame whose oldest sample predates the previous newest timestamp
    // (clock reordering, wrapped device counter) must be clamped, not reordered.
    let older = [PointerSample::new(
        16,
        5,
        Some(0.5),
        newest - 8 * millisecond,
    )];
    let clamped = normalizer.normalize(&older);
    assert_eq!(clamped.len(), 1);
    assert_eq!(
        clamped[0].timestamp, points[4].timestamp,
        "out-of-order samples must clamp to the last emitted timestamp"
    );
    assert!((clamped[0].pressure - 0.5).abs() < 1e-6);

    // Kind and pressure support are recorded from the raw pointer path; the
    // headless path leaves the defaults intact.
    assert_eq!(normalizer.kind(), PointerKind::Other);
}

#[test]
fn contact_cancel_does_not_commit_partial_history() {
    let mut controller = InkController::new();
    controller.set_style(PenStyle::fixed(Rgba8::new(10, 10, 10, 255), 8.0));

    // One committed stroke so the history has a baseline to protect.
    controller.begin(pen_point(10.0, 10.0, 0));
    controller.append(&[pen_point(20.0, 10.0, 1), pen_point(30.0, 10.0, 2)]);
    let committed = controller.end().expect("a non-empty stroke commits");
    assert!(!committed.is_empty());
    let strokes = controller.document().len();
    let undo_depth = controller.document().undo_depth();
    let redo_depth = controller.document().redo_depth();
    assert_eq!(strokes, 1);

    // A contact that is cancelled mid-stroke must leave everything untouched.
    let begin_dirty = controller.begin(pen_point(100.0, 100.0, 3));
    assert!(!begin_dirty.is_empty());
    controller.append(&[pen_point(140.0, 120.0, 4), pen_point(180.0, 160.0, 5)]);
    assert!(controller.active().is_some(), "the stroke is live");
    let cancel_dirty = controller.cancel();
    assert!(
        !cancel_dirty.is_empty(),
        "cancel must return the pixels to repaint"
    );
    assert!(controller.active().is_none());
    assert_eq!(controller.document().len(), strokes);
    assert_eq!(controller.document().undo_depth(), undo_depth);
    assert_eq!(controller.document().redo_depth(), redo_depth);
    let last = controller
        .document()
        .strokes()
        .last()
        .expect("the committed stroke is intact");
    assert_eq!(last.len(), 3);

    // Cancelling with no active contact is a safe no-op.
    assert!(controller.cancel().is_empty());

    // The next stroke begins cleanly and commits only its own points.
    controller.begin(pen_point(200.0, 200.0, 6));
    controller.append(&[pen_point(210.0, 205.0, 7)]);
    let committed = controller.end().expect("the new stroke commits");
    assert!(!committed.is_empty());
    assert_eq!(controller.document().len(), strokes + 1);
    let newest = controller
        .document()
        .strokes()
        .last()
        .expect("the new stroke");
    assert_eq!(newest.len(), 2);
    assert!(
        newest
            .points()
            .iter()
            .all(|point| point.position.x >= 200.0),
        "cancelled points must not leak into the next stroke"
    );
}

// ---------------------------------------------------------------------------
// Step 1 tests: presentation and timing (desktop, #[ignore])
// ---------------------------------------------------------------------------

#[test]
#[ignore = "creates real topmost DirectComposition windows on the desktop"]
fn present_only_submits_dirty_region() {
    let device = GraphicsDevice::new().expect("create graphics device");
    let bounds = PhysicalRect::from_xywh(250, 250, 640, 360);

    // White backdrop so ink visibility is unambiguous on any desktop.
    let mut backdrop = BackdropWindow::new(&device, bounds).expect("backdrop window");
    backdrop
        .fill(Rgba8::new(255, 255, 255, 255))
        .expect("backdrop fill");
    backdrop.show();
    pump_for(Duration::from_millis(200));

    let mut surface = InkSurface::new(&device, bounds).expect("ink surface");
    surface.show();
    pump_for(Duration::from_millis(250));

    // A full static cache with many strokes, the "expensive" case a naive
    // renderer would redraw on every frame.
    let mut controller = InkController::new();
    controller.set_style(PenStyle::fixed(Rgba8::new(20, 20, 20, 255), 12.0));
    for column in 0..30 {
        let x = 260.0 + column as f32 * 20.0;
        controller.begin(pen_point(x, 270.0, column * 4));
        controller.append(&[
            pen_point(x, 340.0, column * 4 + 1),
            pen_point(x + 6.0, 430.0, column * 4 + 2),
            pen_point(x, 560.0, column * 4 + 3),
        ]);
        controller.end().expect("static stroke");
    }
    let full = DirtyRegion::from_rect(DirtyRect::new(250, 250, 890, 610));
    surface
        .redraw_static(controller.document(), &full)
        .expect("rasterize static cache");
    surface.present(full).expect("initial present");

    // One small live segment: only its dirty rect may be rasterized and
    // submitted.
    let small_rect = DirtyRect::new(600, 400, 640, 440);
    let small = DirtyRegion::from_rect(small_rect);
    controller.begin(pen_point(610.0, 420.0, 200));
    controller.append(&[pen_point(620.0, 425.0, 201), pen_point(630.0, 435.0, 202)]);
    let active = controller.active().expect("active stroke");
    surface
        .draw_active(active, &small)
        .expect("rasterize active layer");
    let stats = surface
        .present(small.clone())
        .expect("present dirty region");

    assert_eq!(stats.region, small, "the submitted region is the dirty one");
    assert_eq!(stats.rects, 1);
    assert_eq!(stats.area, small.area());
    assert_eq!(
        stats.painted_area,
        small.area(),
        "rasterization must be clipped to exactly the dirty rect"
    );
    assert_eq!(
        stats.uploaded_bytes, 0,
        "the present path must not upload CPU pixel data"
    );
    let surface_area = bounds.width() as u64 * bounds.height() as u64;
    assert!(
        stats.area * 10 < surface_area,
        "the dirty region must stay a small fraction of the surface ({} of {} px)",
        stats.area,
        surface_area
    );

    // The freshly drawn segment must be visible on screen.
    let expanded = small_rect.expand(2);
    let screen = ScreenRegion::new(PhysicalRect::new(
        expanded.left,
        expanded.top,
        expanded.right,
        expanded.bottom,
    ));
    screen.refresh();
    let dark = screen.count_dark(80);
    assert!(
        dark > 20,
        "the active segment must be visible in the presented frame (dark pixels: {dark})"
    );
}

#[test]
#[ignore = "creates real topmost DirectComposition windows on the desktop"]
fn pointer_timestamp_to_submit_is_monotonic() {
    let device = GraphicsDevice::new().expect("create graphics device");
    let bounds = PhysicalRect::from_xywh(260, 260, 480, 320);
    let mut surface = InkSurface::new(&device, bounds).expect("ink surface");
    surface.show();
    pump_for(Duration::from_millis(150));

    let frequency = qpc_frequency();
    let ticks_per_ms = (frequency / 1000).max(1);
    // Establish the process clock before any sample so the normalizer's
    // first-sample epoch is never ahead of it.
    let clock_start = MonotonicClock::now();

    let mut controller = InkController::new();
    controller.set_style(PenStyle::fixed(Rgba8::new(0, 0, 0, 255), 6.0));
    let mut normalizer = PointerNormalizer::new();
    let mut last_submit = clock_start;
    let mut last_input = Duration::ZERO;
    let mut started = false;

    for frame in 0..12 {
        let now = qpc_now();
        let x = 300.0 + frame as f32 * 12.0;
        let y = 300.0 + (frame as f32 * 9.0) % 40.0;
        // GetPointer*InfoHistory returns newest first; simulate a three-entry
        // coalesced burst.
        let newest_first = [
            PointerSample::new(x as i32 + 2, y as i32 + 2, Some(0.8), now),
            PointerSample::new(x as i32 + 1, y as i32 + 1, Some(0.6), now - ticks_per_ms),
            PointerSample::new(x as i32, y as i32, Some(0.4), now - 2 * ticks_per_ms),
        ];
        let points = normalizer.normalize(&newest_first);
        assert_eq!(points.len(), 3);
        assert!(
            points
                .windows(2)
                .all(|pair| pair[0].timestamp <= pair[1].timestamp),
            "frame {frame}: normalized timestamps must be monotonic"
        );

        let dirty = if !started {
            started = true;
            let mut dirty = controller.begin(points[0]);
            dirty.union(&controller.append(&points[1..]));
            dirty
        } else {
            controller.append(&points)
        };
        let active = controller.active().expect("active stroke");
        surface.draw_active(active, &dirty).expect("draw active");
        surface.present(dirty).expect("present");
        let submit = MonotonicClock::now();

        let newest_input = points[points.len() - 1].timestamp;
        assert!(
            newest_input <= submit,
            "frame {frame}: input timestamp {newest_input:?} must not be later than its submit {submit:?}"
        );
        assert!(
            submit >= last_submit,
            "frame {frame}: submit timestamps must not go backwards"
        );
        assert!(
            newest_input >= last_input || frame == 0,
            "frame {frame}: pointer timestamps must be monotonic to submit"
        );
        last_submit = submit;
        last_input = newest_input;
    }

    let commit = controller.end().expect("stroke commits");
    let stroke = controller
        .document()
        .strokes()
        .last()
        .expect("committed stroke");
    surface
        .draw_committed(stroke, &commit)
        .expect("commit into static cache");
    surface.clear_active(&commit).expect("clear active layer");
    surface.present(commit).expect("present committed stroke");

    let stroke = controller
        .document()
        .strokes()
        .last()
        .expect("committed stroke");
    assert_eq!(stroke.len(), 12 * 3);
    assert!(
        stroke
            .points()
            .windows(2)
            .all(|pair| pair[0].timestamp <= pair[1].timestamp),
        "the committed stroke must keep monotonic timestamps"
    );
}

// ---------------------------------------------------------------------------
// Step 4: real mouse MVP acceptance (desktop, #[ignore])
// ---------------------------------------------------------------------------

#[test]
#[ignore = "moves the real cursor and clicks real toolbar buttons on the desktop"]
fn mvp_end_to_end_mouse_writing_selects_undo_clear_and_exits() {
    let device = GraphicsDevice::new().expect("create graphics device");
    let settings = Settings::defaults();
    let bounds = PhysicalRect::from_xywh(300, 220, 800, 420);
    let original_cursor = cursor_position();

    let mut backdrop = BackdropWindow::new(&device, bounds).expect("backdrop window");
    backdrop
        .fill(Rgba8::new(255, 255, 255, 255))
        .expect("backdrop fill");
    backdrop.show();
    pump_for(Duration::from_millis(150));

    let mut session = InkSession::new(&device, bounds).expect("ink session");
    session.surface_mut().expect("ink surface").show();
    session.set_pen_style(pen_style(&settings, 1));
    pump_for(Duration::from_millis(200));

    // --- Write a red stroke with real routed mouse input. -----------------
    move_cursor(PhysicalPoint::new(360, 300));
    pump_for(Duration::from_millis(40));
    mouse_button(true);
    pump_for(Duration::from_millis(20));
    for step in 1..=20 {
        move_cursor(PhysicalPoint::new(360 + step * 24, 300 + step * 8));
        pump_for(Duration::from_millis(8));
    }
    mouse_button(false);
    pump_for(Duration::from_millis(120));

    assert_eq!(
        session.controller().document().len(),
        1,
        "real mouse input must commit exactly one stroke"
    );
    let first = session
        .controller()
        .document()
        .strokes()
        .last()
        .expect("first stroke");
    assert!(
        first.len() >= 10,
        "the routed stroke must contain the travelled samples (got {})",
        first.len()
    );
    assert_eq!(first.style().color, Rgba8::new(225, 60, 60, 255));

    let screen = ScreenRegion::new(bounds);
    screen.refresh();
    assert!(
        screen.count_matching(PhysicalRect::new(330, 270, 830, 500), |[r, g, b]| {
            r > 190 && g < 110 && b < 110
        }) > 100,
        "the red stroke must be visible on screen"
    );

    // --- Select the blue Pen 2 through a real toolbar click. --------------
    let mut toolbar = ToolbarWindow::new(&device, &settings).expect("toolbar window");
    toolbar.enter().expect("show toolbar");
    pump_for(Duration::from_millis(450));
    assert!(toolbar.is_visible());

    click_button(&mut toolbar, ButtonId::Pen(2));
    assert_eq!(toolbar.take_clicked(), Some(ButtonId::Pen(2)));
    session.set_pen_style(pen_style(&settings, 2));
    let mut model: ToolbarModel = toolbar.model().clone();
    model.set_active(Some(ButtonId::Pen(2)));
    toolbar.set_model(model).expect("refresh toolbar model");
    pump_for(Duration::from_millis(60));

    // --- Write a blue stroke with the selected pen. -----------------------
    move_cursor(PhysicalPoint::new(400, 480));
    pump_for(Duration::from_millis(40));
    mouse_button(true);
    for step in 1..=16 {
        move_cursor(PhysicalPoint::new(400 + step * 20, 480));
        pump_for(Duration::from_millis(8));
    }
    mouse_button(false);
    pump_for(Duration::from_millis(120));
    assert_eq!(session.controller().document().len(), 2);
    let second = session
        .controller()
        .document()
        .strokes()
        .last()
        .expect("second stroke");
    assert_eq!(second.style().color, Rgba8::new(30, 110, 200, 255));
    screen.refresh();
    assert!(
        screen.count_matching(PhysicalRect::new(390, 460, 760, 500), |[r, _, b]| {
            b > 150 && r < 100
        }) > 80,
        "the blue stroke must be visible on screen"
    );

    // --- Undo through a real toolbar click. -------------------------------
    click_button(&mut toolbar, ButtonId::Undo);
    assert_eq!(toolbar.take_clicked(), Some(ButtonId::Undo));
    session.undo().expect("undo");
    pump_for(Duration::from_millis(120));
    assert_eq!(session.controller().document().len(), 1);
    screen.refresh();
    assert_eq!(
        screen.count_matching(PhysicalRect::new(390, 460, 760, 500), |[r, _, b]| {
            b > 150 && r < 100
        }),
        0,
        "undo must remove the blue stroke from the presented surface"
    );

    // --- Clear through a real toolbar click, then verify the canvas. ------
    click_button(&mut toolbar, ButtonId::Clear);
    assert_eq!(toolbar.take_clicked(), Some(ButtonId::Clear));
    session.clear().expect("clear");
    pump_for(Duration::from_millis(120));
    assert_eq!(session.controller().document().len(), 0);
    screen.refresh();
    assert_eq!(
        screen.count_matching(PhysicalRect::new(330, 270, 830, 500), |[r, g, b]| {
            (r > 190 && g < 110 && b < 110) || (b > 150 && r < 100)
        }),
        0,
        "clear must remove every stroke from the presented surface"
    );

    // --- Exit cleanly (back to tray state, then drop). --------------------
    session.close_annotation().expect("close annotation");
    session.surface_mut().expect("ink surface").hide();
    toolbar.fast_exit().expect("hide toolbar");
    toolbar.settle_now().expect("settle exit");
    assert_eq!(session.controller().document().len(), 0);
    drop(toolbar);
    drop(session);
    drop(backdrop);
    move_cursor(original_cursor);
    pump_for(Duration::from_millis(80));
}

#[test]
#[ignore = "launches the real plnk.exe and draws on the desktop with real input"]
fn mvp_real_app_launch_draws_and_exits_cleanly() {
    // White backdrop under the app's overlay so any non-white pixel inside the
    // region is unambiguous ink, independent of the user's imported pen colors.
    let device = GraphicsDevice::new().expect("create graphics device");
    let backdrop_bounds = PhysicalRect::from_xywh(250, 300, 700, 320);
    let mut backdrop = BackdropWindow::new(&device, backdrop_bounds).expect("backdrop window");
    backdrop
        .fill(Rgba8::new(255, 255, 255, 255))
        .expect("backdrop fill");
    backdrop.show();
    pump_for(Duration::from_millis(150));

    let exe = env!("CARGO_BIN_EXE_plnk");
    let mut child = Command::new(exe)
        .args(["--no-hotkeys", "--draw", "--exit-after-ms", "30000"])
        .spawn()
        .expect("spawn plnk.exe");
    let overlay = wait_for_window(
        w!("plnk_gate_window"),
        w!("plnk overlay"),
        Duration::from_secs(10),
    );
    assert!(
        overlay.is_some(),
        "the real app must create the drawing overlay when launched with --draw"
    );
    pump_for(Duration::from_millis(600));

    let original_cursor = cursor_position();
    move_cursor(PhysicalPoint::new(320, 400));
    pump_for(Duration::from_millis(60));
    mouse_button(true);
    for step in 1..=20 {
        move_cursor(PhysicalPoint::new(320 + step * 28, 400 + (step % 6) * 7));
        pump_for(Duration::from_millis(10));
    }
    mouse_button(false);
    pump_for(Duration::from_millis(350));

    let screen = ScreenRegion::new(PhysicalRect::new(270, 330, 950, 560));
    screen.refresh();
    let ink_pixels = screen.count_matching(PhysicalRect::new(280, 320, 950, 580), |[r, g, b]| {
        r < 250 || g < 250 || b < 250
    });
    assert!(
        ink_pixels > 200,
        "the real app must paint a visible stroke under routed input (non-white pixels: {ink_pixels})"
    );

    // Exit through the same WM_CLOSE branch the tray menu uses.
    let shell = wait_for_window(
        w!("plnk_app_shell_window"),
        w!("plnk"),
        Duration::from_secs(5),
    );
    let shell = shell.expect("the real app's shell window");
    unsafe { PostMessageW(Some(shell), WM_CLOSE, WPARAM(0), LPARAM(0)) }
        .expect("post WM_CLOSE to the real app");
    let status = wait_for_exit(&mut child, Duration::from_secs(15));
    assert!(
        status.success(),
        "the real app must exit with a success code, got {status:?}"
    );
    let log_path = std::path::Path::new(exe)
        .parent()
        .expect("exe directory")
        .join("data")
        .join("plnk.log");
    if let Ok(log) = std::fs::read_to_string(&log_path) {
        assert!(
            log.contains("shutdown complete"),
            "a clean shutdown must be logged to {}:\n{log}",
            log_path.display()
        );
        assert!(
            log.contains("mode Tray -> Drawing"),
            "the --draw switch must enter drawing mode:\n{log}"
        );
    }

    drop(backdrop);
    move_cursor(original_cursor);
    pump_for(Duration::from_millis(80));
}

fn wait_for_window(class: PCWSTR, title: PCWSTR, timeout: Duration) -> Option<HWND> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(hwnd) = unsafe { FindWindowW(class, title) }
            && !hwnd.is_invalid()
        {
            return Some(hwnd);
        }
        pump_for(Duration::from_millis(50));
    }
    None
}

fn wait_for_exit(child: &mut std::process::Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                panic!("the real app did not exit within {timeout:?}");
            }
            Err(error) => panic!("could not poll the real app: {error}"),
        }
    }
}

#[test]
#[ignore = "creates real topmost windows and reads back screen pixels"]
fn renderer_caps_dots_and_highlighter_blend_are_visible() {
    let device = GraphicsDevice::new().expect("create graphics device");
    let bounds = PhysicalRect::from_xywh(400, 400, 460, 220);
    let mut backdrop = BackdropWindow::new(&device, bounds).expect("backdrop window");
    backdrop
        .fill(Rgba8::new(255, 255, 255, 255))
        .expect("backdrop fill");
    backdrop.show();
    pump_for(Duration::from_millis(150));

    let mut surface = InkSurface::new(&device, bounds).expect("ink surface");
    surface.show();
    pump_for(Duration::from_millis(200));

    let mut controller = InkController::new();
    controller.set_style(PenStyle::fixed(Rgba8::new(10, 10, 10, 255), 40.0));

    // A two-point stroke with round caps: 470 -> 570 at y=500 (screen pixels).
    controller.begin(pen_point(470.0, 500.0, 0));
    controller.append(&[pen_point(570.0, 500.0, 1)]);
    let dirty = controller.end().expect("line commits");
    {
        let stroke = controller
            .document()
            .strokes()
            .last()
            .expect("committed line");
        surface.draw_committed(stroke, &dirty).expect("draw line");
    }
    surface.present(dirty).expect("present line");

    // A single-sample stroke (short dot) at (700, 500), radius 20.
    controller.begin(pen_point(700.0, 500.0, 2));
    let dirty = controller.end().expect("dot commits");
    {
        let stroke = controller
            .document()
            .strokes()
            .last()
            .expect("committed dot");
        surface.draw_committed(stroke, &dirty).expect("draw dot");
    }
    surface.present(dirty).expect("present dot");
    pump_for(Duration::from_millis(200));

    let screen = ScreenRegion::new(bounds);
    screen.refresh();
    let pixel = |x: i32, y: i32| screen.pixel_rgb(x - bounds.left, y - bounds.top);
    let dark = |pixel: [u8; 3]| pixel.iter().all(|channel| *channel < 80);
    // Round caps extend one half-width (20 px) past the endpoints: a point
    // 16 px behind the start endpoint is inside the cap, 24 px is outside.
    assert!(dark(pixel(454, 500)), "the start cap must be round");
    assert!(
        !dark(pixel(446, 500)),
        "the stroke must not extend past the cap"
    );
    assert!(dark(pixel(586, 500)), "the end cap must be round");
    assert!(
        !dark(pixel(594, 500)),
        "the stroke must not extend past the cap"
    );
    // The single sample is a round dot, not an empty or square mark.
    assert!(dark(pixel(700, 500)), "a short tap must leave a dot");
    assert!(
        dark(pixel(682, 500)),
        "the dot is round with the pen radius"
    );
    assert!(
        !dark(pixel(674, 500)),
        "the dot must not exceed the pen radius"
    );

    // Translucent highlighter: two crossing strokes, drawn on top of the
    // static cache one commit at a time.
    controller.set_style(PenStyle::new(Rgba8::new(235, 125, 15, 80), 120.0, false));
    controller.begin(pen_point(620.0, 560.0, 3));
    controller.append(&[pen_point(820.0, 560.0, 4)]);
    let dirty = controller.end().expect("highlighter commits");
    {
        let stroke = controller
            .document()
            .strokes()
            .last()
            .expect("committed highlighter");
        surface
            .draw_committed(stroke, &dirty)
            .expect("draw highlighter");
    }
    surface.present(dirty).expect("present highlighter");
    controller.begin(pen_point(720.0, 460.0, 5));
    controller.append(&[pen_point(720.0, 640.0, 6)]);
    let dirty = controller.end().expect("crossing highlighter commits");
    {
        let stroke = controller
            .document()
            .strokes()
            .last()
            .expect("committed crossing highlighter");
        surface
            .draw_committed(stroke, &dirty)
            .expect("draw crossing highlighter");
    }
    surface
        .present(dirty)
        .expect("present crossing highlighter");
    pump_for(Duration::from_millis(200));
    screen.refresh();

    let single = pixel(640, 560);
    let overlap = pixel(720, 560);
    assert!(
        single[0] > 200 && single[1] < 250,
        "one highlighter pass must tint the white backdrop orange: {single:?}"
    );
    assert!(
        overlap[1] + 10 < single[1],
        "the translucent overlap must be darker than a single pass (single {single:?}, overlap {overlap:?})"
    );
}

#[test]
#[ignore = "creates a real full-monitor overlay and clicks at the screen edge"]
fn overlay_covers_true_physical_monitor_and_accepts_input_at_edges() {
    // REGRESSION (fix round 1): `current_monitor_rect` must establish DPI
    // awareness before it queries the monitor. In an unaware process a scaled
    // display reports its virtualized logical size (1707x1067 instead of
    // 2560x1600 here), so the "full-screen" overlay missed the right/bottom
    // part of the screen. DEVMODE's physical pixel size is the independent
    // oracle. Run this test filtered in a fresh process to exercise start-up.
    let physical = current_monitor_rect().expect("physical monitor rect");
    let (panel_width, panel_height) = display_settings_pels();
    assert_eq!(
        (physical.width(), physical.height()),
        (panel_width, panel_height),
        "the monitor rect must equal the panel's true physical pixels (DPI awareness regression)"
    );

    let device = GraphicsDevice::new().expect("create graphics device");
    // White backdrop near the physical bottom-right corner, created before the
    // overlay so the overlay sits above it.
    let corner = PhysicalRect::from_xywh(physical.right - 440, physical.bottom - 340, 420, 300);
    let mut backdrop = BackdropWindow::new(&device, corner).expect("corner backdrop window");
    backdrop
        .fill(Rgba8::new(255, 255, 255, 255))
        .expect("backdrop fill");
    backdrop.show();
    pump_for(Duration::from_millis(150));

    let mut session = InkSession::new(&device, physical).expect("full-monitor ink session");
    session.surface_mut().expect("ink surface").show();
    pump_for(Duration::from_millis(250));
    assert_eq!(
        session
            .surface()
            .expect("ink surface")
            .window_rect()
            .expect("overlay window rect"),
        physical,
        "the overlay window must cover the true physical monitor rect"
    );

    session.set_pen_style(PenStyle::fixed(Rgba8::new(10, 10, 10, 255), 24.0));
    let original_cursor = cursor_position();
    let start = PhysicalPoint::new(physical.right - 380, physical.bottom - 260);
    move_cursor(start);
    pump_for(Duration::from_millis(50));
    mouse_button(true);
    for step in 1..=8 {
        move_cursor(PhysicalPoint::new(start.x + step * 32, start.y + step * 10));
        pump_for(Duration::from_millis(8));
    }
    mouse_button(false);
    pump_for(Duration::from_millis(200));

    assert_eq!(
        session.controller().document().len(),
        1,
        "routed real input at the physical right/bottom edge must reach the overlay"
    );
    let stroke = session
        .controller()
        .document()
        .strokes()
        .last()
        .expect("edge stroke");
    assert!(
        stroke
            .points()
            .iter()
            .any(|point| point.position.x > (physical.right - 160) as f32),
        "the routed stroke must reach the right edge region"
    );
    assert!(
        stroke
            .points()
            .iter()
            .any(|point| point.position.y > (physical.bottom - 220) as f32),
        "the routed stroke must reach the bottom edge region"
    );

    let screen = ScreenRegion::new(corner);
    screen.refresh();
    let ink_pixels = screen.count_matching(corner, |[r, g, b]| r < 200 && g < 200 && b < 200);
    assert!(
        ink_pixels > 100,
        "ink must render at the physical right/bottom edge (non-white pixels: {ink_pixels})"
    );

    session.surface_mut().expect("ink surface").hide();
    drop(session);
    drop(backdrop);
    move_cursor(original_cursor);
    pump_for(Duration::from_millis(80));
}

// ---------------------------------------------------------------------------
// Step 4: performance probe (Release, #[ignore])
// ---------------------------------------------------------------------------

#[test]
#[ignore = "release probe: full-screen surface, 1000 strokes, latency sampling"]
fn ink_mvp_probe() {
    if cfg!(debug_assertions) {
        // Spec §4.2/§8: performance probes are Release-only. The debug build
        // would measure unoptimized code and report numbers the spec explicitly
        // does not accept.
        println!("[ink-probe] skipped in a debug build; run with --release");
        return;
    }
    let baseline_memory = memory_snapshot();
    let device = GraphicsDevice::new().expect("create graphics device");
    let device_memory = memory_snapshot();
    // `current_monitor_rect` establishes per-monitor-v2 awareness itself; the
    // panel's physical size from DEVMODE is the independent oracle for the
    // regression that the overlay must cover the true physical screen.
    let bounds = current_monitor_rect().expect("physical monitor rect");
    let (panel_width, panel_height) = display_settings_pels();
    assert_eq!(
        (bounds.width(), bounds.height()),
        (panel_width, panel_height),
        "the probe surface must be the true physical monitor rect, not a DPI-virtualized one"
    );
    let mut surface = InkSurface::new(&device, bounds).expect("monitor-sized ink surface");
    surface.show();
    pump_for(Duration::from_millis(250));
    let surface_memory = memory_snapshot();
    println!(
        "[ink-probe] memory.baseline_private_bytes={} device_private_bytes={} surface_private_bytes={} surface={}x{} display_hz={:.1}",
        baseline_memory.private_bytes,
        device_memory.private_bytes,
        surface_memory.private_bytes,
        bounds.width(),
        bounds.height(),
        display_refresh_hz()
    );

    let mut controller = InkController::new();
    let pen_styles = [
        PenStyle::fixed(Rgba8::new(225, 60, 60, 255), 80.0),
        PenStyle::fixed(Rgba8::new(30, 110, 200, 255), 30.0),
        PenStyle::new(Rgba8::new(235, 125, 15, 80), 500.0, false),
        PenStyle::pressure(Rgba8::new(120, 175, 70, 255), 80.0),
    ];
    let stroke_count = 1000usize;
    let points_per_stroke = 200usize;
    let mut random = 0x2f6e_2b1d_4c5a_9e01u64;
    let mut dirty_total = DirtyRegion::new();
    let (mut append_ns, mut static_ns, mut present_ns) = (0u128, 0u128, 0u128);
    let (mut intersections, mut max_intersections) = (0u64, 0usize);
    let mut memory_stages = Vec::new();
    let start = Instant::now();
    for stroke_index in 0..stroke_count {
        controller.set_style(pen_styles[stroke_index % pen_styles.len()]);
        // Scatter mixed strokes across the monitor with a margin that keeps
        // even the 500px highlighter inside the surface.
        let start_x = 320.0 + (next_random(&mut random) % 1100) as f32;
        let start_y = 320.0 + (next_random(&mut random) % 500) as f32;
        let mut x = start_x;
        let mut y = start_y;
        let mut timestamp = Duration::from_millis(stroke_index as u64);
        let pressure = (stroke_index % 4 == 3).then_some(0.5);
        let append_start = Instant::now();
        controller.begin(PenPoint::new(
            PhysicalPointF::new(x, y),
            pressure.unwrap_or(1.0),
            timestamp,
        ));
        let mut points = Vec::with_capacity(points_per_stroke - 1);
        for _ in 1..points_per_stroke {
            x += ((next_random(&mut random) % 9) as i32 - 4) as f32;
            y += ((next_random(&mut random) % 9) as i32 - 4) as f32;
            timestamp += Duration::from_micros(4);
            points.push(PenPoint::new(
                PhysicalPointF::new(x, y),
                pressure.unwrap_or(1.0),
                timestamp,
            ));
        }
        controller.append(&points);
        let dirty = controller.end().expect("probe stroke commits");
        append_ns += append_start.elapsed().as_nanos();
        let rect = dirty.bounds();
        let hits = controller
            .document()
            .strokes()
            .filter(|stroke| stroke.bounds().intersects(rect))
            .count();
        intersections += hits as u64;
        max_intersections = max_intersections.max(hits);
        let static_start = Instant::now();
        {
            let stroke = controller
                .document()
                .strokes()
                .last()
                .expect("probe committed stroke");
            surface
                .draw_committed(stroke, &dirty)
                .expect("probe commit draw");
        }
        static_ns += static_start.elapsed().as_nanos();
        let present_start = Instant::now();
        surface.present(dirty.clone()).expect("probe present");
        present_ns += present_start.elapsed().as_nanos();
        dirty_total.union(&dirty);
        if (stroke_index + 1) % 250 == 0 {
            memory_stages.push((stroke_index + 1, memory_snapshot()));
        }
    }
    let stroke_wall = start.elapsed();
    pump_for(Duration::from_millis(300));
    let memory = memory_snapshot();
    let document_points: usize = controller.document().strokes().map(|s| s.len()).sum();
    println!(
        "[ink-probe] strokes={stroke_count} points={document_points} write_wall_ms={:.1} append_ms={:.1} commit_ms={:.1} present_ms={:.1} strokes_in_dirty_avg={:.1} max={max_intersections}",
        stroke_wall.as_secs_f64() * 1000.0,
        append_ns as f64 / 1_000_000.0,
        static_ns as f64 / 1_000_000.0,
        present_ns as f64 / 1_000_000.0,
        intersections as f64 / stroke_count as f64
    );
    for (count, snapshot) in &memory_stages {
        println!(
            "[ink-probe] memory.after_{count}_strokes.private_bytes={}",
            snapshot.private_bytes
        );
    }
    println!(
        "[ink-probe] memory.private_bytes={} working_set_bytes={} raw_point_bytes={} growth_since_surface_bytes={}",
        memory.private_bytes,
        memory.working_set,
        document_points * std::mem::size_of::<PenPoint>(),
        memory.private_bytes as i64 - surface_memory.private_bytes as i64
    );

    // Pointer-to-submit latency: normalize three coalesced samples, append,
    // rasterize the active segment and submit, per simulated pointer message.
    // The measured interval is wall time around the complete application-side
    // path (it includes any Present wait but excludes digitizer hardware).
    // GetThreadTimes is not used: its ~15.6ms scheduler granularity cannot
    // resolve a sub-millisecond pipeline.
    let ticks_per_ms = (qpc_frequency() / 1000).max(1);
    let mut normalizer = PointerNormalizer::new();
    let mut wall_samples = Vec::new();
    let messages = 2000usize;
    let mut x = 400.0f32;
    let mut y = 700.0f32;
    let mut active = false;
    let mut stroke_messages = 0usize;
    for _ in 0..messages {
        let now = qpc_now();
        let burst = [
            PointerSample::new(x as i32 + 2, y as i32 + 1, Some(0.7), now),
            PointerSample::new(x as i32 + 1, y as i32, Some(0.5), now - ticks_per_ms),
            PointerSample::new(x as i32, y as i32, Some(0.4), now - 2 * ticks_per_ms),
        ];
        let wall_start = Instant::now();
        let points = normalizer.normalize(&burst);
        let dirty = if !active {
            let mut dirty = controller.begin(points[0]);
            dirty.union(&controller.append(&points[1..]));
            active = true;
            stroke_messages = 0;
            dirty
        } else {
            controller.append(&points)
        };
        let active_stroke = controller.active().expect("probe active stroke");
        surface
            .draw_active(active_stroke, &dirty)
            .expect("probe draw");
        surface.present(dirty).expect("probe submit");
        wall_samples.push(wall_start.elapsed().as_secs_f64() * 1000.0);
        x += 0.7;
        y += 0.2;
        stroke_messages += 1;
        if stroke_messages >= 240 {
            let commit = controller.end().expect("probe commit");
            {
                let stroke = controller
                    .document()
                    .strokes()
                    .last()
                    .expect("probe committed stroke");
                surface
                    .draw_committed(stroke, &commit)
                    .expect("probe commit draw");
            }
            surface.clear_active(&commit).expect("probe commit clear");
            surface.present(commit).expect("probe commit present");
            active = false;
        }
    }
    let wall_median = percentile(&wall_samples, 0.50);
    let wall_p95 = percentile(&wall_samples, 0.95);
    println!(
        "[ink-probe] latency.pointer_to_submit_ms median={wall_median:.3} p95={wall_p95:.3} max={:.3} samples={messages}",
        wall_samples.iter().fold(0.0f64, |a, &b| a.max(b))
    );
    println!(
        "[ink-probe] document.strokes={} dirty_rects_total={}",
        controller.document().len(),
        dirty_total.rects().len()
    );
    assert!(
        wall_median <= 2.0,
        "pointer-to-submit median target missed: {wall_median:.3}ms"
    );
    assert!(
        wall_p95 <= 4.0,
        "pointer-to-submit p95 target missed: {wall_p95:.3}ms"
    );
    let private_mb = memory.private_bytes as f64 / (1024.0 * 1024.0);
    println!("[ink-probe] memory.private_mb={private_mb:.1}");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pen_point(x: f32, y: f32, tick: u64) -> PenPoint {
    PenPoint::new(PhysicalPointF::new(x, y), 1.0, Duration::from_millis(tick))
}

fn pen_style(settings: &Settings, index: usize) -> PenStyle {
    let pen = &settings.pens[index];
    PenStyle::fixed(
        Rgba8::new(pen.color.red, pen.color.green, pen.color.blue, pen.alpha),
        f32::from(pen.width),
    )
}

fn cursor_position() -> PhysicalPoint {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.expect("GetCursorPos");
    PhysicalPoint::new(point.x, point.y)
}

fn move_cursor(point: PhysicalPoint) {
    unsafe { SetCursorPos(point.x, point.y) }.expect("SetCursorPos");
}

fn mouse_button(down: bool) {
    let flag = if down {
        MOUSEEVENTF_LEFTDOWN
    } else {
        MOUSEEVENTF_LEFTUP
    };
    unsafe { mouse_event(flag, 0, 0, 0, 0) };
}

fn click_button(toolbar: &mut ToolbarWindow, button: ButtonId) {
    let rect = toolbar
        .button_screen_rect(button)
        .unwrap_or_else(|| panic!("{button:?} must have a screen rect"));
    move_cursor(rect.center());
    pump_for(Duration::from_millis(40));
    mouse_button(true);
    pump_for(Duration::from_millis(30));
    mouse_button(false);
    pump_for(Duration::from_millis(80));
    let _ = toolbar.take_error();
}

fn qpc_now() -> i64 {
    let mut counter = 0i64;
    unsafe { QueryPerformanceCounter(&mut counter) }.expect("QueryPerformanceCounter");
    counter
}

fn qpc_frequency() -> i64 {
    let mut frequency = 0i64;
    unsafe { QueryPerformanceFrequency(&mut frequency) }.expect("QueryPerformanceFrequency");
    frequency.max(1)
}

fn next_random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    assert!(!values.is_empty(), "percentile of an empty sample");
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite sample"));
    let rank = (sorted.len() as f64 * fraction).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn pump_pending() {
    let mut message = MSG::default();
    loop {
        let available = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
        if !available.as_bool() {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        pump_pending();
        std::thread::sleep(Duration::from_millis(1));
    }
    pump_pending();
}

fn display_refresh_hz() -> f64 {
    let mut devmode = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let ok = unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut devmode) };
    if ok.as_bool() {
        f64::from(devmode.dmDisplayFrequency)
    } else {
        0.0
    }
}

/// The panel's physical pixel size from the display driver. DEVMODE is not
/// DPI-virtualized, so it is the independent oracle for the true monitor rect.
fn display_settings_pels() -> (i32, i32) {
    let mut devmode = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let ok = unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut devmode) };
    assert!(ok.as_bool(), "EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)");
    (devmode.dmPelsWidth as i32, devmode.dmPelsHeight as i32)
}

#[derive(Clone, Copy)]
struct MemorySnapshot {
    private_bytes: usize,
    working_set: usize,
}

fn memory_snapshot() -> MemorySnapshot {
    let process = unsafe { GetCurrentProcess() };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    unsafe {
        GetProcessMemoryInfo(
            process,
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
        .expect("GetProcessMemoryInfo");
    }
    MemorySnapshot {
        private_bytes: counters.PrivateUsage,
        working_set: counters.WorkingSetSize,
    }
}

/// A screen readback used to prove that presented ink is actually visible.
struct ScreenRegion {
    bounds: PhysicalRect,
    screen: windows::Win32::Graphics::Gdi::HDC,
    memory: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    previous: windows::Win32::Graphics::Gdi::HGDIOBJ,
    pixels: *mut u32,
}

impl ScreenRegion {
    fn new(bounds: PhysicalRect) -> Self {
        assert!(bounds.width() > 0 && bounds.height() > 0);
        unsafe {
            let screen = GetDC(None);
            assert!(!screen.is_invalid(), "GetDC(NULL) failed");
            let memory = CreateCompatibleDC(Some(screen));
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: bounds.width(),
                    biHeight: -bounds.height(),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixel_pointer: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                Some(memory),
                &info,
                DIB_RGB_COLORS,
                &mut pixel_pointer,
                None,
                0,
            )
            .expect("CreateDIBSection");
            let previous = SelectObject(memory, windows::Win32::Graphics::Gdi::HGDIOBJ(bitmap.0));
            Self {
                bounds,
                screen,
                memory,
                bitmap,
                previous,
                pixels: pixel_pointer.cast(),
            }
        }
    }

    fn refresh(&self) {
        unsafe {
            BitBlt(
                self.memory,
                0,
                0,
                self.bounds.width(),
                self.bounds.height(),
                Some(self.screen),
                self.bounds.left,
                self.bounds.top,
                SRCCOPY | CAPTUREBLT,
            )
            .expect("BitBlt from the screen");
        }
    }

    fn pixel_rgb(&self, x: i32, y: i32) -> [u8; 3] {
        assert!(x >= 0 && y >= 0 && x < self.bounds.width() && y < self.bounds.height());
        let value = unsafe { *self.pixels.add((y * self.bounds.width() + x) as usize) };
        [(value >> 16) as u8, (value >> 8) as u8, value as u8]
    }

    fn for_each_in(&self, region: PhysicalRect, mut visit: impl FnMut([u8; 3])) {
        let left = (region.left - self.bounds.left).max(0);
        let top = (region.top - self.bounds.top).max(0);
        let right = (region.right - self.bounds.left).min(self.bounds.width());
        let bottom = (region.bottom - self.bounds.top).min(self.bounds.height());
        let mut y = top;
        while y < bottom {
            let mut x = left;
            while x < right {
                visit(self.pixel_rgb(x, y));
                x += 2;
            }
            y += 2;
        }
    }

    fn count_dark(&self, threshold: u8) -> usize {
        let mut count = 0;
        self.for_each_in(self.bounds, |[r, g, b]| {
            if r < threshold && g < threshold && b < threshold {
                count += 1;
            }
        });
        count
    }

    fn count_matching(&self, region: PhysicalRect, predicate: impl Fn([u8; 3]) -> bool) -> usize {
        let mut count = 0;
        self.for_each_in(region, |pixel| {
            if predicate(pixel) {
                count += 1;
            }
        });
        count
    }
}

impl Drop for ScreenRegion {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.memory);
            let _ = ReleaseDC(None, self.screen);
        }
    }
}

#[test]
#[ignore = "launches the real app and clicks toolbar buttons with routed OS input"]
fn real_app_toolbar_buttons_work_after_drawing() {
    let exe = env!("CARGO_BIN_EXE_plnk");
    let log_path = app_log_path(exe);
    let log_start = std::fs::metadata(&log_path).map_or(0, |meta| meta.len());
    let mut child = Command::new(exe)
        .args(["--no-hotkeys", "--draw", "--exit-after-ms", "30000"])
        .spawn()
        .expect("spawn plnk.exe");
    let overlay = wait_for_window(
        w!("plnk_gate_window"),
        w!("plnk overlay"),
        Duration::from_secs(10),
    )
    .expect("overlay");
    let toolbar = wait_for_window(
        w!("plnk_gate_window"),
        w!("plnk toolbar"),
        Duration::from_secs(10),
    )
    .expect("toolbar");
    pump_for(Duration::from_millis(600));

    // Draw first: clicking the overlay must not make its topmost band cover the
    // toolbar (the user-facing failure this regression guards).
    move_cursor(PhysicalPoint::new(320, 400));
    pump_for(Duration::from_millis(50));
    mouse_button(true);
    for step in 1..=20 {
        move_cursor(PhysicalPoint::new(320 + step * 20, 400 + step * 3));
        pump_for(Duration::from_millis(8));
    }
    mouse_button(false);
    pump_for(Duration::from_millis(300));

    for (button, evidence) in [
        (ButtonId::Pen(2), "toolbar click: Pen(2)"),
        (ButtonId::Undo, "toolbar click: Undo"),
        (ButtonId::Clear, "toolbar click: Clear"),
        (ButtonId::Pointer, "toolbar click: Pointer"),
        (ButtonId::Exit, "leaving annotation: toolbar exit button"),
    ] {
        let point = real_toolbar_button_center(toolbar, button);
        let under = unsafe { windows::Win32::UI::WindowsAndMessaging::WindowFromPoint(point) };
        assert_eq!(
            under, toolbar,
            "{button:?} at ({}, {}) must be routed to the toolbar, not the overlay {overlay:?}",
            point.x, point.y
        );
        move_cursor(PhysicalPoint::new(point.x, point.y));
        pump_for(Duration::from_millis(50));
        mouse_button(true);
        pump_for(Duration::from_millis(30));
        mouse_button(false);
        assert!(
            wait_for_log_contains(&log_path, log_start, evidence, Duration::from_secs(3)),
            "real click on {button:?} must produce {evidence:?}; log:\n{}",
            app_log_since(&log_path, log_start)
        );
        pump_for(Duration::from_millis(100));
    }

    let shell = wait_for_window(
        w!("plnk_app_shell_window"),
        w!("plnk"),
        Duration::from_secs(5),
    )
    .expect("shell");
    unsafe { PostMessageW(Some(shell), WM_CLOSE, WPARAM(0), LPARAM(0)) }.expect("post WM_CLOSE");
    assert!(wait_for_exit(&mut child, Duration::from_secs(10)).success());
}

#[test]
#[ignore = "launches the real app and sends a real Esc key"]
fn real_app_escape_exits_annotation() {
    let exe = env!("CARGO_BIN_EXE_plnk");
    let log_path = app_log_path(exe);
    let log_start = std::fs::metadata(&log_path).map_or(0, |meta| meta.len());
    let mut child = Command::new(exe)
        .args(["--no-hotkeys", "--draw", "--exit-after-ms", "20000"])
        .spawn()
        .expect("spawn plnk.exe");
    wait_for_window(
        w!("plnk_gate_window"),
        w!("plnk overlay"),
        Duration::from_secs(10),
    )
    .expect("overlay");
    pump_for(Duration::from_millis(500));

    unsafe {
        windows::Win32::UI::Input::KeyboardAndMouse::keybd_event(
            windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 as u8,
            0,
            windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0),
            0,
        );
        windows::Win32::UI::Input::KeyboardAndMouse::keybd_event(
            windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 as u8,
            0,
            windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP,
            0,
        );
    }
    assert!(
        wait_for_log_contains(
            &log_path,
            log_start,
            "leaving annotation: Escape",
            Duration::from_secs(3)
        ),
        "a real Esc key must exit annotation; log:\n{}",
        app_log_since(&log_path, log_start)
    );

    let shell = wait_for_window(
        w!("plnk_app_shell_window"),
        w!("plnk"),
        Duration::from_secs(5),
    )
    .expect("shell");
    unsafe { PostMessageW(Some(shell), WM_CLOSE, WPARAM(0), LPARAM(0)) }.expect("post WM_CLOSE");
    assert!(wait_for_exit(&mut child, Duration::from_secs(10)).success());
}

fn app_log_path(exe: &str) -> std::path::PathBuf {
    std::path::Path::new(exe)
        .parent()
        .expect("exe directory")
        .join("data")
        .join("plnk.log")
}

fn app_log_since(path: &std::path::Path, start: u64) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = (start as usize).min(bytes.len());
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

fn wait_for_log_contains(
    path: &std::path::Path,
    start: u64,
    needle: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if app_log_since(path, start).contains(needle) {
            return true;
        }
        pump_for(Duration::from_millis(50));
    }
    false
}

fn real_toolbar_button_center(toolbar: HWND, button: ButtonId) -> POINT {
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(toolbar, &mut rect) }
        .expect("toolbar rect");
    let dpi = plnk::platform::dpi::DpiMap::from_window(toolbar);
    let model = ToolbarModel::from_settings(&Settings::defaults());
    let layout = plnk::ui::toolbar::ToolbarLayout::new(
        &model,
        (rect.bottom - rect.top) as f32 / dpi.scale(),
        false,
    );
    let button = layout
        .button_rect(button)
        .expect("button in the real toolbar layout");
    POINT {
        x: rect.left + ((button.left + button.right) * 0.5 * dpi.scale()) as i32,
        y: rect.top + ((button.top + button.bottom) * 0.5 * dpi.scale()) as i32,
    }
}
