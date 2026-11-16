//! Original gInk toolbar bitmaps decoded through WIC and drawn by Direct2D.
//!
//! The legacy gInk toolbar uses 128x128 PNG bitmaps (`pencil1.png`,
//! `eraser.png`, ...) scaled into its native buttons. `IconBitmaps` owns the
//! decoded D2D resources for one window. A missing or damaged asset is reported
//! and falls back to the previous vector geometry, so asset packaging problems
//! never prevent annotation. The pen buttons retain the legacy transparent
//! stripe through which the configured pen colour is visible.

use std::path::Path;

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND, D2D1_ELLIPSE,
    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, D2D1_LINE_JOIN_ROUND, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES1, D2D1_SWEEP_DIRECTION_CLOCKWISE, ID2D1Bitmap, ID2D1Brush,
    ID2D1DeviceContext, ID2D1Factory1, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnLoad,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::PCWSTR;
use windows_numerics::{Matrix3x2, Vector2};

use crate::gfx::device::{GfxError, failed};
use crate::settings::Rgb;
use crate::ui::toolbar::{ButtonId, ToolbarModel};

/// Panel background, legacy `WhiteSmoke`.
const PAPER: Rgb = Rgb::new(245, 245, 245);
/// Active icon colour sampled from the legacy `*_act` bitmaps.
const GOLD: Rgb = Rgb::new(239, 192, 67);
/// Eraser cap and shadow.
const GRAY: Rgb = Rgb::new(142, 142, 142);
/// Crossed-out ink eye stroke.
const RED: Rgb = Rgb::new(240, 57, 34);
const INK: Rgb = Rgb::new(0, 0, 0);
const WHITE: Rgb = Rgb::new(255, 255, 255);

/// The icon bitmap state selected for a button, mirroring the legacy
/// `image_*`/`image_*_act` bitmaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Dock,
    DockBack,
    Pencil,
    PencilActive,
    Highlighter,
    HighlighterActive,
    Eraser,
    EraserActive,
    Pointer,
    PointerActive,
    Pan,
    PanActive,
    PenWidth,
    InkVisible,
    InkHidden,
    Snapshot,
    Undo,
    Clear,
    Exit,
}

impl Icon {
    pub const ALL: [Self; 19] = [
        Self::Dock,
        Self::DockBack,
        Self::Pencil,
        Self::PencilActive,
        Self::Highlighter,
        Self::HighlighterActive,
        Self::Eraser,
        Self::EraserActive,
        Self::Pointer,
        Self::PointerActive,
        Self::Pan,
        Self::PanActive,
        Self::PenWidth,
        Self::InkVisible,
        Self::InkHidden,
        Self::Snapshot,
        Self::Undo,
        Self::Clear,
        Self::Exit,
    ];

    /// Original gInk PNG used for this toolbar state.
    pub const fn asset_name(self) -> &'static str {
        match self {
            Self::Dock => "dock.png",
            Self::DockBack => "dockback.png",
            Self::Pencil => "pencil1.png",
            Self::PencilActive => "pencil_act.png",
            Self::Highlighter => "highlighter1.png",
            Self::HighlighterActive => "highlighter_act.png",
            Self::Eraser => "eraser.png",
            Self::EraserActive => "eraseract.png",
            Self::Pointer => "pointer1.png",
            Self::PointerActive => "pointer_act.png",
            Self::Pan => "pan.png",
            Self::PanActive => "pan_act.png",
            Self::PenWidth => "penwidth.png",
            Self::InkVisible => "visible.png",
            Self::InkHidden => "visible_not.png",
            Self::Snapshot => "snap.png",
            Self::Undo => "undo.png",
            Self::Clear => "garbage.png",
            Self::Exit => "exit.png",
        }
    }
}

/// Window-local decoded originals. `None` entries select the vector fallback.
pub(crate) struct IconBitmaps {
    bitmaps: [Option<ID2D1Bitmap>; 19],
    warnings: Vec<String>,
}

impl IconBitmaps {
    pub(crate) fn load(context: &ID2D1DeviceContext) -> Self {
        match crate::assets::icon_directory() {
            Ok(directory) => Self::load_from(context, &directory),
            Err(error) => Self {
                bitmaps: std::array::from_fn(|_| None),
                warnings: vec![format!(
                    "could not locate the executable asset directory: {error}; using vector icons"
                )],
            },
        }
    }

    fn load_from(context: &ID2D1DeviceContext, directory: &Path) -> Self {
        let mut bitmaps: [Option<ID2D1Bitmap>; 19] = std::array::from_fn(|_| None);
        let mut warnings = Vec::new();
        match BitmapLoader::new() {
            Ok(loader) => {
                for icon in Icon::ALL {
                    let path = directory.join(icon.asset_name());
                    match loader.load(context, &path) {
                        Ok(bitmap) => bitmaps[icon as usize] = Some(bitmap),
                        Err(error) => warnings.push(format!(
                            "legacy icon {} could not be loaded: {error}; using vector fallback",
                            path.display()
                        )),
                    }
                }
            }
            Err(error) => warnings.push(format!(
                "WIC could not be initialized: {error}; using vector toolbar icons"
            )),
        }
        Self { bitmaps, warnings }
    }

    pub(crate) fn get(&self, icon: Icon) -> Option<&ID2D1Bitmap> {
        self.bitmaps[icon as usize].as_ref()
    }

    pub(crate) fn len(&self) -> usize {
        self.bitmaps
            .iter()
            .filter(|bitmap| bitmap.is_some())
            .count()
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

struct BitmapLoader {
    factory: IWICImagingFactory,
}

impl BitmapLoader {
    fn new() -> Result<Self, windows::core::Error> {
        let factory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?;
        Ok(Self { factory })
    }

    fn load(&self, context: &ID2D1DeviceContext, path: &Path) -> Result<ID2D1Bitmap, String> {
        use std::os::windows::ffi::OsStrExt;

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let decoder = unsafe {
            self.factory.CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnLoad,
            )
        }
        .map_err(|error| error.to_string())?;
        let frame = unsafe { decoder.GetFrame(0) }.map_err(|error| error.to_string())?;
        let converter =
            unsafe { self.factory.CreateFormatConverter() }.map_err(|error| error.to_string())?;
        unsafe {
            converter.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }
        .map_err(|error| error.to_string())?;
        // Direct2D creates its own resource from the converted pixels. The WIC
        // decoder, frame and converter can therefore all be released on return.
        unsafe { context.CreateBitmapFromWicBitmap(&converter, None) }
            .map_err(|error| error.to_string())
            .and_then(|bitmap| {
                windows::core::Interface::cast(&bitmap).map_err(|error| error.to_string())
            })
    }
}

/// Loads one additional window-local legacy bitmap (the pen-width panel).
pub(crate) fn load_legacy_bitmap(
    context: &ID2D1DeviceContext,
    path: &Path,
) -> Result<ID2D1Bitmap, String> {
    BitmapLoader::new()
        .map_err(|error| error.to_string())?
        .load(context, path)
}

/// Resolves the icon state for `button`.
///
/// Legacy rules (`src/FormCollection.cs`): the selected pen, eraser, pointer or
/// pan shows its `_act` bitmap; pens with `Transparency >= 100` (alpha <= 155)
/// use the highlighter bitmaps; the ink eye is crossed out while ink is hidden;
/// the dock chevron points back while docked.
pub fn icon_for(button: ButtonId, model: &ToolbarModel, docked: bool) -> Icon {
    let active = model.active() == Some(button);
    match button {
        ButtonId::Dock => {
            if docked {
                Icon::DockBack
            } else {
                Icon::Dock
            }
        }
        ButtonId::Pen(index) => {
            let highlighter = model
                .pen(usize::from(index))
                .is_some_and(|pen| pen.alpha <= 155);
            match (highlighter, active) {
                (false, false) => Icon::Pencil,
                (false, true) => Icon::PencilActive,
                (true, false) => Icon::Highlighter,
                (true, true) => Icon::HighlighterActive,
            }
        }
        ButtonId::Eraser => {
            if active {
                Icon::EraserActive
            } else {
                Icon::Eraser
            }
        }
        ButtonId::Pan => {
            if active {
                Icon::PanActive
            } else {
                Icon::Pan
            }
        }
        ButtonId::Pointer => {
            if active {
                Icon::PointerActive
            } else {
                Icon::Pointer
            }
        }
        ButtonId::PenWidth => Icon::PenWidth,
        ButtonId::InkVisible => {
            if model.ink_visible() {
                Icon::InkVisible
            } else {
                Icon::InkHidden
            }
        }
        ButtonId::Snapshot => Icon::Snapshot,
        ButtonId::Undo => Icon::Undo,
        ButtonId::Clear => Icon::Clear,
        ButtonId::Exit => Icon::Exit,
    }
}

/// Solid-colour brush set used for one toolbar frame.
pub(crate) struct Canvas<'a> {
    context: &'a ID2D1DeviceContext,
    factory: &'a ID2D1Factory1,
    ink: ID2D1SolidColorBrush,
    paper: ID2D1SolidColorBrush,
    gold: ID2D1SolidColorBrush,
    gray: ID2D1SolidColorBrush,
    red: ID2D1SolidColorBrush,
    white: ID2D1SolidColorBrush,
}

impl<'a> Canvas<'a> {
    pub(crate) fn new(
        context: &'a ID2D1DeviceContext,
        factory: &'a ID2D1Factory1,
    ) -> Result<Self, GfxError> {
        Ok(Self {
            context,
            factory,
            ink: solid(context, INK)?,
            paper: solid(context, PAPER)?,
            gold: solid(context, GOLD)?,
            gray: solid(context, GRAY)?,
            red: solid(context, RED)?,
            white: solid(context, WHITE)?,
        })
    }

    fn palette(&self, color: Rgb) -> &ID2D1SolidColorBrush {
        if color == INK {
            &self.ink
        } else if color == PAPER {
            &self.paper
        } else if color == GOLD {
            &self.gold
        } else if color == GRAY {
            &self.gray
        } else if color == RED {
            &self.red
        } else if color == WHITE {
            &self.white
        } else {
            &self.ink
        }
    }

    /// A brush for an arbitrary colour (pen swatches).
    pub(crate) fn solid(&self, color: Rgb) -> Result<ID2D1SolidColorBrush, GfxError> {
        solid(self.context, color)
    }

    pub(crate) fn fill_rect(&self, rect: [f32; 4], color: Rgb) -> Result<(), GfxError> {
        self.fill_rect_with(rect, self.palette(color))
    }

    /// Fills a rectangle with a caller-created brush (pen swatch colours).
    pub(crate) fn fill_rect_solid(
        &self,
        rect: [f32; 4],
        brush: &ID2D1SolidColorBrush,
    ) -> Result<(), GfxError> {
        self.fill_rect_with(rect, brush)
    }

    /// Draws an original PNG into the exact physical button rectangle.
    pub(crate) fn draw_bitmap(&self, bitmap: &ID2D1Bitmap, rect: [f32; 4]) -> Result<(), GfxError> {
        let destination = D2D_RECT_F {
            left: rect[0],
            top: rect[1],
            right: rect[2],
            bottom: rect[3],
        };
        unsafe {
            self.context.DrawBitmap(
                bitmap,
                Some(&destination),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        }
        Ok(())
    }

    fn fill_rect_with(&self, rect: [f32; 4], brush: &ID2D1SolidColorBrush) -> Result<(), GfxError> {
        let shape = D2D_RECT_F {
            left: rect[0],
            top: rect[1],
            right: rect[2],
            bottom: rect[3],
        };
        unsafe { self.context.FillRectangle(&shape, brush) };
        Ok(())
    }

    fn fill_round_rect(&self, rect: [f32; 4], radius: f32, color: Rgb) -> Result<(), GfxError> {
        let shape = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: rect[0],
                top: rect[1],
                right: rect[2],
                bottom: rect[3],
            },
            radiusX: radius,
            radiusY: radius,
        };
        let geometry = unsafe { self.factory.CreateRoundedRectangleGeometry(&shape) }
            .map_err(failed("ID2D1Factory::CreateRoundedRectangleGeometry"))?;
        unsafe {
            self.context
                .FillGeometry(&geometry, self.palette(color), None::<&ID2D1Brush>);
        }
        Ok(())
    }

    fn fill_ellipse(&self, centre: [f32; 2], radius: [f32; 2], color: Rgb) -> Result<(), GfxError> {
        let shape = D2D1_ELLIPSE {
            point: Vector2::new(centre[0], centre[1]),
            radiusX: radius[0],
            radiusY: radius[1],
        };
        unsafe { self.context.FillEllipse(&shape, self.palette(color)) };
        Ok(())
    }

    fn fill_polygon(&self, points: &[[f32; 2]], color: Rgb) -> Result<(), GfxError> {
        let geometry = unsafe { self.factory.CreatePathGeometry() }
            .map_err(failed("ID2D1Factory::CreatePathGeometry"))?;
        let sink = unsafe { geometry.Open() }.map_err(failed("ID2D1PathGeometry::Open"))?;
        if let Some(first) = points.first() {
            unsafe {
                sink.BeginFigure(Vector2::new(first[0], first[1]), D2D1_FIGURE_BEGIN_FILLED);
                for point in &points[1..] {
                    sink.AddLine(Vector2::new(point[0], point[1]));
                }
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
        }
        unsafe { sink.Close() }.map_err(failed("ID2D1GeometrySink::Close"))?;
        unsafe {
            self.context
                .FillGeometry(&geometry, self.palette(color), None::<&ID2D1Brush>);
        }
        Ok(())
    }

    fn line(&self, from: [f32; 2], to: [f32; 2], width: f32, color: Rgb) -> Result<(), GfxError> {
        let style = self.round_style()?;
        unsafe {
            self.context.DrawLine(
                Vector2::new(from[0], from[1]),
                Vector2::new(to[0], to[1]),
                self.palette(color),
                width,
                &style,
            );
        }
        Ok(())
    }

    fn round_style(
        &self,
    ) -> Result<windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle1, GfxError> {
        let properties = D2D1_STROKE_STYLE_PROPERTIES1 {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            ..Default::default()
        };
        unsafe { self.factory.CreateStrokeStyle(&properties, None) }
            .map_err(failed("ID2D1Factory::CreateStrokeStyle"))
    }

    fn stroke_arc(
        &self,
        centre: [f32; 2],
        radius: f32,
        start_degrees: f32,
        sweep_degrees: f32,
        width: f32,
        color: Rgb,
    ) -> Result<(), GfxError> {
        let geometry = unsafe { self.factory.CreatePathGeometry() }
            .map_err(failed("ID2D1Factory::CreatePathGeometry"))?;
        let sink = unsafe { geometry.Open() }.map_err(failed("ID2D1PathGeometry::Open"))?;
        let start = polar(centre, radius, start_degrees);
        let end = polar(centre, radius, start_degrees + sweep_degrees);
        unsafe {
            sink.BeginFigure(Vector2::new(start[0], start[1]), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2::new(end[0], end[1]),
                size: D2D_SIZE_F {
                    width: radius,
                    height: radius,
                },
                rotationAngle: 0.0,
                sweepDirection: if sweep_degrees >= 0.0 {
                    D2D1_SWEEP_DIRECTION_CLOCKWISE
                } else {
                    windows::Win32::Graphics::Direct2D::D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                },
                arcSize: if sweep_degrees.abs() > 180.0 {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        unsafe { sink.Close() }.map_err(failed("ID2D1GeometrySink::Close"))?;
        let style = self.round_style()?;
        unsafe {
            self.context
                .DrawGeometry(&geometry, self.palette(color), width, &style);
        }
        Ok(())
    }

    fn with_rotation(
        &self,
        centre: [f32; 2],
        degrees: f32,
        draw: impl FnOnce(&Self) -> Result<(), GfxError>,
    ) -> Result<(), GfxError> {
        let transform = Matrix3x2::rotation_around(degrees, Vector2::new(centre[0], centre[1]));
        unsafe { self.context.SetTransform(&transform) };
        let result = draw(self);
        unsafe { self.context.SetTransform(&Matrix3x2::identity()) };
        result
    }
}

fn solid(context: &ID2D1DeviceContext, color: Rgb) -> Result<ID2D1SolidColorBrush, GfxError> {
    let color = D2D1_COLOR_F {
        r: f32::from(color.red) / 255.0,
        g: f32::from(color.green) / 255.0,
        b: f32::from(color.blue) / 255.0,
        a: 1.0,
    };
    unsafe { context.CreateSolidColorBrush(&color, None) }
        .map_err(failed("ID2D1RenderTarget::CreateSolidColorBrush"))
}

fn polar(centre: [f32; 2], radius: f32, degrees: f32) -> [f32; 2] {
    let radians = degrees.to_radians();
    [
        centre[0] + radius * radians.cos(),
        centre[1] + radius * radians.sin(),
    ]
}

/// Unit box mapped into a physical button rectangle.
struct IconBox {
    x: f32,
    y: f32,
    size: f32,
}

impl IconBox {
    /// The legacy bitmaps use ~14% padding around the glyph.
    fn new(rect: [f32; 4]) -> Self {
        let width = rect[2] - rect[0];
        let height = rect[3] - rect[1];
        let side = width.min(height);
        let pad = side * 0.14;
        Self {
            x: rect[0] + pad,
            y: rect[1] + pad,
            size: side - 2.0 * pad,
        }
    }

    fn point(&self, x: f32, y: f32) -> [f32; 2] {
        [self.x + x * self.size, self.y + y * self.size]
    }

    fn length(&self, fraction: f32) -> f32 {
        self.size * fraction
    }

    fn rect(&self, left: f32, top: f32, right: f32, bottom: f32) -> [f32; 4] {
        [
            self.x + left * self.size,
            self.y + top * self.size,
            self.x + right * self.size,
            self.y + bottom * self.size,
        ]
    }

    fn centre(&self) -> [f32; 2] {
        self.point(0.5, 0.5)
    }
}

/// Draws the original bitmap when available, otherwise the vector fallback.
pub(crate) fn draw_icon(
    canvas: &Canvas<'_>,
    icon: Icon,
    rect: [f32; 4],
    bitmaps: Option<&IconBitmaps>,
) -> Result<(), GfxError> {
    if let Some(bitmap) = bitmaps.and_then(|assets| assets.get(icon)) {
        return canvas.draw_bitmap(bitmap, rect);
    }
    draw_vector_icon(canvas, icon, rect)
}

fn draw_vector_icon(canvas: &Canvas<'_>, icon: Icon, rect: [f32; 4]) -> Result<(), GfxError> {
    let b = IconBox::new(rect);
    match icon {
        Icon::Pencil => pencil(canvas, &b, INK),
        Icon::PencilActive => {
            pencil(canvas, &b, WHITE)?;
            canvas.fill_ellipse(b.point(0.16, 0.16), [b.length(0.045); 2], INK)
        }
        Icon::Highlighter => highlighter(canvas, &b, INK),
        Icon::HighlighterActive => {
            highlighter(canvas, &b, WHITE)?;
            canvas.fill_ellipse(b.point(0.16, 0.16), [b.length(0.045); 2], INK)
        }
        Icon::Eraser => eraser(canvas, &b, INK),
        Icon::EraserActive => eraser(canvas, &b, GOLD),
        Icon::Pointer => canvas.fill_polygon(&pointer_points(&b), INK),
        Icon::PointerActive => canvas.fill_polygon(&pointer_points(&b), GOLD),
        Icon::Pan => pan(canvas, &b, INK),
        Icon::PanActive => pan(canvas, &b, GOLD),
        Icon::PenWidth => pen_width(canvas, &b),
        Icon::InkVisible => ink_eye(canvas, &b, false),
        Icon::InkHidden => ink_eye(canvas, &b, true),
        Icon::Snapshot => snapshot(canvas, &b),
        Icon::Undo => undo(canvas, &b),
        Icon::Clear => clear(canvas, &b),
        Icon::Exit => {
            let width = b.length(0.13);
            canvas.line(b.point(0.22, 0.22), b.point(0.78, 0.78), width, INK)?;
            canvas.line(b.point(0.78, 0.22), b.point(0.22, 0.78), width, INK)
        }
        Icon::Dock => dock_chevron(canvas, &b, false),
        Icon::DockBack => dock_chevron(canvas, &b, true),
    }
}

fn pencil(canvas: &Canvas<'_>, b: &IconBox, color: Rgb) -> Result<(), GfxError> {
    let width = b.length(0.20);
    canvas.line(b.point(0.34, 0.76), b.point(0.70, 0.34), width, color)?;
    // The white gap that separates the body from the eraser cap.
    canvas.line(
        b.point(0.66, 0.25),
        b.point(0.80, 0.39),
        b.length(0.055),
        PAPER,
    )?;
    canvas.line(
        b.point(0.77, 0.30),
        b.point(0.87, 0.18),
        b.length(0.18),
        color,
    )?;
    canvas.fill_polygon(
        &[
            b.point(0.10, 0.98),
            b.point(0.26, 0.80),
            b.point(0.33, 0.92),
        ],
        color,
    )
}

fn highlighter(canvas: &Canvas<'_>, b: &IconBox, color: Rgb) -> Result<(), GfxError> {
    canvas.line(
        b.point(0.40, 0.70),
        b.point(0.72, 0.26),
        b.length(0.30),
        color,
    )?;
    canvas.fill_polygon(
        &[
            b.point(0.14, 0.88),
            b.point(0.32, 0.72),
            b.point(0.42, 0.84),
        ],
        color,
    )
}

fn eraser(canvas: &Canvas<'_>, b: &IconBox, body: Rgb) -> Result<(), GfxError> {
    let width = b.length(0.50);
    let height = b.length(0.27);
    let centre = b.centre();
    canvas.with_rotation(centre, -38.0, |rotated| {
        let (cx, cy) = (centre[0], centre[1]);
        rotated.fill_round_rect(
            [
                cx - width / 2.0,
                cy - height / 2.0 - b.length(0.06),
                cx + width / 2.0,
                cy + height / 2.0 - b.length(0.06),
            ],
            b.length(0.07),
            body,
        )?;
        rotated.fill_round_rect(
            [
                cx - width / 2.0 - b.length(0.16),
                cy - height / 2.0 + b.length(0.20),
                cx - width / 2.0 + b.length(0.12),
                cy + height / 2.0 + b.length(0.20),
            ],
            b.length(0.06),
            GRAY,
        )
    })?;
    canvas.line(
        b.point(0.46, 0.94),
        b.point(0.88, 0.94),
        b.length(0.07),
        GRAY,
    )
}

fn pan(canvas: &Canvas<'_>, b: &IconBox, color: Rgb) -> Result<(), GfxError> {
    let width = b.length(0.09);
    canvas.line(b.point(0.18, 0.5), b.point(0.82, 0.5), width, color)?;
    canvas.line(b.point(0.5, 0.18), b.point(0.5, 0.82), width, color)?;
    // The legacy pan bitmap has solid triangular arrow heads on all four axes.
    for head in [
        [(0.94, 0.5), (0.70, 0.34), (0.70, 0.66)],
        [(0.06, 0.5), (0.30, 0.34), (0.30, 0.66)],
        [(0.5, 0.94), (0.34, 0.70), (0.66, 0.70)],
        [(0.5, 0.06), (0.34, 0.30), (0.66, 0.30)],
    ] {
        canvas.fill_polygon(
            &[
                b.point(head[0].0, head[0].1),
                b.point(head[1].0, head[1].1),
                b.point(head[2].0, head[2].1),
            ],
            color,
        )?;
    }
    Ok(())
}

fn pointer_points(b: &IconBox) -> [[f32; 2]; 7] {
    [
        b.point(0.27, 0.10),
        b.point(0.70, 0.54),
        b.point(0.55, 0.58),
        b.point(0.68, 0.84),
        b.point(0.56, 0.90),
        b.point(0.44, 0.64),
        b.point(0.30, 0.78),
    ]
}

fn pen_width(canvas: &Canvas<'_>, b: &IconBox) -> Result<(), GfxError> {
    for (from, to, width) in [
        ((0.18, 0.82), (0.34, 0.38), 0.05),
        ((0.45, 0.86), (0.61, 0.34), 0.09),
        ((0.69, 0.90), (0.87, 0.28), 0.14),
    ] {
        canvas.line(
            b.point(from.0, from.1),
            b.point(to.0, to.1),
            b.length(width),
            INK,
        )?;
    }
    Ok(())
}

fn ink_eye(canvas: &Canvas<'_>, b: &IconBox, hidden: bool) -> Result<(), GfxError> {
    let centre = b.point(0.5, 0.5);
    canvas.fill_ellipse(centre, [b.length(0.40), b.length(0.24)], INK)?;
    canvas.fill_ellipse(centre, [b.length(0.22), b.length(0.15)], PAPER)?;
    canvas.fill_ellipse(centre, [b.length(0.10), b.length(0.10)], INK)?;
    if hidden {
        canvas.line(
            b.point(0.16, 0.86),
            b.point(0.86, 0.14),
            b.length(0.08),
            RED,
        )?;
    }
    Ok(())
}

fn snapshot(canvas: &Canvas<'_>, b: &IconBox) -> Result<(), GfxError> {
    canvas.fill_round_rect(b.rect(0.08, 0.30, 0.92, 0.86), b.length(0.07), INK)?;
    canvas.fill_round_rect(b.rect(0.34, 0.18, 0.62, 0.34), b.length(0.04), INK)?;
    let lens = b.point(0.47, 0.58);
    canvas.fill_ellipse(lens, [b.length(0.21); 2], PAPER)?;
    canvas.fill_ellipse(lens, [b.length(0.12); 2], INK)?;
    canvas.fill_ellipse(b.point(0.78, 0.42), [b.length(0.035); 2], PAPER)
}

fn undo(canvas: &Canvas<'_>, b: &IconBox) -> Result<(), GfxError> {
    // Open circular arrow with the head at the lower-left, like the legacy
    // undo bitmap.
    let centre = b.point(0.54, 0.46);
    canvas.stroke_arc(centre, b.length(0.30), -140.0, 290.0, b.length(0.14), INK)?;
    canvas.fill_polygon(
        &[
            b.point(0.08, 0.82),
            b.point(0.30, 0.56),
            b.point(0.44, 0.74),
        ],
        INK,
    )
}

fn clear(canvas: &Canvas<'_>, b: &IconBox) -> Result<(), GfxError> {
    canvas.fill_round_rect(b.rect(0.14, 0.14, 0.86, 0.27), b.length(0.05), INK)?;
    canvas.fill_round_rect(b.rect(0.38, 0.08, 0.62, 0.18), b.length(0.04), INK)?;
    canvas.fill_round_rect(b.rect(0.20, 0.29, 0.80, 0.88), b.length(0.04), INK)?;
    canvas.fill_round_rect(b.rect(0.34, 0.39, 0.42, 0.79), b.length(0.03), PAPER)?;
    canvas.fill_round_rect(b.rect(0.58, 0.39, 0.66, 0.79), b.length(0.03), PAPER)
}

fn dock_chevron(canvas: &Canvas<'_>, b: &IconBox, back: bool) -> Result<(), GfxError> {
    let width = b.length(0.10);
    if back {
        canvas.line(b.point(0.66, 0.18), b.point(0.34, 0.5), width, INK)?;
        canvas.line(b.point(0.34, 0.5), b.point(0.66, 0.82), width, INK)
    } else {
        canvas.line(b.point(0.34, 0.18), b.point(0.66, 0.5), width, INK)?;
        canvas.line(b.point(0.66, 0.5), b.point(0.34, 0.82), width, INK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::device::GraphicsDevice;

    #[test]
    #[ignore = "creates a real Direct2D device and decodes WIC assets"]
    fn wic_loads_every_legacy_icon_and_missing_assets_degrade() {
        let device = GraphicsDevice::new().expect("graphics device");
        let context = device.create_device_context().expect("device context");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let loaded = IconBitmaps::load_from(&context, &root);
        assert_eq!(loaded.len(), Icon::ALL.len());
        assert!(loaded.warnings().is_empty(), "{:?}", loaded.warnings());

        let missing = std::env::temp_dir().join(format!(
            "plnk-missing-icons-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let fallback = IconBitmaps::load_from(&context, &missing);
        assert_eq!(fallback.len(), 0);
        assert_eq!(fallback.warnings().len(), Icon::ALL.len());
        assert!(fallback.get(Icon::Pencil).is_none());
    }
}
