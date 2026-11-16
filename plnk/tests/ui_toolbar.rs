//! Desktop acceptance for the composited native toolbar (plan Task 5, Step 4).
//!
//! The `#[ignore]` tests create real, short-lived topmost windows on the
//! desktop: a black backdrop plus the toolbar itself. They never send
//! synthetic input, never register global hotkeys and never touch the running
//! gInk instance. Every window is destroyed before the test returns.
//!
//! What is measured, and what is not:
//! - `presentation frames`: `IDCompositionDevice::GetFrameStatistics`
//!   (`lastFrameTime`, QPC ticks) sampled in a tight loop while a
//!   DirectComposition animation runs. Callback frequency is never used as a
//!   frame rate.
//! - `app callbacks`: real WM_TIMER / WM_PAINT / WM_APP+1 messages delivered to
//!   the toolbar window procedure. During a slide only the single animation-end
//!   settle timer may arrive; there is no per-frame timer or paint loop.
//! - `visible movement`: screen pixels sampled with BitBlt from the composited
//!   desktop while the toolbar slides across the black backdrop.
//! - `visual fidelity`: panel background, pen underline colours and dark icon
//!   pixels are sampled and compared with the legacy FormCollection palette.
//!
//! Run with:
//! `cargo test --manifest-path plnk/Cargo.toml --test ui_toolbar -- --ignored --test-threads=1`

#[path = "support/mod.rs"]
mod support;

/// Yields the desktop or returns early with a SKIP marker.
///
/// Expands to a `let` binding so the guard lives for the rest of the test body
/// and its `Drop` restores the pointer. Kept as a macro so all desktop tests
/// express the gate identically and a future change lands in one place.
macro_rules! acquire_desktop {
    () => {
        let _desktop_guard = match $crate::support::gui::DesktopGuard::acquire(
            &$crate::support::gui::InputGate::production(),
        ) {
            Some(guard) => guard,
            None => {
                $crate::support::gui::skip("the user is using the computer");
                return;
            }
        };
    };
}

use std::time::{Duration, Instant};

use plnk::gfx::Rgba8;
use plnk::gfx::device::GraphicsDevice;
use plnk::platform::dpi::{DpiMap, PhysicalPoint, PhysicalRect};
use plnk::platform::window::ToolbarWindow as BackdropWindow;
use plnk::settings::Settings;
use plnk::ui::toolbar::{ButtonId, ToolbarWindow};

use windows::Win32::Foundation::{FILETIME, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DEVMODEW, DIB_RGB_COLORS, DeleteDC, DeleteObject, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW,
    GetDC, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC,
    SRCCOPY, SelectObject,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, MSG, PM_REMOVE, PeekMessageW, PostMessageW, SetCursorPos,
    TranslateMessage, WM_LBUTTONDOWN, WM_LBUTTONUP,
};
use windows::core::PCWSTR;

// ---------------------------------------------------------------------------
// The acceptance test
// ---------------------------------------------------------------------------

#[test]
#[ignore = "creates real topmost windows on the desktop"]
fn toolbar_slides_exits_and_docks_without_per_frame_callbacks() {
    acquire_desktop!();
    let device = GraphicsDevice::new().expect("create graphics device");
    let settings = Settings::defaults();
    // Far from the screen edge so the whole slide is visible.
    let origin = PhysicalPoint::new(400, 900);
    let mut toolbar =
        ToolbarWindow::new_at(&device, &settings, origin).expect("create native toolbar");
    assert_eq!(
        toolbar.legacy_bitmap_count(),
        19,
        "the real toolbar must decode all original legacy PNG states"
    );
    assert!(
        toolbar.take_asset_warnings().is_empty(),
        "packaged legacy assets must load without fallback warnings"
    );
    let dpi = DpiMap::from_window(toolbar.hwnd());
    let panel = toolbar.panel_rect();
    let strip = toolbar.dock_strip_rect();
    assert_eq!(panel.left, origin.x);
    assert_eq!(panel.top, origin.y);
    assert!(
        panel.width() > 800 && panel.width() < 2500,
        "unexpected panel width {}",
        panel.width()
    );
    assert!(strip.width() < panel.width() / 2);
    assert_eq!(strip.right, panel.right);
    assert_eq!(strip.height(), panel.height());

    // Black backdrop directly behind the panel's travel area.
    let mut backdrop = BackdropWindow::new(&device, panel).expect("create black backdrop window");
    backdrop
        .fill(Rgba8::new(0, 0, 0, 255))
        .expect("fill backdrop");
    backdrop.show();
    pump_for(Duration::from_millis(150));

    let original_cursor = cursor_position();
    move_cursor(PhysicalPoint::new(10, 10));
    let region = ScreenRegion::new(panel);

    let display_hz = display_refresh_hz();
    assert!(
        display_hz >= 200.0,
        "the acceptance monitor must run at >= 200Hz (got {display_hz:.1}Hz)"
    );

    // --- 1) Slide in over ENTER_DURATION with no per-frame callbacks -------
    let enter_start_counts = toolbar.callback_counts();
    toolbar.enter().expect("enter");
    let enter = measure_animation(&device, &region, Duration::from_millis(300), 15);
    let after_enter = toolbar.callback_counts();
    assert_eq!(
        after_enter.timer - enter_start_counts.timer,
        0,
        "the slide-in must not use a per-frame timer"
    );
    assert!(
        after_enter.paint - enter_start_counts.paint <= 1,
        "slide-in must not repaint per frame (saw {} paints)",
        after_enter.paint - enter_start_counts.paint
    );
    assert_eq!(
        after_enter.custom - enter_start_counts.custom,
        0,
        "slide-in must not schedule app-level per-frame callbacks"
    );
    assert!(
        enter.frames >= 20,
        "the compositor must present many frames while sliding (saw {})",
        enter.frames
    );
    let median = percentile(&enter.intervals_ms, 0.50);
    let p95 = percentile(&enter.intervals_ms, 0.95);
    assert!(median <= 4.2, "240Hz slide-in median {median:.3}ms > 4.2ms");
    assert!(p95 <= 8.4, "240Hz slide-in p95 {p95:.3}ms > 8.4ms");

    pump_for(Duration::from_millis(60));
    assert!(toolbar.is_visible());
    region.refresh();
    assert_eq!(
        region.find_left_edge(),
        Some(0),
        "the panel must be fully slid in"
    );
    assert!(
        enter.positions.iter().any(|position| *position > 100),
        "the panel must be seen mid-slide: {:?}",
        enter.positions
    );
    for pair in enter.positions.windows(2) {
        assert!(
            pair[1] <= pair[0] + 4,
            "the slide-in must move left monotonically: {:?}",
            enter.positions
        );
    }

    // --- 2) Visual fidelity against the legacy palette ---------------------
    region.refresh();
    let background = region.pixel_rgb(4, 4);
    assert!(
        (235..=252).contains(&background[0])
            && (235..=252).contains(&background[1])
            && (235..=252).contains(&background[2]),
        "legacy panel background is WhiteSmoke, got {background:?}"
    );
    // Legacy pen 1 is RGB(225,60,60); its colour shows through the transparent
    // underline stripe of the pencil bitmap (84-88% of the button height).
    let pen1 = toolbar
        .button_screen_rect(ButtonId::Pen(1))
        .expect("pen 1 visible");
    region.refresh();
    let pen1_colour = region.pixel_rgb(
        pen1.left - panel.left + pen1.width() / 2,
        pen1.top - panel.top + (pen1.height() as f32 * 0.86) as i32,
    );
    assert!(
        (pen1_colour[0] as i32 - 225).abs() <= 12
            && (pen1_colour[1] as i32 - 60).abs() <= 12
            && (pen1_colour[2] as i32 - 60).abs() <= 12,
        "pen 1 underline must show the legacy pen colour, got {pen1_colour:?}"
    );
    // Every drawn button carries dark icon pixels.
    for id in [
        ButtonId::Dock,
        ButtonId::Eraser,
        ButtonId::Snapshot,
        ButtonId::Exit,
    ] {
        let rect = toolbar
            .button_screen_rect(id)
            .unwrap_or_else(|| panic!("{id:?} must be visible"));
        assert!(
            region.dark_pixels(PhysicalRect::new(
                rect.left - panel.left,
                rect.top - panel.top,
                rect.right - panel.left,
                rect.bottom - panel.top,
            )) > 20,
            "{id:?} must draw a dark icon"
        );
    }

    // The active pencil must contain the original bitmap's WhiteSmoke body
    // and black selection dot, not the old pure-white vector active treatment.
    let active_pen = toolbar
        .button_screen_rect(ButtonId::Pen(1))
        .expect("active pen visible");
    // Source-derived interior sample: pencil_act.png (40,80) is WhiteSmoke
    // (245) over the same WhiteSmoke button face. The old vector active body
    // was pure white, so this catches a fallback/regression at a stable point
    // away from resampling edges.
    let active_body = region.pixel_rgb(
        active_pen.left - panel.left + active_pen.width() * 40 / 128,
        active_pen.top - panel.top + active_pen.height() * 80 / 128,
    );
    assert!(
        active_body
            .iter()
            .all(|channel| (238..=248).contains(channel)),
        "selected pencil must use the original WhiteSmoke body, got {active_body:?}"
    );
    let active_dot = region.pixel_rgb(
        active_pen.left - panel.left + active_pen.width() * 40 / 128,
        active_pen.top - panel.top + active_pen.height() * 15 / 128,
    );
    assert!(
        active_dot.iter().all(|channel| *channel < 60),
        "the original active bitmap's black selection dot must remain visible, got {active_dot:?}"
    );

    // --- 3) Fast exit: 160 ms, measured frames, then hidden ----------------
    let before_exit = toolbar.callback_counts();
    toolbar.fast_exit().expect("exit");
    let exit = measure_animation(&device, &region, Duration::from_millis(160), 15);
    let after_exit_animation = toolbar.callback_counts();
    assert_eq!(
        after_exit_animation.timer - before_exit.timer,
        0,
        "the exit animation itself must not schedule per-frame timers"
    );
    assert_eq!(
        after_exit_animation.paint - before_exit.paint,
        0,
        "the exit animation must not repaint per frame"
    );
    assert_eq!(
        after_exit_animation.custom - before_exit.custom,
        0,
        "the exit animation must not schedule app-level callbacks"
    );
    assert!(
        exit.frames >= 20,
        "the compositor must present many frames while exiting (saw {})",
        exit.frames
    );
    let exit_median = percentile(&exit.intervals_ms, 0.50);
    let exit_p95 = percentile(&exit.intervals_ms, 0.95);
    assert!(
        exit_median <= 4.2,
        "240Hz exit median {exit_median:.3}ms > 4.2ms"
    );
    assert!(exit_p95 <= 8.4, "240Hz exit p95 {exit_p95:.3}ms > 8.4ms");
    for pair in exit.positions.windows(2) {
        assert!(
            pair[1] + 4 >= pair[0],
            "the exit must move right monotonically: {:?}",
            exit.positions
        );
    }
    wait_until(Duration::from_millis(500), || !toolbar.is_visible());
    pump_for(Duration::from_millis(30));
    assert!(!toolbar.is_visible());
    region.refresh();
    assert_eq!(region.find_left_edge(), None, "exit must hide the panel");
    assert_eq!(
        toolbar.callback_counts().timer - before_exit.timer,
        1,
        "exactly one animation-end settle timer is allowed per fast exit"
    );

    // --- 4) Esc during the animation: retarget from the current position ---
    let exit_counts = toolbar.callback_counts();
    toolbar.enter().expect("re-enter");
    pump_for(Duration::from_millis(100));
    region.refresh();
    let mid_slide = region
        .find_left_edge()
        .expect("panel must be partially visible before Esc");
    assert!(
        mid_slide > 100,
        "expected a mid-slide position, got {mid_slide}"
    );
    let before_esc = toolbar.callback_counts();
    toolbar.fast_exit().expect("Esc during slide");
    let esc = measure_animation(&device, &region, Duration::from_millis(160), 15);
    let after_esc = toolbar.callback_counts();
    assert_eq!(
        after_esc.timer - before_esc.timer,
        0,
        "the retargeted exit itself must not schedule per-frame timers"
    );
    assert!(
        esc.positions
            .first()
            .is_some_and(|first| first.abs_diff(mid_slide) < 220),
        "retargeting must start at the current visual position ({mid_slide}), saw {:?}",
        esc.positions.first()
    );
    for pair in esc.positions.windows(2) {
        assert!(
            pair[1] + 4 >= pair[0],
            "the retargeted exit must move right monotonically: {:?}",
            esc.positions
        );
    }
    wait_until(Duration::from_millis(500), || !toolbar.is_visible());
    let settled_counts = toolbar.callback_counts();
    assert_eq!(
        settled_counts.timer - exit_counts.timer,
        1,
        "exactly one animation-end settle timer is allowed per visible animation"
    );

    // --- 4) Rapid repeated show/hide --------------------------------------
    let rapid_start = toolbar.callback_counts();
    for _ in 0..5 {
        toolbar.enter().expect("rapid enter");
        pump_for(Duration::from_millis(60));
        toolbar.fast_exit().expect("rapid exit");
        pump_for(Duration::from_millis(40));
    }
    toolbar.enter().expect("final enter");
    wait_until(Duration::from_millis(700), || {
        region.refresh();
        toolbar.is_visible() && region.find_left_edge() == Some(0)
    });
    assert!(toolbar.is_visible());
    region.refresh();
    assert_eq!(region.find_left_edge(), Some(0));
    let rapid_end = toolbar.callback_counts();
    assert!(
        rapid_end.paint - rapid_start.paint <= 2,
        "rapid show/hide must not repaint per frame (saw {} paints)",
        rapid_end.paint - rapid_start.paint
    );
    assert_eq!(
        rapid_end.custom - rapid_start.custom,
        0,
        "rapid show/hide must not schedule app-level callbacks"
    );

    // --- 5) Dock / undock --------------------------------------------------
    let dock_counts = toolbar.callback_counts();
    toolbar.dock_to(true).expect("dock");
    let dock = measure_animation(&device, &region, Duration::from_millis(300), 15);
    wait_until(Duration::from_millis(600), || {
        toolbar.is_docked() && toolbar.window_rect().is_ok_and(|rect| rect == strip)
    });
    assert!(toolbar.is_docked());
    assert_eq!(
        toolbar.window_rect().expect("window rect"),
        strip,
        "a docked toolbar window is exactly the dock strip"
    );
    region.refresh();
    assert_eq!(
        region.find_left_edge(),
        Some(strip.left - panel.left),
        "the docked strip sits at the panel's right edge"
    );
    let dock_centre = strip.center();
    assert_eq!(
        toolbar.hit_test_screen(dock_centre),
        Some(ButtonId::Dock),
        "the dock strip centre must hit the dock button"
    );
    assert_eq!(
        toolbar.hit_test_screen(PhysicalPoint::new(
            panel.left + 40,
            panel.top + panel.height() / 2
        )),
        None,
        "the empty area left of the dock strip must not be part of the window"
    );
    assert!(
        dock.positions.windows(2).all(|pair| pair[1] + 4 >= pair[0]),
        "docking must slide right monotonically: {:?}",
        dock.positions
    );
    assert!(
        dock.frames >= 20,
        "the compositor must present many frames while docking (saw {})",
        dock.frames
    );
    let dock_median = percentile(&dock.intervals_ms, 0.50);
    let dock_p95 = percentile(&dock.intervals_ms, 0.95);
    assert!(
        dock_median <= 4.2,
        "240Hz dock median {dock_median:.3}ms > 4.2ms"
    );
    assert!(dock_p95 <= 8.4, "240Hz dock p95 {dock_p95:.3}ms > 8.4ms");

    toolbar.dock_to(false).expect("undock");
    wait_until(Duration::from_millis(700), || {
        !toolbar.is_docked()
            && !toolbar.is_animating()
            && toolbar.window_rect().is_ok_and(|rect| rect == panel)
    });
    assert!(
        !toolbar.is_animating(),
        "the undock slide must have finished before hit testing"
    );
    assert_eq!(
        toolbar.window_rect().expect("window rect"),
        panel,
        "an undocked toolbar window is the full panel"
    );
    let pen3_center = toolbar
        .button_screen_rect(ButtonId::Pen(3))
        .expect("pen 3 visible")
        .center();
    assert_eq!(
        toolbar.hit_test_screen(pen3_center),
        Some(ButtonId::Pen(3)),
        "the undocked panel must hit-test its buttons again"
    );
    let dock_settles = toolbar.callback_counts().timer - dock_counts.timer;
    assert_eq!(
        dock_settles, 1,
        "only the dock settle may use a timer; undocking needs none"
    );

    // --- 6) Clicks are dispatched through the real window procedure --------
    let pen3 = toolbar
        .button_screen_rect(ButtonId::Pen(3))
        .expect("pen 3 visible")
        .center();
    post_click(&toolbar, pen3);
    pump_for(Duration::from_millis(80));
    assert_eq!(
        toolbar.take_clicked(),
        Some(ButtonId::Pen(3)),
        "a real routed click must reach the explicit click stub"
    );

    let dock_button = toolbar
        .button_screen_rect(ButtonId::Dock)
        .expect("dock visible")
        .center();
    post_click(&toolbar, dock_button);
    wait_until(Duration::from_millis(800), || toolbar.is_docked());
    assert!(
        toolbar.is_docked(),
        "clicking the dock button must dock the toolbar"
    );
    assert_eq!(toolbar.take_clicked(), Some(ButtonId::Dock));
    toolbar.dock_to(false).expect("undock after click");
    wait_until(Duration::from_millis(700), || {
        !toolbar.is_docked() && !toolbar.is_animating()
    });

    // --- 7) App CPU during a compositor-driven animation -------------------
    let cpu_before = process_cpu_time();
    let wall = Instant::now();
    toolbar.fast_exit().expect("cpu exit");
    while wall.elapsed() < Duration::from_millis(400) {
        pump_pending();
        std::thread::sleep(Duration::from_millis(2));
    }
    let cpu_seconds = (process_cpu_time() - cpu_before).as_secs_f64();
    let wall_seconds = wall.elapsed().as_secs_f64();
    let percent = cpu_seconds / wall_seconds * 100.0;
    assert!(
        percent <= 2.0,
        "app CPU during the animation must stay <= 2% of one core, got {percent:.3}%"
    );

    move_cursor(original_cursor);
    assert_eq!(
        toolbar.take_error(),
        None,
        "no toolbar operation may fail silently"
    );

    // --- 8) Default position matches the legacy bottom-right rule ----------
    let default_toolbar = ToolbarWindow::new(&device, &settings).expect("default-position toolbar");
    let work = primary_work_area();
    let default_panel = default_toolbar.panel_rect();
    assert_eq!(
        default_panel.left,
        work.right - default_panel.width(),
        "default x must be the working-area right edge minus the panel width"
    );
    assert_eq!(
        default_panel.top,
        work.bottom - default_panel.height() - 15,
        "default y keeps the legacy 15 px bottom margin"
    );

    println!(
        "[toolbar] display={display_hz:.1}Hz dpi={} scale={:.3} panel={}x{} strip={}x{}",
        dpi.dpi(),
        dpi.scale(),
        panel.width(),
        panel.height(),
        strip.width(),
        strip.height()
    );
    println!(
        "[toolbar] enter frames={} intervals={} median={:.3}ms p95={:.3}ms positions={:?}",
        enter.frames,
        enter.intervals_ms.len(),
        percentile(&enter.intervals_ms, 0.50),
        percentile(&enter.intervals_ms, 0.95),
        enter.positions
    );
    println!(
        "[toolbar] exit frames={} median={:.3}ms p95={:.3}ms positions={:?}",
        exit.frames,
        percentile(&exit.intervals_ms, 0.50),
        percentile(&exit.intervals_ms, 0.95),
        exit.positions
    );
    println!(
        "[toolbar] dock frames={} median={:.3}ms p95={:.3}ms positions={:?}",
        dock.frames,
        percentile(&dock.intervals_ms, 0.50),
        percentile(&dock.intervals_ms, 0.95),
        dock.positions
    );
    println!(
        "[toolbar] callbacks enter timer={} paint={} custom={}",
        after_enter.timer - enter_start_counts.timer,
        after_enter.paint - enter_start_counts.paint,
        after_enter.custom - enter_start_counts.custom
    );
    println!("[toolbar] cpu.during_animation={percent:.3}% of one core");
    println!("[toolbar] PASS");
}

/// Captures a screenshot of the rendered toolbar for visual comparison with
/// the legacy screenshots (`screenshot1.jpg` / `screenshot2.jpg`). The file is
/// written as a 32-bit BMP below `%TEMP%` so no image encoder is needed here.
#[test]
#[ignore = "writes a screenshot of the toolbar to %TEMP%"]
fn capture_toolbar_preview() {
    acquire_desktop!();
    let device = GraphicsDevice::new().expect("create graphics device");
    let mut settings = Settings::defaults();
    // Twice the default height makes the icons legible in the screenshot; the
    // layout fractions are identical at any toolbar height.
    settings.toolbar_height = 0.09;
    let origin = PhysicalPoint::new(100, 300);
    let mut toolbar =
        ToolbarWindow::new_at(&device, &settings, origin).expect("create native toolbar");
    toolbar.enter().expect("enter");
    pump_for(Duration::from_millis(450));
    let panel = toolbar.panel_rect();
    let region = ScreenRegion::new(panel);
    region.refresh();
    let path = std::env::temp_dir().join("plnk-toolbar-preview.bmp");
    region.save_bmp(&path).expect("save toolbar screenshot");
    println!(
        "[toolbar] preview={} panel={}x{}",
        path.display(),
        panel.width(),
        panel.height()
    );
}

/// Screen working area of the monitor containing the origin.
fn primary_work_area() -> PhysicalRect {
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.expect("GetMonitorInfoW");
    PhysicalRect::new(
        info.rcWork.left,
        info.rcWork.top,
        info.rcWork.right,
        info.rcWork.bottom,
    )
}

// ---------------------------------------------------------------------------
// Evidence helpers
// ---------------------------------------------------------------------------

struct AnimationEvidence {
    frames: u64,
    intervals_ms: Vec<f64>,
    /// Region-relative left edge of the panel at the capture cadence.
    positions: Vec<i32>,
}

fn measure_animation(
    device: &GraphicsDevice,
    region: &ScreenRegion,
    duration: Duration,
    capture_ms: u64,
) -> AnimationEvidence {
    let mut frame_times: Vec<i64> = Vec::new();
    let mut last_frame_time: Option<i64> = None;
    let mut time_frequency = 0i64;
    let mut positions = Vec::new();
    let start = Instant::now();
    let mut next_capture = start;
    while start.elapsed() < duration {
        if let Ok(statistics) = device.frame_statistics() {
            time_frequency = statistics.time_frequency;
            if last_frame_time != Some(statistics.last_frame_time) {
                last_frame_time = Some(statistics.last_frame_time);
                frame_times.push(statistics.last_frame_time);
            }
        }
        if Instant::now() >= next_capture {
            region.refresh();
            if let Some(left) = region.find_left_edge() {
                positions.push(left);
            }
            next_capture += Duration::from_millis(capture_ms);
        }
        pump_pending();
    }
    let mut intervals_ms = Vec::new();
    if time_frequency > 0 {
        for pair in frame_times.windows(2).skip(1) {
            let delta_ms = (pair[1] - pair[0]) as f64 * 1000.0 / time_frequency as f64;
            if (0.1..50.0).contains(&delta_ms) {
                intervals_ms.push(delta_ms);
            }
        }
    }
    AnimationEvidence {
        frames: frame_times.len() as u64,
        intervals_ms,
        positions,
    }
}

fn post_click(toolbar: &ToolbarWindow, screen: PhysicalPoint) {
    let window = toolbar.window_rect().expect("window rect for click");
    let x = screen.x - window.left;
    let y = screen.y - window.top;
    let packed = ((y as isize) << 16) | (x as isize & 0xFFFF);
    unsafe {
        PostMessageW(
            Some(toolbar.hwnd()),
            WM_LBUTTONDOWN,
            // MK_LBUTTON (0x0001); the WPARAM flag is informational for a
            // posted button-down message and needs no extra windows feature.
            WPARAM(0x0001),
            LPARAM(packed),
        )
        .expect("post WM_LBUTTONDOWN");
        PostMessageW(
            Some(toolbar.hwnd()),
            WM_LBUTTONUP,
            WPARAM(0),
            LPARAM(packed),
        )
        .expect("post WM_LBUTTONUP");
    }
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        pump_for(Duration::from_millis(10));
    }
    condition()
}

fn cursor_position() -> PhysicalPoint {
    let mut point = windows::Win32::Foundation::POINT::default();
    unsafe { GetCursorPos(&mut point) }.expect("GetCursorPos");
    PhysicalPoint::new(point.x, point.y)
}

fn move_cursor(point: PhysicalPoint) {
    unsafe { SetCursorPos(point.x, point.y) }.expect("SetCursorPos");
}

fn process_cpu_time() -> Duration {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
        .expect("GetProcessTimes");
    }
    Duration::from_nanos((filetime_ticks(kernel) + filetime_ticks(user)) * 100)
}

fn filetime_ticks(value: FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
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

fn percentile(values: &[f64], fraction: f64) -> f64 {
    assert!(!values.is_empty(), "percentile of an empty sample");
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite interval sample"));
    let rank = (sorted.len() as f64 * fraction).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
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

/// BitBlt readback of the desktop region the toolbar lives in.
struct ScreenRegion {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    screen: windows::Win32::Graphics::Gdi::HDC,
    memory: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    previous: windows::Win32::Graphics::Gdi::HGDIOBJ,
    pixels: *mut u32,
}

impl ScreenRegion {
    fn new(bounds: PhysicalRect) -> Self {
        let width = bounds.width();
        let height = bounds.height();
        assert!(width > 0 && height > 0, "screen region must be non-empty");
        unsafe {
            let screen = GetDC(None);
            assert!(!screen.is_invalid(), "GetDC(NULL) failed");
            let memory = CreateCompatibleDC(Some(screen));
            assert!(!memory.is_invalid(), "CreateCompatibleDC failed");
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
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
            assert!(
                !pixel_pointer.is_null(),
                "CreateDIBSection returned no bits"
            );
            let previous = SelectObject(memory, windows::Win32::Graphics::Gdi::HGDIOBJ(bitmap.0));
            Self {
                x: bounds.left,
                y: bounds.top,
                width,
                height,
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
                self.width,
                self.height,
                Some(self.screen),
                self.x,
                self.y,
                SRCCOPY | CAPTUREBLT,
            )
            .expect("BitBlt from the screen");
        }
    }

    fn pixel_rgb(&self, x: i32, y: i32) -> [u8; 3] {
        assert!(x >= 0 && y >= 0 && x < self.width && y < self.height);
        let value = unsafe { *self.pixels.add((y * self.width + x) as usize) };
        [(value >> 16) as u8, (value >> 8) as u8, value as u8]
    }

    /// Writes the captured pixels as a top-down 32-bit BMP.
    fn save_bmp(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let image_size = (self.width as u32)
            .checked_mul(self.height as u32)
            .and_then(|value| value.checked_mul(4))
            .expect("screen region size overflow");
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        file.write_all(b"BM")?;
        file.write_all(&(14u32 + 40 + image_size).to_le_bytes())?;
        file.write_all(&0u16.to_le_bytes())?;
        file.write_all(&0u16.to_le_bytes())?;
        file.write_all(&(14u32 + 40).to_le_bytes())?;
        file.write_all(&40u32.to_le_bytes())?;
        file.write_all(&self.width.to_le_bytes())?;
        file.write_all(&(-self.height).to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&32u16.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&image_size.to_le_bytes())?;
        file.write_all(&2835i32.to_le_bytes())?;
        file.write_all(&2835i32.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        let pixels =
            unsafe { std::slice::from_raw_parts(self.pixels as *const u8, image_size as usize) };
        file.write_all(pixels)?;
        file.flush()
    }

    /// Region-relative x of the toolbar's left edge, found by scanning for the
    /// first run of non-black pixels against the black backdrop.
    fn find_left_edge(&self) -> Option<i32> {
        const MIN_RUN: i32 = 8;
        let mut edges: Vec<i32> = Vec::new();
        for y in (2..self.height - 2).step_by(7) {
            let mut run = 0;
            for x in 0..self.width {
                let [r, g, b] = self.pixel_rgb(x, y);
                let lit = u32::from(r) + u32::from(g) + u32::from(b) > 150;
                if lit {
                    run += 1;
                    if run == MIN_RUN {
                        edges.push(x - MIN_RUN + 1);
                        break;
                    }
                } else {
                    run = 0;
                }
            }
        }
        if edges.is_empty() {
            return None;
        }
        edges.sort_unstable();
        Some(edges[edges.len() / 2])
    }

    /// Counts dark pixels inside a region-relative rectangle (icon proof).
    fn dark_pixels(&self, rect: PhysicalRect) -> usize {
        let left = rect.left.max(0);
        let top = rect.top.max(0);
        let right = rect.right.min(self.width);
        let bottom = rect.bottom.min(self.height);
        let mut count = 0;
        for y in (top..bottom).step_by(2) {
            for x in (left..right).step_by(2) {
                let [r, g, b] = self.pixel_rgb(x, y);
                if u32::from(r) + u32::from(g) + u32::from(b) < 200 {
                    count += 1;
                }
            }
        }
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
