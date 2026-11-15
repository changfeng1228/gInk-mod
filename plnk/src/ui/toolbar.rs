//! Native toolbar: model, legacy layout, hit testing, icons and compositor
//! animation.
//!
//! Layout is computed in DIPs (plan §6.3: DIP only for toolbar layout) and
//! converted to physical pixels only for window geometry and drawing. The
//! button order, sizes, group gaps and visibility rules are copied from the
//! legacy `FormCollection` constructor:
//!
//! * dock chevron at the panel's left edge (`0.75 * button` wide),
//! * enabled pens in index order starting at `1.2 * button`, consecutive
//!   buttons advancing `1.1 * button`,
//! * a `0.8 * button` gap, then eraser / pan / pointer,
//! * a `0.8 * button` gap, then pen width / ink visible / snapshot / undo /
//!   clear,
//! * a `0.8 * button` gap, then the exit button; panel width is exit right plus
//!   half a button.
//!
//! The legacy program animated the toolbar panel inside a full-screen form with
//! a 4 ms timer. Here the window is only the toolbar (plan §6.1) and
//! DirectComposition animates the visual offset; the application only sets
//! start, end, duration and easing. A single one-shot timer per visible
//! animation switches the window between the full-panel frame and the dock
//! strip frame once the compositor has arrived, so the transparent parts of a
//! window never keep swallowing desktop input.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HWND_TOPMOST, PostMessageW, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetTimer, SetWindowPos, ShowWindow,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_TIMER,
};
use windows::core::w;

use crate::gfx::composition::WindowComposition;
use crate::gfx::device::{GfxError, GraphicsDevice, failed};
use crate::platform::dpi::{DipPoint, DipRect, DpiMap, PhysicalPoint, PhysicalRect};
use crate::platform::window::{CallbackCounts, MonotonicClock, NativeWindow};
use crate::settings::{MAX_PEN_COUNT, Settings};
use crate::ui::animation::{SlidePlan, ToolbarAnimator};
use crate::ui::icons::{Canvas, draw_icon, icon_for};

/// Number of configurable pen buttons (legacy `Root.MaxPenCount`).
pub const MAX_PEN_BUTTONS: usize = MAX_PEN_COUNT;

/// Legacy drag threshold in physical pixels (`FormCollection` checks > 20).
const DRAG_THRESHOLD: i32 = 20;
/// Legacy 15 px bottom margin for the default position.
const DEFAULT_BOTTOM_MARGIN: i32 = 15;
/// Timer id of the animation-end settle action.
const SETTLE_TIMER_ID: usize = 0x5442; // "TB"
/// Extra delay over the animation duration before the one-shot settle timer.
const SETTLE_MARGIN_MS: u32 = 20;
const PAPER: crate::settings::Rgb = crate::settings::Rgb::new(245, 245, 245);
const PEN_BORDER_PX: f32 = 3.0;
const PEN_UNDERLINE_TOP: f32 = 0.84;
const PEN_UNDERLINE_BOTTOM: f32 = 0.88;

/// A toolbar button, in legacy visual order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ButtonId {
    Dock,
    Pen(u8),
    Eraser,
    Pan,
    Pointer,
    PenWidth,
    InkVisible,
    Snapshot,
    Undo,
    Clear,
    Exit,
}

impl ButtonId {
    /// Stable numeric encoding for the click-notification window message.
    /// Pens occupy `0x100 + index` so the ten slots cannot collide with the
    /// fixed action buttons.
    pub fn code(self) -> u32 {
        match self {
            Self::Dock => 1,
            Self::Eraser => 2,
            Self::Pan => 3,
            Self::Pointer => 4,
            Self::PenWidth => 5,
            Self::InkVisible => 6,
            Self::Snapshot => 7,
            Self::Undo => 8,
            Self::Clear => 9,
            Self::Exit => 10,
            Self::Pen(index) => 0x100 + u32::from(index),
        }
    }

    /// Inverse of [`Self::code`].
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            1 => Self::Dock,
            2 => Self::Eraser,
            3 => Self::Pan,
            4 => Self::Pointer,
            5 => Self::PenWidth,
            6 => Self::InkVisible,
            7 => Self::Snapshot,
            8 => Self::Undo,
            9 => Self::Clear,
            10 => Self::Exit,
            code if (0x100..0x100 + MAX_PEN_BUTTONS as u32).contains(&code) => {
                Self::Pen((code - 0x100) as u8)
            }
            _ => return None,
        })
    }
}

/// Pen appearance needed to draw the toolbar button (legacy `PenAttr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PenSwatch {
    pub color: crate::settings::Rgb,
    /// 0-255 alpha; values <= 155 select the legacy highlighter bitmap
    /// (`Transparency >= 100`).
    pub alpha: u8,
}

/// Everything the toolbar displays.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarModel {
    buttons: Vec<ButtonId>,
    pens: [PenSwatch; MAX_PEN_BUTTONS],
    active: Option<ButtonId>,
    ink_visible: bool,
}

impl ToolbarModel {
    /// The enabled buttons in legacy visual order.
    ///
    /// Only buttons that exist in the legacy toolbar are produced: pens in
    /// index order, eraser, pan, pointer, pen width, ink visible, snapshot,
    /// undo, clear and the always-present exit button. The legacy program has
    /// no toolbar buttons for redo, pinning or options (those are hotkeys and
    /// tray-menu entries), so none are invented here.
    pub fn visible_buttons(settings: &Settings) -> Vec<ButtonId> {
        let mut buttons = Vec::with_capacity(MAX_PEN_BUTTONS + 10);
        buttons.push(ButtonId::Dock);
        for (index, pen) in settings.pens.iter().enumerate() {
            if pen.enabled {
                buttons.push(ButtonId::Pen(index as u8));
            }
        }
        if settings.eraser_icon {
            buttons.push(ButtonId::Eraser);
        }
        if settings.pan_icon {
            buttons.push(ButtonId::Pan);
        }
        if settings.pointer_icon {
            buttons.push(ButtonId::Pointer);
        }
        if settings.pen_width_icon {
            buttons.push(ButtonId::PenWidth);
        }
        if settings.ink_visible_icon {
            buttons.push(ButtonId::InkVisible);
        }
        if settings.snapshot_icon {
            buttons.push(ButtonId::Snapshot);
        }
        if settings.undo_icon {
            buttons.push(ButtonId::Undo);
        }
        if settings.clear_icon {
            buttons.push(ButtonId::Clear);
        }
        buttons.push(ButtonId::Exit);
        buttons
    }

    /// Builds the model from settings, reproducing the legacy initial selection:
    /// pen 1 when enabled, otherwise the first enabled pen, otherwise the
    /// pointer tool; ink starts visible.
    pub fn from_settings(settings: &Settings) -> Self {
        let mut pens = [PenSwatch {
            color: crate::settings::Rgb::new(0, 0, 0),
            alpha: 255,
        }; MAX_PEN_BUTTONS];
        for (slot, pen) in pens.iter_mut().zip(settings.pens.iter()) {
            *slot = PenSwatch {
                color: pen.color,
                alpha: pen.alpha,
            };
        }
        let active = if settings.pens[1].enabled {
            Some(ButtonId::Pen(1))
        } else {
            settings
                .pens
                .iter()
                .position(|pen| pen.enabled)
                .map(|index| ButtonId::Pen(index as u8))
                .or(Some(ButtonId::Pointer))
        };
        Self {
            buttons: Self::visible_buttons(settings),
            pens,
            active,
            ink_visible: true,
        }
    }

    pub fn buttons(&self) -> &[ButtonId] {
        &self.buttons
    }

    pub fn pen(&self, index: usize) -> Option<PenSwatch> {
        self.pens.get(index).copied()
    }

    pub fn active(&self) -> Option<ButtonId> {
        self.active
    }

    pub fn set_active(&mut self, active: Option<ButtonId>) {
        self.active = active;
    }

    pub fn ink_visible(&self) -> bool {
        self.ink_visible
    }

    pub fn set_ink_visible(&mut self, visible: bool) {
        self.ink_visible = visible;
    }
}

/// DIP-only button layout: the legacy `FormCollection` geometry.
#[derive(Clone, Debug)]
pub struct ToolbarLayout {
    height: f32,
    dock_width: f32,
    panel_width: f32,
    placements: Vec<(ButtonId, DipRect)>,
    docked: bool,
}

impl ToolbarLayout {
    /// Lays out `model` for a toolbar `height_dip`. `docked` selects the
    /// presentation: the settled docked window shows only the dock strip.
    pub fn new(model: &ToolbarModel, height_dip: f32, docked: bool) -> Self {
        let height = if height_dip.is_finite() && height_dip > 0.0 {
            height_dip
        } else {
            1.0
        };
        let button = 0.88 * height;
        let top = 0.07 * height;
        let pen_top = 0.08 * height;
        let dock_width = 0.75 * button;
        let mut placements = Vec::with_capacity(model.buttons().len());

        placements.push((
            ButtonId::Dock,
            DipRect::new(0.0, top, dock_width, top + button),
        ));
        // The legacy code places pens 1.2 button widths in, leaving room for
        // the dock button, and advances 1.1 button widths per visible pen.
        let mut x = 1.2 * button;
        let buttons = model.buttons();
        for index in 0..MAX_PEN_BUTTONS {
            let id = ButtonId::Pen(index as u8);
            if buttons.contains(&id) {
                placements.push((id, DipRect::new(x, pen_top, x + button, pen_top + button)));
                x += 1.1 * button;
            }
        }
        x += 0.8 * button;
        for id in [ButtonId::Eraser, ButtonId::Pan, ButtonId::Pointer] {
            if buttons.contains(&id) {
                placements.push((id, DipRect::new(x, top, x + button, top + button)));
                x += 1.1 * button;
            }
        }
        x += 0.8 * button;
        for id in [
            ButtonId::PenWidth,
            ButtonId::InkVisible,
            ButtonId::Snapshot,
            ButtonId::Undo,
            ButtonId::Clear,
        ] {
            if buttons.contains(&id) {
                placements.push((id, DipRect::new(x, top, x + button, top + button)));
                x += 1.1 * button;
            }
        }
        x += 0.8 * button;
        placements.push((
            ButtonId::Exit,
            DipRect::new(x, top, x + button, top + button),
        ));
        let panel_width = x + button + 0.5 * button;

        Self {
            height,
            dock_width,
            panel_width,
            placements,
            docked,
        }
    }

    /// Size of the window that presents this layout, in DIP.
    pub fn size(&self) -> (f32, f32) {
        if self.docked {
            (self.dock_width, self.height)
        } else {
            (self.panel_width, self.height)
        }
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn panel_width(&self) -> f32 {
        self.panel_width
    }

    pub fn dock_width(&self) -> f32 {
        self.dock_width
    }

    pub fn docked(&self) -> bool {
        self.docked
    }

    /// All placements in full-panel DIP coordinates.
    pub fn placements(&self) -> &[(ButtonId, DipRect)] {
        &self.placements
    }

    /// Button rectangle in window-local DIP coordinates.
    pub fn button_rect(&self, id: ButtonId) -> Option<DipRect> {
        if self.docked && id != ButtonId::Dock {
            return None;
        }
        self.placements
            .iter()
            .find(|(button, _)| *button == id)
            .map(|(_, rect)| *rect)
    }

    /// Hit tests a window-local DIP point.
    pub fn hit_test(&self, point: DipPoint) -> Option<ButtonId> {
        if self.docked {
            let dock = self.button_rect(ButtonId::Dock)?;
            return dip_rect_contains(dock, point).then_some(ButtonId::Dock);
        }
        self.placements
            .iter()
            .find(|(_, rect)| dip_rect_contains(*rect, point))
            .map(|(id, _)| *id)
    }

    /// Physical window rectangle for the given panel origin and DPI.
    pub fn window_rect(&self, panel_origin: PhysicalPoint, dpi: DpiMap) -> PhysicalRect {
        let scale = dpi.scale();
        let panel_width = (self.panel_width * scale).round() as i32;
        let height = (self.height * scale).round() as i32;
        if self.docked {
            let dock_width = (self.dock_width * scale).round() as i32;
            PhysicalRect::from_xywh(
                panel_origin.x + panel_width - dock_width,
                panel_origin.y,
                dock_width,
                height,
            )
        } else {
            PhysicalRect::from_xywh(panel_origin.x, panel_origin.y, panel_width, height)
        }
    }

    /// Hit tests a screen point against a physical window rectangle.
    pub fn hit_test_physical(
        &self,
        point: PhysicalPoint,
        window: PhysicalRect,
        dpi: DpiMap,
    ) -> Option<ButtonId> {
        if !rect_contains(window, point) {
            return None;
        }
        let local = PhysicalPoint::new(point.x - window.left, point.y - window.top);
        self.hit_test(dpi.physical_to_dip(local))
    }
}

/// Result of one pointer gesture on the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarEvent {
    None,
    /// A button click that survived the legacy drag-threshold suppression.
    Click(ButtonId),
    /// The toolbar was dragged; the release must not become a click.
    Dragged,
}

#[derive(Clone, Copy, Debug)]
struct PressState {
    button: Option<ButtonId>,
    point: PhysicalPoint,
    origin: PhysicalPoint,
    allow_drag: bool,
    dragging: bool,
}

/// Hit-test / drag state machine for the toolbar window.
///
/// Mirrors the legacy `ToolbarMoved` behaviour: dragging begins after more than
/// 20 physical pixels of movement, cannot start while the toolbar is sliding,
/// and a drag suppresses the click that would otherwise fire on release.
#[derive(Debug, Default)]
pub struct ToolbarInteraction {
    press: Option<PressState>,
}

impl ToolbarInteraction {
    pub fn begin(
        &mut self,
        button: Option<ButtonId>,
        point: PhysicalPoint,
        panel_origin: PhysicalPoint,
        allow_drag: bool,
    ) {
        self.press = Some(PressState {
            button,
            point,
            origin: panel_origin,
            allow_drag,
            dragging: false,
        });
    }

    pub fn is_dragging(&self) -> bool {
        self.press.is_some_and(|press| press.dragging)
    }

    /// Returns the desired clamped panel origin once the drag threshold crossed.
    pub fn drag_to(
        &mut self,
        point: PhysicalPoint,
        bounds: PhysicalRect,
        panel_size: (i32, i32),
    ) -> Option<PhysicalPoint> {
        let press = self.press.as_mut()?;
        if !press.allow_drag {
            return None;
        }
        let dx = point.x - press.point.x;
        let dy = point.y - press.point.y;
        if !press.dragging && (dx.abs() > DRAG_THRESHOLD || dy.abs() > DRAG_THRESHOLD) {
            press.dragging = true;
        }
        if !press.dragging {
            return None;
        }
        let desired = PhysicalPoint::new(press.origin.x + dx, press.origin.y + dy);
        Some(clamp_origin(desired, bounds, panel_size))
    }

    pub fn end(&mut self, button_under_pointer: Option<ButtonId>) -> ToolbarEvent {
        let Some(press) = self.press.take() else {
            return ToolbarEvent::None;
        };
        if press.dragging {
            return ToolbarEvent::Dragged;
        }
        match (press.button, button_under_pointer) {
            (Some(pressed), Some(released)) if pressed == released => ToolbarEvent::Click(pressed),
            _ => ToolbarEvent::None,
        }
    }
}

fn clamp_origin(
    desired: PhysicalPoint,
    bounds: PhysicalRect,
    panel_size: (i32, i32),
) -> PhysicalPoint {
    let max_x = (bounds.right - panel_size.0).max(bounds.left);
    let max_y = (bounds.bottom - panel_size.1).max(bounds.top);
    PhysicalPoint::new(
        desired.x.clamp(bounds.left, max_x),
        desired.y.clamp(bounds.top, max_y),
    )
}

fn rect_contains(rect: PhysicalRect, point: PhysicalPoint) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn dip_rect_contains(rect: DipRect, point: DipPoint) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

struct MonitorGeometry {
    bounds: PhysicalRect,
    work: PhysicalRect,
}

fn monitor_geometry(reference: PhysicalPoint) -> MonitorGeometry {
    let point = POINT {
        x: reference.x,
        y: reference.y,
    };
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return MonitorGeometry {
            bounds: PhysicalRect::new(
                info.rcMonitor.left,
                info.rcMonitor.top,
                info.rcMonitor.right,
                info.rcMonitor.bottom,
            ),
            work: PhysicalRect::new(
                info.rcWork.left,
                info.rcWork.top,
                info.rcWork.right,
                info.rcWork.bottom,
            ),
        };
    }
    // GetMonitorInfoW only fails for an invalid monitor handle; keep the
    // toolbar usable with the primary screen metrics instead of failing.
    let bounds = PhysicalRect::from_xywh(
        0,
        0,
        unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1),
        unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(1),
    );
    MonitorGeometry {
        bounds,
        work: bounds,
    }
}

fn toolbar_height_physical(settings: &Settings, monitor_height: i32) -> i32 {
    let fraction = settings.toolbar_height;
    let fraction = if fraction.is_finite() && fraction > 0.0 {
        fraction
    } else {
        0.06
    };
    ((monitor_height as f32) * fraction)
        .round()
        .clamp(1.0, monitor_height.max(1) as f32) as i32
}

fn default_panel_origin(
    settings: &Settings,
    monitor: &MonitorGeometry,
    panel_size: (i32, i32),
) -> PhysicalPoint {
    let candidate = PhysicalPoint::new(settings.toolbar_left, settings.toolbar_top);
    let rect = PhysicalRect::from_xywh(candidate.x, candidate.y, panel_size.0, panel_size.1);
    let inside = rect_contains(monitor.bounds, PhysicalPoint::new(rect.left, rect.top))
        && rect_contains(monitor.bounds, PhysicalPoint::new(rect.right - 1, rect.top))
        && rect_contains(
            monitor.bounds,
            PhysicalPoint::new(rect.left, rect.bottom - 1),
        )
        && rect_contains(
            monitor.bounds,
            PhysicalPoint::new(rect.right - 1, rect.bottom - 1),
        );
    if (candidate.x == 0 && candidate.y == 0) || !inside {
        PhysicalPoint::new(
            monitor.work.right - panel_size.0,
            monitor.work.bottom - panel_size.1 - DEFAULT_BOTTOM_MARGIN,
        )
    } else {
        candidate
    }
}

/// The native toolbar window.
pub struct ToolbarWindow {
    // Boxed so the pointer registered with the window procedure is stable
    // while the window object moves.
    ui: Box<ToolbarUi>,
    _not_send: PhantomData<Rc<()>>,
}

impl ToolbarWindow {
    /// Creates the toolbar at the configured/default position of the monitor
    /// under `Toolbar_Left/Top`.
    pub fn new(device: &GraphicsDevice, settings: &Settings) -> Result<Self, GfxError> {
        Self::create(device, settings, None)
    }

    /// Creates the toolbar at an explicit panel origin (tests and previews).
    pub fn new_at(
        device: &GraphicsDevice,
        settings: &Settings,
        panel_origin: PhysicalPoint,
    ) -> Result<Self, GfxError> {
        Self::create(device, settings, Some(panel_origin))
    }

    fn create(
        device: &GraphicsDevice,
        settings: &Settings,
        panel_origin: Option<PhysicalPoint>,
    ) -> Result<Self, GfxError> {
        let model = ToolbarModel::from_settings(settings);
        let provisional_origin = panel_origin.unwrap_or(PhysicalPoint::new(
            settings.toolbar_left,
            settings.toolbar_top,
        ));
        let monitor = monitor_geometry(provisional_origin);
        let placeholder_layout = ToolbarLayout::new(&model, 64.0, false);
        let mut ui = Box::new(ToolbarUi {
            composition: None,
            window: None,
            model,
            layout: placeholder_layout,
            dpi: DpiMap::from_dpi(96),
            height_dip: 64.0,
            panel_origin: provisional_origin,
            monitor,
            animator: ToolbarAnimator::new(PhysicalPoint::new(0, 0)),
            interaction: ToolbarInteraction::default(),
            allow_drag: settings.allow_dragging_toolbar,
            clicked: None,
            notify: None,
            docked: false,
            strip_view: false,
            visible: false,
            settle: Settle::None,
            last_error: None,
        });
        let ui_pointer = ui.as_mut() as *mut ToolbarUi as *mut c_void;
        let provisional =
            PhysicalRect::from_xywh(provisional_origin.x, provisional_origin.y, 64, 64);
        let window = NativeWindow::create_with_hook(
            provisional,
            w!("plnk toolbar"),
            false,
            Some(toolbar_hook),
            ui_pointer,
        )?;
        let hwnd = window.hwnd();
        ui.window = Some(window);
        ui.dpi = DpiMap::from_window(hwnd);
        let monitor = monitor_geometry(provisional_origin);
        let height = toolbar_height_physical(settings, monitor.bounds.height());
        ui.height_dip = height as f32 / ui.dpi.scale();
        ui.monitor = monitor;
        ui.layout = ToolbarLayout::new(&ui.model, ui.height_dip, false);
        if let Some(origin) = panel_origin {
            ui.panel_origin = origin;
        } else {
            let panel_size = ui.panel_size_px();
            ui.panel_origin = default_panel_origin(settings, &ui.monitor, panel_size);
        }
        let rect = ui.panel_rect();
        set_window_rect_raw(hwnd, rect)?;
        let composition =
            WindowComposition::new(device, hwnd, rect.width() as u32, rect.height() as u32)?;
        ui.composition = Some(composition);
        ui.render()?;
        Ok(Self {
            ui,
            _not_send: PhantomData,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.ui
            .window
            .as_ref()
            .map(NativeWindow::hwnd)
            .unwrap_or_default()
    }

    /// Replaces the displayed model and redraws immediately.
    pub fn set_model(&mut self, model: ToolbarModel) -> Result<(), GfxError> {
        self.ui.model = model;
        self.ui.layout = ToolbarLayout::new(&self.ui.model, self.ui.height_dip, self.ui.strip_view);
        let rect = if self.ui.strip_view {
            self.ui.strip_rect()
        } else {
            self.ui.panel_rect()
        };
        let ops = vec![
            Op::SetWindowRect(rect),
            Op::Resize(rect.width() as u32, rect.height() as u32),
            Op::Render,
        ];
        self.run(ops)
    }

    pub fn model(&self) -> &ToolbarModel {
        &self.ui.model
    }

    pub fn layout(&self) -> &ToolbarLayout {
        &self.ui.layout
    }

    pub fn is_docked(&self) -> bool {
        self.ui.docked
    }

    pub fn is_visible(&self) -> bool {
        self.ui.visible
    }

    pub fn is_animating(&self) -> bool {
        self.ui.animator.is_active(MonotonicClock::now())
    }

    pub fn panel_origin(&self) -> PhysicalPoint {
        self.ui.panel_origin
    }

    pub fn panel_rect(&self) -> PhysicalRect {
        self.ui.panel_rect()
    }

    pub fn dock_strip_rect(&self) -> PhysicalRect {
        self.ui.strip_rect()
    }

    pub fn window_rect(&self) -> Result<PhysicalRect, GfxError> {
        self.ui
            .window
            .as_ref()
            .map(NativeWindow::window_rect)
            .unwrap_or(Ok(self.ui.panel_rect()))
    }

    /// Screen rectangle of a button in the current settled presentation.
    pub fn button_screen_rect(&self, id: ButtonId) -> Option<PhysicalRect> {
        let rect = self.ui.layout.button_rect(id)?;
        let window = self.window_rect().ok()?;
        let scale = self.ui.dpi.scale();
        Some(PhysicalRect::from_xywh(
            window.left + (rect.left * scale).round() as i32,
            window.top + (rect.top * scale).round() as i32,
            ((rect.right - rect.left) * scale).round() as i32,
            ((rect.bottom - rect.top) * scale).round() as i32,
        ))
    }

    pub fn hit_test_screen(&self, point: PhysicalPoint) -> Option<ButtonId> {
        self.ui.hit_test_screen(point)
    }

    /// Takes the last stubbed button click; the application shell consumes the
    /// same click through [`Self::set_click_notifier`].
    pub fn take_clicked(&mut self) -> Option<ButtonId> {
        self.ui.clicked.take()
    }

    /// Posts a message to `target` (with the clicked [`ButtonId::code`] in
    /// `wparam`) whenever a button click survives the drag threshold.
    ///
    /// The toolbar window's own messages are dispatched inside its hook, where
    /// the shell cannot safely be borrowed; a posted notification lets the
    /// application handle pen selection, undo, clear and exit outside that
    /// borrow, without a polling timer.
    pub fn set_click_notifier(&mut self, target: HWND, message: u32) {
        self.ui.notify = Some((target, message));
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.ui.last_error.take()
    }

    /// Slides the toolbar in (300 ms, legacy `ButtonsEntering = 1`).
    pub fn enter(&mut self) -> Result<(), GfxError> {
        let ops = self.ui.op_enter();
        self.run(ops)
    }

    /// Slides the toolbar out and hides it (160 ms, legacy `-9`, Esc).
    pub fn fast_exit(&mut self) -> Result<(), GfxError> {
        let ops = self.ui.op_fast_exit();
        self.run(ops)
    }

    /// Docks or undocks with the 300 ms legacy slide.
    pub fn dock_to(&mut self, docked: bool) -> Result<(), GfxError> {
        let ops = self.ui.op_dock_to(docked);
        self.run(ops)
    }

    /// Runs a pending animation-end action immediately. Production runs it from
    /// the window's one-shot timer; tests and shutdown use this directly.
    pub fn settle_now(&mut self) -> Result<(), GfxError> {
        let ops = self.ui.op_settle();
        self.run(ops)
    }

    pub fn callback_counts(&self) -> CallbackCounts {
        self.ui
            .window
            .as_ref()
            .map(NativeWindow::callback_counts)
            .unwrap_or_default()
    }

    fn run(&mut self, ops: Vec<Op>) -> Result<(), GfxError> {
        let pointer = &mut *self.ui as *mut ToolbarUi;
        let hwnd = self.hwnd();
        // `run_ops` never holds a `&mut ToolbarUi` across a call that can
        // re-enter the window procedure (SetWindowPos / ShowWindow), so the
        // re-entrant hook never aliases this box.
        run_ops(hwnd, pointer, ops);
        match self.ui.last_error.take() {
            Some(message) => Err(GfxError::Operation {
                context: "toolbar",
                source: windows::core::Error::new(windows::Win32::Foundation::E_FAIL, message),
            }),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Settle {
    None,
    /// Hide the window after a fast exit.
    Hide,
    /// Switch the window to the dock strip after a dock slide.
    DockStrip,
}

/// Operations produced by state changes and executed without holding a borrow
/// of the UI state across re-entrant Win32 calls.
enum Op {
    SetWindowRect(PhysicalRect),
    Resize(u32, u32),
    ShowWindow,
    HideWindow,
    SetVisualOffset(PhysicalPoint),
    Animate(SlidePlan),
    Render,
    ArmSettle(Duration),
    CancelSettle,
    SetCapture,
    ReleaseCapture,
}

struct ToolbarUi {
    // Declared before `window`: the composition target must be released before
    // the HWND is destroyed.
    composition: Option<WindowComposition>,
    window: Option<NativeWindow>,
    model: ToolbarModel,
    layout: ToolbarLayout,
    dpi: DpiMap,
    height_dip: f32,
    panel_origin: PhysicalPoint,
    monitor: MonitorGeometry,
    animator: ToolbarAnimator,
    interaction: ToolbarInteraction,
    /// Legacy `Allow_Dragging_Toolbar`: when false the toolbar ignores drags
    /// (the buttons still click).
    allow_drag: bool,
    clicked: Option<ButtonId>,
    /// Where a click is announced (`HWND` as integer plus message id); set by
    /// the application shell, `None` in tests and previews.
    notify: Option<(HWND, u32)>,
    /// Logical docked state (also selects the dock icon).
    docked: bool,
    /// Window presentation: true when only the dock strip is shown.
    strip_view: bool,
    visible: bool,
    settle: Settle,
    last_error: Option<String>,
}

impl ToolbarUi {
    fn panel_size_px(&self) -> (i32, i32) {
        let scale = self.dpi.scale();
        (
            (self.layout.panel_width() * scale).round() as i32,
            (self.layout.height() * scale).round() as i32,
        )
    }

    fn panel_rect(&self) -> PhysicalRect {
        let (width, height) = self.panel_size_px();
        PhysicalRect::from_xywh(self.panel_origin.x, self.panel_origin.y, width, height)
    }

    fn strip_rect(&self) -> PhysicalRect {
        let (panel_width, height) = self.panel_size_px();
        let dock_width = (self.layout.dock_width() * self.dpi.scale()).round() as i32;
        PhysicalRect::from_xywh(
            self.panel_origin.x + panel_width - dock_width,
            self.panel_origin.y,
            dock_width,
            height,
        )
    }

    fn panel_width_px(&self) -> i32 {
        self.panel_size_px().0
    }

    fn dock_width_px(&self) -> i32 {
        (self.layout.dock_width() * self.dpi.scale()).round() as i32
    }

    fn current_offset(&self, now: Duration) -> i32 {
        self.animator.position_at(now).x
    }

    fn render(&mut self) -> Result<(), GfxError> {
        let Self {
            composition,
            model,
            layout,
            dpi,
            docked,
            strip_view,
            ..
        } = self;
        let Some(composition) = composition.as_mut() else {
            return Ok(());
        };
        let scale = dpi.scale();
        let docked = *docked;
        let strip = *strip_view;
        composition.draw(|context, factory| {
            let canvas = Canvas::new(context, factory)?;
            render_view(&canvas, layout, model, docked, strip, scale)
        })
    }

    fn hit_test_screen(&self, point: PhysicalPoint) -> Option<ButtonId> {
        if !self.visible {
            return None;
        }
        let window = self.window.as_ref()?.window_rect().ok()?;
        if !rect_contains(window, point) {
            return None;
        }
        let now = MonotonicClock::now();
        let shift = if self.animator.is_active(now) {
            self.animator.position_at(now).x
        } else {
            0
        };
        let local = PhysicalPoint::new(point.x - window.left - shift, point.y - window.top);
        self.layout.hit_test(self.dpi.physical_to_dip(local))
    }

    fn op_enter(&mut self) -> Vec<Op> {
        let now = MonotonicClock::now();
        let mut ops = vec![Op::CancelSettle];
        self.settle = Settle::None;
        let target_strip = self.docked;
        if self.strip_view != target_strip {
            let hidden = if target_strip {
                self.dock_width_px()
            } else {
                self.panel_width_px()
            };
            let offset = if self.visible {
                self.preserved_offset(target_strip, now)
            } else {
                hidden
            };
            ops.extend(self.present(target_strip, offset));
        }
        if !self.visible {
            let hidden = if self.strip_view {
                self.dock_width_px()
            } else {
                self.panel_width_px()
            };
            self.animator.snap_to(PhysicalPoint::new(hidden, 0));
            ops.push(Op::SetVisualOffset(PhysicalPoint::new(hidden, 0)));
            self.visible = true;
            ops.push(Op::ShowWindow);
        }
        let plan = self.animator.enter(now, PhysicalPoint::new(0, 0));
        ops.push(Op::Animate(plan));
        ops
    }

    fn op_fast_exit(&mut self) -> Vec<Op> {
        let now = MonotonicClock::now();
        let mut ops = vec![Op::CancelSettle];
        self.settle = Settle::None;
        if !self.visible {
            return ops;
        }
        let hidden = if self.strip_view {
            self.dock_width_px()
        } else {
            self.panel_width_px()
        };
        let plan = self.animator.fast_exit(now, PhysicalPoint::new(hidden, 0));
        self.settle = Settle::Hide;
        ops.push(Op::Animate(plan));
        ops.push(Op::ArmSettle(plan.duration));
        ops
    }

    fn op_dock_to(&mut self, docked: bool) -> Vec<Op> {
        let now = MonotonicClock::now();
        let mut ops = vec![Op::CancelSettle];
        if self.docked == docked && !self.animator.is_active(now) && self.settle == Settle::None {
            return ops;
        }
        self.settle = Settle::None;
        let changed = self.docked != docked;
        self.docked = docked;
        if !self.visible {
            return ops;
        }
        if self.strip_view {
            let preserved =
                self.current_offset(now) + (self.panel_width_px() - self.dock_width_px());
            ops.extend(self.present(false, preserved));
        } else if changed {
            // The dock icon flips immediately like the legacy `btDock.Image`.
            ops.push(Op::Render);
        }
        let target = if docked {
            self.panel_width_px() - self.dock_width_px()
        } else {
            0
        };
        if self.current_offset(now) == target {
            if docked {
                ops.extend(self.present(true, 0));
            }
            return ops;
        }
        let plan = self.animator.dock_to(now, PhysicalPoint::new(target, 0));
        if docked {
            self.settle = Settle::DockStrip;
            ops.push(Op::ArmSettle(plan.duration));
        }
        ops.push(Op::Animate(plan));
        ops
    }

    fn op_settle(&mut self) -> Vec<Op> {
        let mut ops = vec![Op::CancelSettle];
        match std::mem::replace(&mut self.settle, Settle::None) {
            Settle::Hide => {
                let hidden = if self.strip_view {
                    self.dock_width_px()
                } else {
                    self.panel_width_px()
                };
                self.animator.snap_to(PhysicalPoint::new(hidden, 0));
                ops.push(Op::SetVisualOffset(PhysicalPoint::new(hidden, 0)));
                ops.push(Op::HideWindow);
                self.visible = false;
            }
            Settle::DockStrip => {
                ops.extend(self.present(true, 0));
            }
            Settle::None => {}
        }
        ops
    }

    /// Switches the window frame and redraws, anchoring `offset` in the new
    /// frame so the content does not jump on screen.
    fn present(&mut self, strip: bool, offset: i32) -> Vec<Op> {
        self.strip_view = strip;
        self.layout = ToolbarLayout::new(&self.model, self.height_dip, strip);
        self.animator.snap_to(PhysicalPoint::new(offset, 0));
        let rect = if strip {
            self.strip_rect()
        } else {
            self.panel_rect()
        };
        vec![
            Op::SetWindowRect(rect),
            Op::Resize(rect.width() as u32, rect.height() as u32),
            Op::SetVisualOffset(PhysicalPoint::new(offset, 0)),
            Op::Render,
        ]
    }

    /// Current visual offset expressed in the other window frame.
    fn preserved_offset(&self, target_strip: bool, now: Duration) -> i32 {
        let current = self.current_offset(now);
        let delta = self.panel_width_px() - self.dock_width_px();
        if target_strip {
            current - delta
        } else {
            current + delta
        }
    }

    fn on_message(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Vec<Op> {
        match message {
            WM_TIMER if wparam.0 == SETTLE_TIMER_ID => self.op_settle(),
            WM_LBUTTONDOWN => self.op_mouse_down(lparam),
            WM_LBUTTONUP => self.op_mouse_up(lparam),
            WM_MOUSEMOVE => self.op_mouse_move(lparam),
            _ => Vec::new(),
        }
    }

    fn client_point(&self, lparam: LPARAM) -> Option<PhysicalPoint> {
        let x = i32::from((lparam.0 & 0xFFFF) as u16 as i16);
        let y = i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16);
        let window = self.window.as_ref()?.window_rect().ok()?;
        Some(PhysicalPoint::new(window.left + x, window.top + y))
    }

    fn op_mouse_down(&mut self, lparam: LPARAM) -> Vec<Op> {
        let Some(point) = self.client_point(lparam) else {
            return Vec::new();
        };
        let button = self.hit_test_screen(point);
        // Legacy `gpButtons_MouseDown` refuses to drag when the option is off
        // and while the toolbar is still animating; the click itself still
        // works.
        let allow_drag = self.allow_drag && !self.animator.is_active(MonotonicClock::now());
        self.interaction
            .begin(button, point, self.panel_origin, allow_drag);
        vec![Op::SetCapture]
    }

    fn op_mouse_move(&mut self, lparam: LPARAM) -> Vec<Op> {
        let Some(point) = self.client_point(lparam) else {
            return Vec::new();
        };
        let panel = self.panel_size_px();
        let bounds = self.monitor.bounds;
        let Some(origin) = self.interaction.drag_to(point, bounds, panel) else {
            return Vec::new();
        };
        if origin == self.panel_origin {
            return Vec::new();
        }
        self.panel_origin = origin;
        let rect = if self.strip_view {
            self.strip_rect()
        } else {
            self.panel_rect()
        };
        vec![Op::SetWindowRect(rect)]
    }

    fn op_mouse_up(&mut self, lparam: LPARAM) -> Vec<Op> {
        let mut ops = vec![Op::ReleaseCapture];
        let Some(point) = self.client_point(lparam) else {
            self.interaction.end(None);
            return ops;
        };
        let button = self.hit_test_screen(point);
        match self.interaction.end(button) {
            ToolbarEvent::Click(id) => {
                self.clicked = Some(id);
                if let Some((target, message)) = self.notify {
                    let posted = unsafe {
                        PostMessageW(Some(target), message, WPARAM(id.code() as usize), LPARAM(0))
                    };
                    if posted.is_err() {
                        // The shell window is gone (shutdown race); the click
                        // is still available through `take_clicked`.
                    }
                }
                if id == ButtonId::Dock {
                    let docked = self.docked;
                    ops.extend(self.op_dock_to(!docked));
                }
            }
            ToolbarEvent::Dragged | ToolbarEvent::None => {}
        }
        ops
    }
}

unsafe fn toolbar_hook(
    data: *mut c_void,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<windows::Win32::Foundation::LRESULT> {
    match message {
        WM_TIMER | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE => {
            // SAFETY: `data` is the live Box<ToolbarUi> registered at window
            // creation; the window clears the hook before dropping it. The
            // borrow ends before any operation that can re-enter this
            // procedure (SetWindowPos / ShowWindow).
            // The borrow ends at `on_message`; `run_ops` never holds a
            // `&mut ToolbarUi` across a re-entrant Win32 call.
            let ops =
                unsafe { (&mut *(data as *mut ToolbarUi)).on_message(message, wparam, lparam) };
            run_ops(hwnd, data as *mut ToolbarUi, ops);
        }
        _ => {}
    }
    None
}

fn run_ops(hwnd: HWND, ui: *mut ToolbarUi, ops: Vec<Op>) {
    for op in ops {
        if hwnd.is_invalid() {
            return;
        }
        let result = match op {
            // These two can send messages back into the window procedure, so
            // they run without any live `&mut ToolbarUi`.
            Op::SetWindowRect(rect) => set_window_rect_raw(hwnd, rect),
            Op::ShowWindow => {
                // Re-assert the topmost band on every show: the toolbar must
                // stay above the ink overlay and any other topmost window that
                // was created later.
                match unsafe {
                    SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    )
                } {
                    Ok(()) => {
                        unsafe {
                            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        }
                        Ok(())
                    }
                    Err(error) => Err(failed("SetWindowPos(HWND_TOPMOST)")(error)),
                }
            }
            Op::HideWindow => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                Ok(())
            }
            Op::ArmSettle(duration) => {
                let millis = (duration.as_millis() as u64)
                    .saturating_add(u64::from(SETTLE_MARGIN_MS))
                    .min(u64::from(u32::MAX)) as u32;
                unsafe {
                    let _ = SetTimer(Some(hwnd), SETTLE_TIMER_ID, millis, None);
                }
                Ok(())
            }
            Op::CancelSettle => {
                let _ = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), SETTLE_TIMER_ID)
                };
                Ok(())
            }
            Op::SetCapture => {
                unsafe { SetCapture(hwnd) };
                Ok(())
            }
            Op::ReleaseCapture => unsafe { ReleaseCapture() }.map_err(failed("ReleaseCapture")),
            // GPU operations do not dispatch window messages; a short borrow is
            // aliasing-safe.
            Op::Resize(width, height) => {
                let ui = unsafe { &mut *ui };
                match ui.composition.as_mut() {
                    Some(composition) => composition.resize(width, height),
                    None => Ok(()),
                }
            }
            Op::SetVisualOffset(offset) => {
                let ui = unsafe { &mut *ui };
                match ui.composition.as_mut() {
                    Some(composition) => composition.set_offset(offset),
                    None => Ok(()),
                }
            }
            Op::Animate(plan) => {
                let ui = unsafe { &mut *ui };
                match ui.composition.as_mut() {
                    Some(composition) => {
                        composition.animate_offset(plan.from, plan.to, plan.duration, plan.easing)
                    }
                    None => Ok(()),
                }
            }
            Op::Render => {
                let ui = unsafe { &mut *ui };
                ui.render()
            }
        };
        if let Err(error) = result {
            let ui = unsafe { &mut *ui };
            ui.last_error = Some(error.to_string());
            return;
        }
    }
}

fn set_window_rect_raw(hwnd: HWND, rect: PhysicalRect) -> Result<(), GfxError> {
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            SWP_NOACTIVATE,
        )
    }
    .map_err(failed("SetWindowPos"))
}

fn render_view(
    canvas: &Canvas<'_>,
    layout: &ToolbarLayout,
    model: &ToolbarModel,
    docked: bool,
    strip: bool,
    scale: f32,
) -> Result<(), GfxError> {
    let (width, height) = layout.size();
    canvas.fill_rect([0.0, 0.0, width * scale, height * scale], PAPER)?;
    for (id, rect) in layout.placements() {
        if strip && *id != ButtonId::Dock {
            continue;
        }
        let physical = [
            rect.left * scale,
            rect.top * scale,
            rect.right * scale,
            rect.bottom * scale,
        ];
        if let ButtonId::Pen(index) = id {
            draw_pen_face(canvas, *index, physical, scale, model)?;
        }
        let icon = icon_for(*id, model, docked);
        draw_icon(canvas, icon, physical)?;
    }
    Ok(())
}

/// The legacy pen button: WhiteSmoke face, a 3 px WhiteSmoke border and a
/// transparent stripe that lets the pen colour show through as an underline.
fn draw_pen_face(
    canvas: &Canvas<'_>,
    index: u8,
    rect: [f32; 4],
    scale: f32,
    model: &ToolbarModel,
) -> Result<(), GfxError> {
    let Some(pen) = model.pen(usize::from(index)) else {
        return Ok(());
    };
    canvas.fill_rect(rect, PAPER)?;
    let inside = [
        rect[0] + PEN_BORDER_PX * scale,
        rect[1] + PEN_BORDER_PX * scale,
        rect[2] - PEN_BORDER_PX * scale,
        rect[3] - PEN_BORDER_PX * scale,
    ];
    if inside[2] <= inside[0] || inside[3] <= inside[1] {
        return Ok(());
    }
    let height = rect[3] - rect[1];
    let stripe = [
        inside[0],
        rect[1] + height * PEN_UNDERLINE_TOP,
        inside[2],
        rect[1] + height * PEN_UNDERLINE_BOTTOM,
    ];
    let brush = canvas.solid(pen.color)?;
    canvas.fill_rect_solid(stripe, &brush)
}
