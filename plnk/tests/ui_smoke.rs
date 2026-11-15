//! Desktop UI smoke tests and the native graphics decision-gate probe.
//!
//! The `#[ignore]` tests create real, short-lived topmost windows on the
//! desktop so the measurements describe what the user can actually see. They
//! never send synthetic input, never register global hotkeys and never touch
//! the running gInk instance; every window is destroyed before the test
//! returns.
//!
//! What the animation test measures (and what it does not):
//! - `app callbacks`: real WM_TIMER / WM_PAINT / WM_APP+1 messages delivered to
//!   the window procedure while a DirectComposition animation is running.
//! - `presentation frames`: `IDCompositionDevice::GetFrameStatistics`
//!   (`lastFrameTime`, QPC ticks) sampled in a tight loop, cross-checked
//!   against DWM's `DWM_TIMING_INFO` (`cFramesDisplayed`, `qpcFrameDisplayed`,
//!   `rateRefresh`). Callback frequency is never used as a frame rate.
//! - `visible movement`: screen pixels sampled with BitBlt from the composited
//!   desktop while the toolbar block slides.
//!
//! Run the desktop tests with:
//! `cargo test --manifest-path plnk/Cargo.toml --test ui_smoke -- --ignored --test-threads=1`
//! Run the decision-gate probe in Release with:
//! `cargo test --release --manifest-path plnk/Cargo.toml --test ui_smoke -- --ignored --test-threads=1 graphics_gate_probe --nocapture`

use std::time::{Duration, Instant};

use plnk::gfx::device::GraphicsDevice;
use plnk::gfx::{Easing, Rgba8};
use plnk::platform::dpi::{DpiMap, PhysicalPoint, PhysicalRect};
use plnk::platform::window::{APP_CALLBACK_MESSAGE, MonotonicClock, OverlayWindow, ToolbarWindow};

use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DEVMODEW, DIB_RGB_COLORS, DeleteDC, DeleteObject, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW,
    GetDC, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::Performance::QueryPerformanceFrequency;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources, GetProcessHandleCount,
    GetProcessTimes,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, KillTimer, MSG, PM_REMOVE, PeekMessageW, PostMessageW,
    SetCursorPos, SetTimer, TranslateMessage,
};
use windows::core::PCWSTR;

// ---------------------------------------------------------------------------
// Required acceptance tests
// ---------------------------------------------------------------------------

#[test]
#[ignore = "creates real topmost windows on the desktop"]
fn transparent_overlay_accepts_and_passes_input_by_mode() {
    let device = GraphicsDevice::new().expect("create graphics device");
    let evidence = overlay_evidence(&device);

    assert!(
        evidence.backdrop_white[0] > 240
            && evidence.backdrop_white[1] > 240
            && evidence.backdrop_white[2] > 240,
        "backdrop must be visible as white before the overlay is shown: {:?}",
        evidence.backdrop_white
    );
    // A 50% red DComp layer over the white backdrop must read ~(255,128,128):
    // this is the honest screen-visible proof of per-pixel transparency.
    assert!(
        evidence.blended[0] >= 220,
        "red channel must stay high for a half-transparent red overlay: {:?}",
        evidence.blended
    );
    assert!(
        (80..=180).contains(&evidence.blended[1]) && (80..=180).contains(&evidence.blended[2]),
        "green/blue must be roughly half-blended with the white backdrop: {:?}",
        evidence.blended
    );
    assert!(
        evidence.blended[1].abs_diff(evidence.blended[2]) <= 20,
        "blend must be neutral in green/blue: {:?}",
        evidence.blended
    );
    assert!(
        (80..=180).contains(&evidence.blended_click_through[1])
            && (80..=180).contains(&evidence.blended_click_through[2]),
        "overlay content must stay visibly transparent after switching to click-through: {:?}",
        evidence.blended_click_through
    );

    assert_ne!(
        evidence.interactive_overlay_moves, 0,
        "with click-through off the overlay must receive real routed WM_MOUSEMOVE \
         (overlay={}, backdrop={})",
        evidence.interactive_overlay_moves, evidence.interactive_backdrop_moves
    );
    assert_ne!(
        evidence.click_through_backdrop_moves, 0,
        "with click-through on the window below must receive the routed mouse moves \
         (overlay={}, backdrop={})",
        evidence.click_through_overlay_moves, evidence.click_through_backdrop_moves
    );
    assert_eq!(
        evidence.click_through_overlay_moves, 0,
        "a click-through overlay must not receive routed mouse messages"
    );
}

#[test]
#[ignore = "creates real topmost windows on the desktop"]
fn toolbar_animation_has_no_per_frame_app_callback() {
    let device = GraphicsDevice::new().expect("create graphics device");
    let evidence = animation_evidence(&device);

    assert!(
        evidence.timer_meter_ok && evidence.custom_meter_ok,
        "the callback meter itself must be provably live (timer={}, custom={})",
        evidence.timer_meter_ok,
        evidence.custom_meter_ok
    );
    assert_eq!(
        evidence.timer_callbacks, 0,
        "DComp animation must not rely on WM_TIMER ticks"
    );
    assert_eq!(
        evidence.paint_callbacks, 0,
        "DComp animation must not rely on WM_PAINT redraws"
    );
    assert_eq!(
        evidence.custom_callbacks, 0,
        "DComp animation must not schedule app-level per-frame callbacks"
    );

    assert!(
        evidence.frames_observed >= 20,
        "the compositor must present many frames during the animation (saw {})",
        evidence.frames_observed
    );
    assert!(
        evidence.frame_intervals_ms.len() >= 20,
        "need enough presented-frame intervals, got {}",
        evidence.frame_intervals_ms.len()
    );
    if evidence.display_refresh_hz >= 200.0 {
        let median = percentile(&evidence.frame_intervals_ms, 0.50);
        let p95 = percentile(&evidence.frame_intervals_ms, 0.95);
        assert!(
            median <= 4.2,
            "240Hz presented-frame interval median must be <= 4.2ms, got {median:.2}ms"
        );
        assert!(
            p95 <= 8.4,
            "240Hz presented-frame interval p95 must be <= 8.4ms, got {p95:.2}ms"
        );
    }

    let first = evidence
        .positions
        .first()
        .expect("the moving block must be visible at the start")
        .1;
    let last = evidence
        .positions
        .last()
        .expect("the moving block must be visible at the end")
        .1;
    assert!(
        first <= 80,
        "block must start near the initial offset (saw {first}px)"
    );
    assert!(
        (330..=410).contains(&last),
        "block must visibly reach the animation target (saw {last}px)"
    );
    assert!(
        last - first >= 250,
        "block must visibly travel across the screen (start {first}, end {last})"
    );
    for pair in evidence.positions.windows(2) {
        assert!(
            pair[1].1 >= pair[0].1 - 4,
            "visible motion must be monotonic within capture noise: {:?} -> {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn dpi_physical_coordinates_roundtrip() {
    // Pure conversion test (no desktop window): physical -> DIP -> physical
    // must be lossless for every realistic screen coordinate at every Windows
    // DPI, including the acceptance target's 150% (144 DPI).
    let dpis = [96u32, 120, 144, 168, 192, 216, 240, 288];
    let coordinates = [
        -1_000_000, -32_768, -1_920, -1, 0, 1, 96, 144, 1_707, 4_096, 32_768, 1_000_000,
    ];
    for dpi in dpis {
        let map = DpiMap::from_dpi(dpi);
        assert_eq!(map.dpi(), dpi);
        for &x in &coordinates {
            for &y in &[0, -1_440, 999_999, -999_999] {
                let physical = PhysicalPoint::new(x, y);
                let dip = map.physical_to_dip(physical);
                let back = map.dip_to_physical(dip);
                assert_eq!(
                    back, physical,
                    "physical roundtrip failed at {dpi} DPI: {physical:?} -> {dip:?} -> {back:?}"
                );
            }
        }
        // Rect corners must survive the same trip.
        let rect = PhysicalRect::new(-100, -200, 1_707, 1_067);
        let dip_rect = map.physical_rect_to_dip(rect);
        let back = map.dip_rect_to_physical(dip_rect);
        assert_eq!(back, rect, "physical rect roundtrip failed at {dpi} DPI");
    }

    assert_eq!(
        DpiMap::from_dpi(144).scale(),
        1.5,
        "150% DPI is a 1.5 scale"
    );
    assert_eq!(
        DpiMap::from_dpi(0).dpi(),
        96,
        "invalid DPI falls back to 96"
    );

    // DIP -> physical -> DIP may quantize to a whole physical pixel (that
    // quantization is the point: physical pixels are authoritative), so the
    // error is bounded by half a physical pixel expressed in DIPs.
    for dpi in dpis {
        let map = DpiMap::from_dpi(dpi);
        let tolerance = 0.51 * 96.0 / f64::from(dpi);
        for &(x, y) in &[
            (0.0f32, 0.0f32),
            (12.5, 37.25),
            (-100.75, 640.0),
            (1_706.666_7, 1_066.666_7),
        ] {
            let dip = plnk::platform::dpi::DipPoint { x, y };
            let back = map.physical_to_dip(map.dip_to_physical(dip));
            assert!(
                f64::from((back.x - x).abs()) <= tolerance
                    && f64::from((back.y - y).abs()) <= tolerance,
                "DIP roundtrip exceeded half a physical pixel at {dpi} DPI: ({x}, {y}) -> ({}, {})",
                back.x,
                back.y
            );
        }
    }
}

#[test]
fn monotonic_clock_is_non_decreasing() {
    let first = MonotonicClock::now();
    std::thread::sleep(Duration::from_millis(5));
    let second = MonotonicClock::now();
    assert!(
        second > first,
        "MonotonicClock::now must advance ({first:?} -> {second:?})"
    );
    assert!(
        second - first >= Duration::from_millis(4),
        "MonotonicClock must track wall time across a 5ms sleep (measured {:?})",
        second - first
    );
}

// ---------------------------------------------------------------------------
// Decision-gate probe (Step 4) — run in Release, prints the numbers recorded in
// docs/acceptance/graphics-gate.md.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "decision-gate probe: real desktop windows and ~10s of measurements"]
fn graphics_gate_probe() {
    let device = GraphicsDevice::new().expect("create graphics device");

    // --- Physical coordinates on the real window system -------------------
    let probe_bounds = PhysicalRect::from_xywh(120, 140, 260, 120);
    let probe_window = ToolbarWindow::new(&device, probe_bounds).expect("probe toolbar window");
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(probe_window.hwnd()) };
    let dpi_map = DpiMap::from_window(probe_window.hwnd());
    assert_eq!(dpi_map.dpi(), dpi);
    assert_eq!(
        probe_window.window_rect().expect("query probe window rect"),
        probe_bounds,
        "a per-monitor-v2 window must land on the exact physical pixel rect"
    );
    for corner in [
        (probe_bounds.left, probe_bounds.top),
        (probe_bounds.right, probe_bounds.bottom),
    ] {
        let physical = PhysicalPoint::new(corner.0, corner.1);
        let roundtrip = dpi_map.dip_to_physical(dpi_map.physical_to_dip(physical));
        assert_eq!(roundtrip, physical, "window corner physical roundtrip");
    }
    let devmode_refresh = display_refresh_hz();
    match dwm_timing(probe_window.hwnd()) {
        Ok(timing) => println!(
            "[gate] dwm.timing.window=ok refresh={:?}Hz displayed_frames={}",
            timing.refresh_hz, timing.displayed
        ),
        Err(error) => println!("[gate] dwm.timing.window=error {error}"),
    }
    match dwm_timing(HWND::default()) {
        Ok(timing) => println!(
            "[gate] dwm.timing.desktop=ok refresh={:?}Hz displayed_frames={}",
            timing.refresh_hz, timing.displayed
        ),
        Err(error) => println!("[gate] dwm.timing.desktop=error {error}"),
    }

    // --- Transparent overlay, input modes ---------------------------------
    let overlay = overlay_evidence(&device);
    println!(
        "[gate] overlay.backdrop.pixel={:?} blended.pixel={:?} click_through.pixel={:?} overlay.hwnd={}",
        overlay.backdrop_white,
        overlay.blended,
        overlay.blended_click_through,
        overlay.overlay_hwnd.0 as usize
    );
    println!(
        "[gate] overlay.interactive overlay_moves={} backdrop_moves={}",
        overlay.interactive_overlay_moves, overlay.interactive_backdrop_moves
    );
    println!(
        "[gate] overlay.click_through overlay_moves={} backdrop_moves={}",
        overlay.click_through_overlay_moves, overlay.click_through_backdrop_moves
    );
    assert!(overlay.backdrop_white[0] > 240 && overlay.backdrop_white[1] > 240);
    assert!(
        (80..=180).contains(&overlay.blended[1]) && (80..=180).contains(&overlay.blended[2]),
        "DComp overlay did not blend with the backdrop: {:?}",
        overlay.blended
    );
    assert!(
        (80..=180).contains(&overlay.blended_click_through[1])
            && (80..=180).contains(&overlay.blended_click_through[2]),
        "overlay content disappeared when click-through was enabled: {:?}",
        overlay.blended_click_through
    );
    assert!(overlay.interactive_overlay_moves > 0);
    assert!(overlay.click_through_backdrop_moves > 0);
    assert_eq!(overlay.click_through_overlay_moves, 0);

    // --- CPU during a compositor-driven animation (no probe polling) ------
    let cpu = cpu_evidence(&device);
    println!(
        "[gate] animation.cpu.percent_of_one_core={:.4} cpu_ms={:.3} wall_ms={:.1}",
        cpu.percent_of_one_core,
        cpu.cpu_seconds * 1000.0,
        cpu.wall_seconds * 1000.0
    );

    // --- Memory during the small prototype --------------------------------
    let memory = {
        let bounds = PhysicalRect::from_xywh(200, 200, 320, 120);
        let mut toolbar = ToolbarWindow::new(&device, bounds).expect("memory toolbar");
        toolbar.fill(Rgba8::new(40, 120, 220, 255)).expect("fill");
        toolbar.show();
        let mut overlay = OverlayWindow::new(&device, bounds).expect("memory overlay");
        overlay.fill(Rgba8::new(255, 0, 0, 96)).expect("fill");
        overlay.set_click_through(true).expect("click-through");
        overlay.show();
        toolbar
            .animate_to(
                PhysicalPoint::new(120, 40),
                Duration::from_millis(500),
                Easing::EaseOutCubic,
            )
            .expect("memory animation");
        pump_for(Duration::from_millis(250));
        memory_snapshot()
    };
    println!(
        "[gate] memory.during_prototype.private_bytes={} working_set_bytes={}",
        memory.private_bytes, memory.working_set
    );

    // --- Compositor animation + callbacks ---------------------------------
    let animation = animation_evidence(&device);
    let median = percentile(&animation.frame_intervals_ms, 0.50);
    let p95 = percentile(&animation.frame_intervals_ms, 0.95);
    println!(
        "[gate] display.refresh.devmode={:.1}Hz dcomp={:.1}Hz dwm={:?}Hz (source={}) dpi={} scale={:.3}",
        devmode_refresh,
        animation.refresh_hz,
        animation.dwm_refresh_hz,
        animation.dwm_source,
        dpi,
        dpi_map.scale()
    );
    println!(
        "[gate] animation.frames_observed={} intervals={} median={:.3}ms p95={:.3}ms min={:.3}ms max={:.3}ms",
        animation.frames_observed,
        animation.frame_intervals_ms.len(),
        median,
        p95,
        animation
            .frame_intervals_ms
            .iter()
            .fold(f64::INFINITY, |a, &b| a.min(b)),
        animation
            .frame_intervals_ms
            .iter()
            .fold(0.0f64, |a, &b| a.max(b)),
    );
    println!(
        "[gate] animation.app_callbacks timer={} paint={} custom={} (meter live: timer={} paint={} custom={})",
        animation.timer_callbacks,
        animation.paint_callbacks,
        animation.custom_callbacks,
        animation.timer_meter_ok,
        animation.paint_meter_ok,
        animation.custom_meter_ok
    );
    if animation.dwm_intervals_ms.len() >= 2 {
        println!(
            "[gate] dwm.cross_check intervals={} median={:.3}ms p95={:.3}ms",
            animation.dwm_intervals_ms.len(),
            percentile(&animation.dwm_intervals_ms, 0.50),
            percentile(&animation.dwm_intervals_ms, 0.95),
        );
    } else {
        println!(
            "[gate] dwm.cross_check intervals=0 (DWM displayed-frame counter did not advance; \
             the DComp device statistics above are the frame-interval source)"
        );
    }
    println!(
        "[gate] animation.visible_start={:?} visible_end={:?} samples={}",
        animation.positions.first(),
        animation.positions.last(),
        animation.positions.len()
    );

    assert!(
        animation.timer_callbacks == 0
            && animation.paint_callbacks == 0
            && animation.custom_callbacks == 0,
        "compositor animation must not produce app-level per-frame callbacks"
    );
    assert!(
        animation.timer_meter_ok && animation.custom_meter_ok,
        "callback meter must be demonstrably live"
    );
    assert!(animation.frames_observed >= 20);
    assert!(animation.frame_intervals_ms.len() >= 20);
    assert!(
        devmode_refresh >= 200.0,
        "the acceptance monitor must currently run at >= 200Hz (got {devmode_refresh:.1}Hz)"
    );
    assert!(
        median <= 4.2,
        "240Hz presented-frame median target missed: {median:.3}ms"
    );
    assert!(
        p95 <= 8.4,
        "240Hz presented-frame p95 target missed: {p95:.3}ms"
    );
    let first = animation.positions.first().expect("visible start").1;
    let last = animation.positions.last().expect("visible end").1;
    assert!(first <= 80 && (330..=410).contains(&last) && last - first >= 250);

    // --- 100 window create/destroy cycles (three batches of 100) ----------
    let cycles = leak_cycle_evidence(&device);
    for (index, snapshot) in cycles.batches.iter().enumerate() {
        println!(
            "[gate] cycles100.batch={} handles={} gdi={} user={} private={}",
            index,
            snapshot.handles,
            snapshot.gdi_objects,
            snapshot.user_objects,
            snapshot.private_bytes
        );
    }
    let baseline = cycles.batches[0];
    let latest = *cycles.batches.last().expect("cycle snapshots");
    assert!(
        latest.handles <= baseline.handles + 2,
        "process handles grew across 300 window cycles ({} -> {})",
        baseline.handles,
        latest.handles
    );
    assert!(
        latest.gdi_objects <= baseline.gdi_objects + 2,
        "window cycles leaked GDI objects ({} -> {})",
        baseline.gdi_objects,
        latest.gdi_objects
    );
    assert!(
        latest.user_objects <= baseline.user_objects + 2,
        "window cycles leaked USER objects ({} -> {})",
        baseline.user_objects,
        latest.user_objects
    );
    let private_growth = latest.private_bytes as f64 - baseline.private_bytes as f64;
    assert!(
        private_growth <= baseline.private_bytes as f64 * 0.05 + 2.0 * 1024.0 * 1024.0,
        "window cycles grew private bytes by {private_growth:.0} bytes"
    );
    println!("[gate] PASS");
}

// ---------------------------------------------------------------------------
// Evidence helpers
// ---------------------------------------------------------------------------

struct OverlayEvidence {
    backdrop_white: [u8; 3],
    blended: [u8; 3],
    blended_click_through: [u8; 3],
    overlay_hwnd: HWND,
    interactive_overlay_moves: u32,
    interactive_backdrop_moves: u32,
    click_through_overlay_moves: u32,
    click_through_backdrop_moves: u32,
}

fn overlay_evidence(device: &GraphicsDevice) -> OverlayEvidence {
    let bounds = PhysicalRect::from_xywh(60, 60, 220, 140);
    let center = bounds.center();
    // The test briefly parks the real cursor over these windows (and restores
    // it) because routed WM_MOUSEMOVE is the honest signal for input
    // pass-through; WindowFromPoint does not model WS_EX_TRANSPARENT routing.
    let original_cursor = cursor_position();

    let mut backdrop = ToolbarWindow::new(device, bounds).expect("backdrop window");
    backdrop
        .fill(Rgba8::new(255, 255, 255, 255))
        .expect("backdrop fill");
    backdrop.show();
    pump_for(Duration::from_millis(150));

    let region = ScreenRegion::new(bounds);
    region.refresh();
    let backdrop_white = region.pixel_rgb(bounds.width() / 2, bounds.height() / 2);

    let mut overlay = OverlayWindow::new(device, bounds).expect("overlay window");
    overlay
        .fill(Rgba8::new(255, 0, 0, 128))
        .expect("overlay fill");
    overlay.show();
    pump_for(Duration::from_millis(300));
    region.refresh();
    let blended = region.pixel_rgb(bounds.width() / 2, bounds.height() / 2);

    // Drawing mode: the topmost overlay owns the routed mouse moves.
    let interactive_start_overlay = overlay.callback_counts().mouse_move;
    let interactive_start_backdrop = backdrop.callback_counts().mouse_move;
    move_cursor(PhysicalPoint::new(center.x - 30, center.y));
    pump_for(Duration::from_millis(40));
    move_cursor(center);
    pump_for(Duration::from_millis(150));
    let interactive_overlay_moves =
        overlay.callback_counts().mouse_move - interactive_start_overlay;
    let interactive_backdrop_moves =
        backdrop.callback_counts().mouse_move - interactive_start_backdrop;

    // Pointer mode: the same moves must fall through to the window below and
    // never reach the overlay itself.
    overlay
        .set_click_through(true)
        .expect("enable click-through");
    pump_for(Duration::from_millis(100));
    // The layered+DComp content must stay visible while input passes through.
    region.refresh();
    let blended_click_through = region.pixel_rgb(bounds.width() / 2, bounds.height() / 2);
    let click_through_start_overlay = overlay.callback_counts().mouse_move;
    let click_through_start_backdrop = backdrop.callback_counts().mouse_move;
    move_cursor(PhysicalPoint::new(center.x - 20, center.y));
    pump_for(Duration::from_millis(40));
    move_cursor(PhysicalPoint::new(center.x + 20, center.y + 8));
    pump_for(Duration::from_millis(150));
    let click_through_overlay_moves =
        overlay.callback_counts().mouse_move - click_through_start_overlay;
    let click_through_backdrop_moves =
        backdrop.callback_counts().mouse_move - click_through_start_backdrop;

    move_cursor(original_cursor);
    pump_for(Duration::from_millis(60));
    let overlay_hwnd = overlay.hwnd();

    OverlayEvidence {
        backdrop_white,
        blended,
        blended_click_through,
        overlay_hwnd,
        interactive_overlay_moves,
        interactive_backdrop_moves,
        click_through_overlay_moves,
        click_through_backdrop_moves,
    }
}

fn cursor_position() -> PhysicalPoint {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.expect("GetCursorPos");
    PhysicalPoint::new(point.x, point.y)
}

fn move_cursor(point: PhysicalPoint) {
    unsafe { SetCursorPos(point.x, point.y) }.expect("SetCursorPos");
}

struct AnimationEvidence {
    display_refresh_hz: f64,
    refresh_hz: f64,
    dwm_refresh_hz: Option<f64>,
    dwm_source: &'static str,
    dwm_intervals_ms: Vec<f64>,
    frames_observed: u64,
    frame_intervals_ms: Vec<f64>,
    timer_callbacks: u32,
    paint_callbacks: u32,
    custom_callbacks: u32,
    timer_meter_ok: bool,
    paint_meter_ok: bool,
    custom_meter_ok: bool,
    /// (elapsed milliseconds, leftmost visible pixel of the magenta block)
    positions: Vec<(f64, i32)>,
}

fn animation_evidence(device: &GraphicsDevice) -> AnimationEvidence {
    let duration = Duration::from_millis(600);
    let bounds = PhysicalRect::from_xywh(260, 240, 520, 140);
    let mut toolbar = ToolbarWindow::new(device, bounds).expect("toolbar window");
    toolbar
        .set_offset(PhysicalPoint::new(20, 20))
        .expect("initial visual offset");
    toolbar
        .fill(Rgba8::new(255, 0, 255, 255))
        .expect("toolbar fill");
    toolbar.show();
    pump_for(Duration::from_millis(200));

    let display_refresh_hz = display_refresh_hz();

    // Pass 1: tight-loop presentation timestamps. No screen captures run here
    // so the poller cannot skip compositor frames.
    let baseline = toolbar.callback_counts();
    toolbar
        .animate_to(PhysicalPoint::new(380, 20), duration, Easing::Linear)
        .expect("start compositor animation");

    let mut frame_times: Vec<i64> = Vec::new();
    let mut last_frame_time: Option<i64> = None;
    let mut refresh_hz = 0.0;
    let mut time_frequency = 0i64;
    let mut dwm_frame_times: Vec<i64> = Vec::new();
    // DWM keeps presentation statistics per composition target; a
    // WS_EX_NOREDIRECTIONBITMAP window may not have one, so fall back to the
    // desktop composition target and record which source was actually used.
    let (dwm_target, dwm_source, mut last_dwm_frame) = match dwm_timing(toolbar.hwnd()) {
        Ok(timing) => (toolbar.hwnd(), "window", Some(timing)),
        Err(_) => match dwm_timing(HWND::default()) {
            Ok(timing) => (HWND::default(), "desktop", Some(timing)),
            Err(_) => (HWND::default(), "unavailable", None),
        },
    };
    let mut dwm_refresh_hz = last_dwm_frame.and_then(|t| t.refresh_hz);
    let deadline = Instant::now() + duration + Duration::from_millis(150);
    while Instant::now() < deadline {
        if let Ok(stats) = device.frame_statistics() {
            refresh_hz = stats.current_refresh_hz;
            time_frequency = stats.time_frequency;
            if last_frame_time != Some(stats.last_frame_time) {
                last_frame_time = Some(stats.last_frame_time);
                frame_times.push(stats.last_frame_time);
            }
        }
        if let Ok(timing) = dwm_timing(dwm_target) {
            if let Some(previous) = last_dwm_frame
                && timing.displayed > previous.displayed
                && timing.qpc_displayed > 0
            {
                dwm_frame_times.push(timing.qpc_displayed);
                if timing.refresh_hz.is_some() {
                    dwm_refresh_hz = timing.refresh_hz;
                }
            }
            last_dwm_frame = Some(timing);
        }
        pump_pending();
    }
    let after_animation = toolbar.callback_counts();

    let mut frame_intervals_ms = Vec::new();
    if time_frequency > 0 {
        for pair in frame_times.windows(2).skip(1) {
            let delta_ms = (pair[1] - pair[0]) as f64 * 1000.0 / time_frequency as f64;
            if (0.1..50.0).contains(&delta_ms) {
                frame_intervals_ms.push(delta_ms);
            }
        }
    }
    let mut dwm_intervals_ms = Vec::new();
    if let Some(frequency) = qpc_frequency() {
        dwm_intervals_ms = dwm_frame_times
            .windows(2)
            .skip(1)
            .map(|pair| (pair[1] - pair[0]) as f64 * 1000.0 / frequency as f64)
            .filter(|ms| (0.1..50.0).contains(ms))
            .collect();
        if !dwm_intervals_ms.is_empty() {
            dwm_refresh_hz = Some(1000.0 / percentile(&dwm_intervals_ms, 0.50));
        }
    }

    // Pass 2: visible screen movement. BitBlt sampling runs at ~15ms so it
    // cannot resolve 240Hz; it only proves the animation is visibly present.
    toolbar
        .set_offset(PhysicalPoint::new(20, 20))
        .expect("reset visual offset");
    pump_for(Duration::from_millis(100));
    let region = ScreenRegion::new(bounds);
    toolbar
        .animate_to(PhysicalPoint::new(380, 20), duration, Easing::Linear)
        .expect("start visible animation");
    let visibility_start = Instant::now();
    let mut positions = Vec::new();
    let mut next_capture = visibility_start;
    let visibility_deadline = visibility_start + duration + Duration::from_millis(200);
    while Instant::now() < visibility_deadline {
        if Instant::now() >= next_capture {
            region.refresh();
            if let Some(left) = region.find_block_left() {
                positions.push((visibility_start.elapsed().as_secs_f64() * 1000.0, left));
            }
            next_capture += Duration::from_millis(15);
        }
        pump_pending();
    }
    let after_visible = toolbar.callback_counts();

    let timer_callbacks = after_visible.timer - baseline.timer;
    let paint_callbacks = after_visible.paint - baseline.paint;
    let custom_callbacks = after_visible.custom - baseline.custom;
    debug_assert!(after_animation.timer <= after_visible.timer);

    // Prove the meter is live: a real window timer, a real posted custom
    // message and a real paint invalidation must all be counted.
    let before_self_check = toolbar.callback_counts();
    let timer_id = unsafe { SetTimer(Some(toolbar.hwnd()), 1, 10, None) };
    assert_ne!(timer_id, 0, "SetTimer for the callback meter failed");
    pump_for(Duration::from_millis(80));
    let _ = unsafe { KillTimer(Some(toolbar.hwnd()), 1) };
    let after_timer = toolbar.callback_counts();
    let timer_meter_ok = after_timer.timer > before_self_check.timer;
    unsafe {
        PostMessageW(
            Some(toolbar.hwnd()),
            APP_CALLBACK_MESSAGE,
            WPARAM(0),
            LPARAM(0),
        )
        .expect("post custom callback message");
    }
    pump_for(Duration::from_millis(30));
    let after_custom = toolbar.callback_counts();
    let custom_meter_ok = after_custom.custom > after_timer.custom;
    unsafe {
        RedrawWindow(
            Some(toolbar.hwnd()),
            None,
            None,
            RDW_INVALIDATE | RDW_UPDATENOW,
        )
        .expect("invalidate toolbar window");
    }
    pump_pending();
    let after_paint = toolbar.callback_counts();
    let paint_meter_ok = after_paint.paint > after_custom.paint;

    AnimationEvidence {
        display_refresh_hz,
        refresh_hz,
        dwm_refresh_hz,
        dwm_source,
        dwm_intervals_ms,
        frames_observed: frame_times.len() as u64,
        frame_intervals_ms,
        timer_callbacks,
        paint_callbacks,
        custom_callbacks,
        timer_meter_ok,
        paint_meter_ok,
        custom_meter_ok,
        positions,
    }
}

struct CpuEvidence {
    cpu_seconds: f64,
    wall_seconds: f64,
    percent_of_one_core: f64,
}

/// Measures application CPU time while a DComp animation runs. The loop below
/// deliberately does not poll anything, so the number describes the shipping
/// animation path rather than the probe itself.
fn cpu_evidence(device: &GraphicsDevice) -> CpuEvidence {
    let bounds = PhysicalRect::from_xywh(300, 240, 400, 120);
    let mut toolbar = ToolbarWindow::new(device, bounds).expect("cpu toolbar");
    toolbar
        .fill(Rgba8::new(30, 160, 90, 255))
        .expect("cpu fill");
    toolbar.show();
    pump_for(Duration::from_millis(150));

    let cpu_before = process_cpu_time();
    let wall_start = Instant::now();
    toolbar
        .animate_to(
            PhysicalPoint::new(200, 0),
            Duration::from_millis(1500),
            Easing::EaseInOutCubic,
        )
        .expect("cpu animation");
    while wall_start.elapsed() < Duration::from_millis(1500) {
        pump_pending();
        std::thread::sleep(Duration::from_millis(2));
    }
    let cpu_seconds = (process_cpu_time() - cpu_before).as_secs_f64();
    let wall_seconds = wall_start.elapsed().as_secs_f64();
    CpuEvidence {
        cpu_seconds,
        wall_seconds,
        percent_of_one_core: cpu_seconds / wall_seconds * 100.0,
    }
}

struct LeakEvidence {
    /// [baseline after warmup, after batch 1, after batch 2, after batch 3 of
    /// 100 create/destroy cycles each]
    batches: Vec<MemorySnapshot>,
}

fn leak_cycle_evidence(device: &GraphicsDevice) -> LeakEvidence {
    for _ in 0..20 {
        window_cycle(device);
    }
    pump_for(Duration::from_millis(200));
    let mut batches = vec![memory_snapshot()];
    for _ in 0..3 {
        for _ in 0..100 {
            window_cycle(device);
        }
        pump_for(Duration::from_millis(200));
        batches.push(memory_snapshot());
    }
    LeakEvidence { batches }
}

fn window_cycle(device: &GraphicsDevice) {
    let bounds = PhysicalRect::from_xywh(80, 80, 200, 120);
    {
        let mut overlay = OverlayWindow::new(device, bounds).expect("cycle overlay");
        overlay
            .fill(Rgba8::new(0, 0, 255, 120))
            .expect("cycle overlay fill");
        overlay
            .set_click_through(true)
            .expect("cycle click-through");
        overlay.show();
        pump_pending();
        let mut toolbar = ToolbarWindow::new(device, bounds).expect("cycle toolbar");
        toolbar
            .fill(Rgba8::new(255, 165, 0, 255))
            .expect("cycle toolbar fill");
        toolbar.show();
        pump_pending();
        drop(toolbar);
        drop(overlay);
    }
    pump_pending();
}

// ---------------------------------------------------------------------------
// Small measurement utilities
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct MemorySnapshot {
    private_bytes: usize,
    working_set: usize,
    handles: u32,
    gdi_objects: u32,
    user_objects: u32,
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
    let mut handles = 0u32;
    unsafe { GetProcessHandleCount(process, &mut handles).expect("GetProcessHandleCount") };
    MemorySnapshot {
        private_bytes: counters.PrivateUsage,
        working_set: counters.WorkingSetSize,
        handles,
        gdi_objects: unsafe { GetGuiResources(process, GR_GDIOBJECTS) },
        user_objects: unsafe { GetGuiResources(process, GR_USEROBJECTS) },
    }
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

#[derive(Clone, Copy)]
struct DwmTiming {
    displayed: u64,
    qpc_displayed: i64,
    refresh_hz: Option<f64>,
}

fn dwm_timing(hwnd: HWND) -> Result<DwmTiming, windows::core::Error> {
    let mut info = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    unsafe { DwmGetCompositionTimingInfo(hwnd, &mut info)? };
    let refresh_hz = if info.rateRefresh.uiDenominator != 0 {
        Some(info.rateRefresh.uiNumerator as f64 / info.rateRefresh.uiDenominator as f64)
    } else {
        None
    };
    Ok(DwmTiming {
        displayed: info.cFramesDisplayed,
        qpc_displayed: info.qpcFrameDisplayed as i64,
        refresh_hz,
    })
}

fn qpc_frequency() -> Option<i64> {
    let mut frequency = 0i64;
    unsafe { QueryPerformanceFrequency(&mut frequency).ok()? };
    if frequency > 0 { Some(frequency) } else { None }
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

/// A reusable screen readback: BitBlt from the composited desktop into a
/// top-down 32bpp DIB. Only used by the gate tests.
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

    /// Returns the median x of the left edge of the opaque magenta test block,
    /// scanning rows with a minimum contiguous run so isolated magenta desktop
    /// pixels cannot masquerade as the block.
    fn find_block_left(&self) -> Option<i32> {
        const MIN_RUN: i32 = 30;
        let mut left_edges: Vec<i32> = Vec::new();
        for y in (0..self.height).step_by(4) {
            let mut run_start: Option<i32> = None;
            for x in 0..self.width {
                let [r, g, b] = self.pixel_rgb(x, y);
                let magenta = r > 230 && b > 230 && g < 70;
                match (magenta, run_start) {
                    (true, None) => run_start = Some(x),
                    (false, Some(start)) => {
                        if x - start >= MIN_RUN {
                            left_edges.push(start);
                            run_start = None;
                            break;
                        }
                        run_start = None;
                    }
                    _ => {}
                }
            }
            if let Some(start) = run_start
                && self.width - start >= MIN_RUN
            {
                left_edges.push(start);
            }
        }
        if left_edges.is_empty() {
            return None;
        }
        left_edges.sort_unstable();
        Some(left_edges[left_edges.len() / 2])
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
