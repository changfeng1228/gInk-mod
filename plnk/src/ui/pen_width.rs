//! Legacy pen-width popup: a 300×80 DIP slider above the toolbar.
//!
//! `FormCollection.gpPenWidth` keeps `GlobalPenWidth` in Microsoft.Ink
//! HIMETRIC units and updates it with `x*x/30` while dragging. The value is
//! session-only (default 80), applies immediately to the active pen, and is not
//! persisted. The popup closes on release, on pen selection, or on the first
//! Esc; only rendering converts the value to physical pixels.

use std::marker::PhantomData;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, PostMessageW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowPos,
    ShowWindow, WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};
use windows::core::w;

use crate::gfx::composition::WindowComposition;
use crate::gfx::device::{GfxError, GraphicsDevice, failed};
use crate::platform::dpi::{DpiMap, PhysicalRect};
use crate::platform::window::NativeWindow;
use crate::settings::Rgb;
use crate::ui::icons::{Canvas, load_legacy_bitmap};

pub const PANEL_WIDTH_DIP: f32 = 300.0;
pub const PANEL_HEIGHT_DIP: f32 = 80.0;
pub const DEFAULT_WIDTH_HIMETRIC: f32 = 80.0;
const MARGIN_DIP: f32 = 10.0;
const INDICATOR_WIDTH_DIP: f32 = 8.0;

/// Session-only pen-width state in legacy HIMETRIC units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenWidthModel {
    width_himetric: f32,
}

impl Default for PenWidthModel {
    fn default() -> Self {
        Self {
            width_himetric: DEFAULT_WIDTH_HIMETRIC,
        }
    }
}

impl PenWidthModel {
    pub fn width_himetric(&self) -> f32 {
        self.width_himetric
    }

    /// Legacy drag/update rule. Positions inside either 10-DIP margin are
    /// ignored; valid positions use `x²/30` and clamp to the legacy width range.
    pub fn update_from_x(&mut self, x_dip: f32) -> Option<f32> {
        if !x_dip.is_finite() || x_dip < MARGIN_DIP || PANEL_WIDTH_DIP - x_dip < MARGIN_DIP {
            return None;
        }
        let width = (x_dip * x_dip / 30.0).clamp(30.0, 3000.0);
        self.width_himetric = width;
        Some(width)
    }

    /// Legacy indicator placement: `sqrt(width * 30)` DIP.
    pub fn indicator_x(&self) -> f32 {
        (self.width_himetric * 30.0).sqrt()
    }
}

enum Op {
    SetRect(PhysicalRect),
    Show,
    Hide,
    SetCapture,
    ReleaseCapture,
    Render,
}

struct PenWidthUi {
    // The bitmap is released before its context; composition precedes the HWND.
    panel_bitmap: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap>,
    composition: Option<WindowComposition>,
    window: Option<NativeWindow>,
    model: PenWidthModel,
    dpi: DpiMap,
    rect: PhysicalRect,
    visible: bool,
    dragging: bool,
    notify: Option<(HWND, u32)>,
    last_error: Option<String>,
    asset_warnings: Vec<String>,
}

impl PenWidthUi {
    fn render(&mut self) -> Result<(), GfxError> {
        let Self {
            panel_bitmap,
            composition,
            model,
            dpi,
            ..
        } = self;
        let Some(composition) = composition.as_mut() else {
            return Ok(());
        };
        let scale = dpi.scale();
        let indicator_x = model.indicator_x();
        composition.draw(|context, factory| {
            let canvas = Canvas::new(context, factory)?;
            let paper = Rgb::new(245, 245, 245);
            canvas.fill_rect(
                [0.0, 0.0, PANEL_WIDTH_DIP * scale, PANEL_HEIGHT_DIP * scale],
                paper,
            )?;
            if let Some(bitmap) = panel_bitmap.as_ref() {
                // Legacy BackgroundImageLayout.Center: the original 200x53
                // image stays centred in the 300x80 panel rather than stretching.
                canvas.draw_bitmap(
                    bitmap,
                    [50.0 * scale, 13.5 * scale, 250.0 * scale, 66.5 * scale],
                )?;
            } else {
                // Missing-asset fallback: retain the earlier native track and
                // exemplars instead of leaving the slider visually blank.
                let y = PANEL_HEIGHT_DIP * 0.5 * scale;
                canvas.fill_rect(
                    [
                        MARGIN_DIP * scale,
                        y - 1.0,
                        (PANEL_WIDTH_DIP - MARGIN_DIP) * scale,
                        y + 1.0,
                    ],
                    Rgb::new(80, 80, 80),
                )?;
                for (x, half_height) in [(55.0, 5.0), (150.0, 12.0), (245.0, 20.0)] {
                    canvas.fill_rect(
                        [
                            x * scale,
                            y - half_height * scale,
                            (x + 4.0) * scale,
                            y + half_height * scale,
                        ],
                        Rgb::new(0, 0, 0),
                    )?;
                }
            }
            // The orange full-height indicator matches the legacy
            // pboxPenWidthIndicator interaction and position.
            let centre = indicator_x * scale;
            let half = INDICATOR_WIDTH_DIP * 0.5 * scale;
            let orange = canvas.solid(Rgb::new(255, 165, 0))?;
            canvas.fill_rect_solid(
                [centre - half, 0.0, centre + half, PANEL_HEIGHT_DIP * scale],
                &orange,
            )
        })
    }

    fn client_x_dip(&self, lparam: LPARAM) -> f32 {
        let x = i32::from((lparam.0 & 0xFFFF) as u16 as i16);
        x as f32 / self.dpi.scale()
    }

    fn update_width(&mut self, x_dip: f32, ops: &mut Vec<Op>) {
        let Some(width) = self.model.update_from_x(x_dip) else {
            return;
        };
        ops.push(Op::Render);
        if let Some((target, message)) = self.notify {
            let _ = unsafe {
                PostMessageW(
                    Some(target),
                    message,
                    WPARAM(width.round() as usize),
                    LPARAM(0),
                )
            };
        }
    }

    fn on_message(&mut self, message: u32, lparam: LPARAM) -> Vec<Op> {
        match message {
            WM_LBUTTONDOWN => {
                self.dragging = true;
                vec![Op::SetCapture]
            }
            WM_MOUSEMOVE if self.dragging => {
                let mut ops = Vec::new();
                self.update_width(self.client_x_dip(lparam), &mut ops);
                ops
            }
            WM_LBUTTONUP if self.dragging => {
                self.dragging = false;
                let mut ops = vec![Op::ReleaseCapture];
                self.update_width(self.client_x_dip(lparam), &mut ops);
                self.visible = false;
                ops.push(Op::Hide);
                ops
            }
            WM_CAPTURECHANGED => {
                self.dragging = false;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// The native popup window. `PenWidthModel` is separately headless-testable.
pub struct PenWidthPanel {
    ui: Box<PenWidthUi>,
    _not_send: PhantomData<Rc<()>>,
}

impl PenWidthPanel {
    pub fn new(device: &GraphicsDevice, owner: HWND) -> Result<Self, GfxError> {
        let dpi = DpiMap::from_window(owner);
        let rect = panel_size_rect(dpi, 0, 0);
        let mut ui = Box::new(PenWidthUi {
            panel_bitmap: None,
            asset_warnings: Vec::new(),
            composition: None,
            window: None,
            model: PenWidthModel::default(),
            dpi,
            rect,
            visible: false,
            dragging: false,
            notify: None,
            last_error: None,
        });
        let data = &mut *ui as *mut PenWidthUi as *mut core::ffi::c_void;
        let window = NativeWindow::create_with_hook(
            rect,
            w!("plnk pen width"),
            false,
            Some(pen_width_hook),
            data,
        )?;
        window.set_owner(owner)?;
        let composition = WindowComposition::new(
            device,
            window.hwnd(),
            rect.width() as u32,
            rect.height() as u32,
        )?;
        match crate::assets::icon_directory() {
            Ok(directory) => {
                let path = directory.join("penwidthpanel.png");
                match load_legacy_bitmap(composition.context(), &path) {
                    Ok(bitmap) => ui.panel_bitmap = Some(bitmap),
                    Err(error) => ui.asset_warnings.push(format!(
                        "legacy pen-width panel {} could not be loaded: {error}; using vector fallback",
                        path.display()
                    )),
                }
            }
            Err(error) => ui.asset_warnings.push(format!(
                "could not locate pen-width panel assets: {error}; using vector fallback"
            )),
        }
        ui.dpi = DpiMap::from_window(window.hwnd());
        ui.window = Some(window);
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

    pub fn set_change_notifier(&mut self, target: HWND, message: u32) {
        self.ui.notify = Some((target, message));
    }

    pub fn is_visible(&self) -> bool {
        self.ui.visible
    }

    pub fn width_himetric(&self) -> f32 {
        self.ui.model.width_himetric()
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.ui.last_error.take()
    }

    pub fn legacy_panel_bitmap_loaded(&self) -> bool {
        self.ui.panel_bitmap.is_some()
    }

    pub fn take_asset_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.ui.asset_warnings)
    }

    /// Centers the 300×80 DIP panel over the pen-width button, 10 DIP above
    /// the toolbar, exactly like the legacy `gpPenWidth.Left/Top` calculation.
    pub fn show_above(&mut self, toolbar: PhysicalRect, pen_button: PhysicalRect) {
        let scale = self.ui.dpi.scale();
        let width = (PANEL_WIDTH_DIP * scale).round() as i32;
        let height = (PANEL_HEIGHT_DIP * scale).round() as i32;
        let left = pen_button.left + pen_button.width() / 2 - width / 2;
        let top = toolbar.top - height - (10.0 * scale).round() as i32;
        self.ui.rect = PhysicalRect::from_xywh(left, top, width, height);
        self.ui.visible = true;
        let ops = vec![Op::SetRect(self.ui.rect), Op::Show, Op::Render];
        self.run(ops);
    }

    pub fn hide(&mut self) {
        self.ui.visible = false;
        self.ui.dragging = false;
        self.run(vec![Op::Hide]);
    }

    fn run(&mut self, ops: Vec<Op>) {
        let pointer = &mut *self.ui as *mut PenWidthUi;
        let hwnd = self.hwnd();
        run_ops(hwnd, pointer, ops);
    }
}

fn panel_size_rect(dpi: DpiMap, left: i32, top: i32) -> PhysicalRect {
    let scale = dpi.scale();
    PhysicalRect::from_xywh(
        left,
        top,
        (PANEL_WIDTH_DIP * scale).round() as i32,
        (PANEL_HEIGHT_DIP * scale).round() as i32,
    )
}

unsafe fn pen_width_hook(
    data: *mut core::ffi::c_void,
    hwnd: HWND,
    message: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match message {
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_CAPTURECHANGED => {
            let ops = unsafe { (&mut *(data as *mut PenWidthUi)).on_message(message, lparam) };
            // Capture operations can synchronously re-enter this hook; the
            // borrow above has ended before they run.
            run_ops(hwnd, data as *mut PenWidthUi, ops);
        }
        _ => {}
    }
    None
}

fn run_ops(hwnd: HWND, ui: *mut PenWidthUi, ops: Vec<Op>) {
    for op in ops {
        let result = match op {
            Op::SetRect(rect) => unsafe {
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
            .map_err(failed("SetWindowPos(pen width panel)")),
            Op::Show => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                Ok(())
            }
            Op::Hide => {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                Ok(())
            }
            Op::SetCapture => {
                unsafe { SetCapture(hwnd) };
                Ok(())
            }
            Op::ReleaseCapture => unsafe { ReleaseCapture() }.map_err(failed("ReleaseCapture")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_slider_formula_margins_and_indicator_are_exact() {
        let mut model = PenWidthModel::default();
        assert_eq!(model.width_himetric(), 80.0);
        assert!((model.indicator_x() - 48.989_795).abs() < 1e-4);
        assert_eq!(model.update_from_x(9.0), None);
        assert_eq!(model.update_from_x(291.0), None);
        assert_eq!(model.update_from_x(10.0), Some(30.0));
        assert_eq!(model.update_from_x(120.0), Some(480.0));
        assert!((model.indicator_x() - 120.0).abs() < 1e-4);
        assert!((model.update_from_x(290.0).unwrap_or_default() - 2_803.333_3).abs() < 1e-3);
    }

    #[test]
    #[ignore = "creates a real Direct2D device and decodes the legacy panel"]
    fn popup_loads_the_original_pen_width_panel_bitmap() {
        let device = crate::gfx::device::GraphicsDevice::new().expect("graphics device");
        let context = device.create_device_context().expect("device context");
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons/penwidthpanel.png");
        let bitmap = super::load_legacy_bitmap(&context, &path).expect("legacy panel PNG");
        assert_eq!(unsafe { bitmap.GetSize() }.width, 200.0);
        assert_eq!(unsafe { bitmap.GetSize() }.height, 53.0);
    }
}
