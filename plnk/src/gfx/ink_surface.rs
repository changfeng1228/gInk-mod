//! Ink presentation surface for the overlay window.
//!
//! Two full-window Direct2D bitmaps act as the layer split from plan §6.1:
//!
//! * the **static cache** holds committed strokes. It is only touched when the
//!   document changes (stroke commit, undo, redo, clear, close); each update is
//!   clipped to the edit's [`DirtyRegion`].
//! * the **active layer** holds the live stroke. It is cleared and redrawn only
//!   inside the dirty rectangle of the newest pointer samples.
//!
//! `present` copies both cached textures through the DirectComposition swap
//! chain with one GPU composite and one `Present`. No CPU pixel buffer is ever
//! uploaded per frame — the old GDI architecture's full-screen bitmap upload is
//! what the rewrite replaces (spec §5/§6.1); the presentation cost is a
//! GPU-to-GPU blit whose CPU side is constant, while stroke rasterization work
//! stays proportional to the dirty region.
//!
//! Coordinates are physical screen pixels. The surface subtracts the window
//! origin when it builds stroke geometry and converts dirty rects into
//! window-local clips, so ink coordinates are independent of where the overlay
//! sits.

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_PRIMITIVE_BLEND_COPY,
    D2D1_PRIMITIVE_BLEND_SOURCE_OVER, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::Interface;

use crate::gfx::device::{GfxError, GraphicsDevice, failed};
use crate::ink::dirty::{DirtyRect, DirtyRegion};
use crate::ink::geometry::PhysicalPointF;
use crate::ink::model::{InkDocument, Stroke};
use crate::ink::renderer::StrokeRenderer;
use crate::platform::dpi::PhysicalRect;
use crate::platform::window::{OverlayWindow, WindowHook};

/// What one [`InkSurface::present`] call submitted.
///
/// `painted_area` sums the clip-rect areas the static/active rasterizers
/// touched since the previous present; `uploaded_bytes` is zero by
/// construction because the present path moves no CPU pixel data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresentStats {
    /// The dirty region this frame was asked to present.
    pub region: DirtyRegion,
    /// Number of disjoint rectangles in `region`.
    pub rects: usize,
    /// Total area of `region`, in physical pixels.
    pub area: u64,
    /// Pixels clipped for stroke rasterization since the previous present.
    pub painted_area: u64,
    /// Bytes of CPU pixel data uploaded during this present (always 0).
    pub uploaded_bytes: u64,
}

/// Transparent DirectComposition overlay with cached ink layers.
pub struct InkSurface {
    // Declared before `window` so the canvas targets release before the
    // composition swap chain and HWND.
    static_context: ID2D1DeviceContext,
    static_target: ID2D1Bitmap1,
    active_context: ID2D1DeviceContext,
    active_target: ID2D1Bitmap1,
    factory: ID2D1Factory1,
    window: OverlayWindow,
    origin: PhysicalPointF,
    size: (u32, u32),
    /// Color applied once at composition time. The active bitmap is an opacity
    /// mask, so overlapping incremental pieces cannot darken a highlighter.
    active_color: crate::gfx::Rgba8,
    /// Raw points already rendered into `active_target`; only the new tail is
    /// smoothed and tessellated on the next pointer message.
    active_rendered_points: usize,
    painted_area: u64,
    last_present: PresentStats,
}

impl InkSurface {
    /// Creates a surface without an input hook (acceptance tests and previews).
    pub fn new(device: &GraphicsDevice, bounds: PhysicalRect) -> Result<Self, GfxError> {
        Self::create(device, bounds, None, std::ptr::null_mut())
    }

    /// Creates a surface whose overlay window routes input to `hook_data`
    /// (the application's drawing session).
    pub(crate) fn new_with_hook(
        device: &GraphicsDevice,
        bounds: PhysicalRect,
        hook: WindowHook,
        hook_data: *mut core::ffi::c_void,
    ) -> Result<Self, GfxError> {
        Self::create(device, bounds, Some(hook), hook_data)
    }

    fn create(
        device: &GraphicsDevice,
        bounds: PhysicalRect,
        hook: Option<WindowHook>,
        hook_data: *mut core::ffi::c_void,
    ) -> Result<Self, GfxError> {
        let window = OverlayWindow::new_with_hook(device, bounds, hook, hook_data)?;
        let factory = device.factory().clone();
        let static_context = device.create_device_context()?;
        let active_context = device.create_device_context()?;
        let size = (bounds.width().max(1) as u32, bounds.height().max(1) as u32);
        let static_target = create_target(&static_context, size)?;
        let active_target = create_target(&active_context, size)?;
        clear_target(&static_context)?;
        clear_target(&active_context)?;
        Ok(Self {
            static_context,
            static_target,
            active_context,
            active_target,
            factory,
            window,
            origin: PhysicalPointF::new(bounds.left as f32, bounds.top as f32),
            size,
            active_color: crate::gfx::Rgba8::new(0, 0, 0, 0),
            active_rendered_points: 0,
            painted_area: 0,
            last_present: PresentStats::default(),
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.window.hwnd()
    }

    pub fn window_rect(&self) -> Result<PhysicalRect, GfxError> {
        self.window.window_rect()
    }

    pub fn show(&self) {
        self.window.show();
    }

    pub fn hide(&self) {
        self.window.hide();
    }

    pub fn set_click_through(&mut self, enabled: bool) -> Result<(), GfxError> {
        self.window.set_click_through(enabled)
    }

    /// Physical screen position of the surface's top-left corner.
    pub fn origin(&self) -> PhysicalPointF {
        self.origin
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn last_present(&self) -> &PresentStats {
        &self.last_present
    }

    /// Repaints the static cache inside `region` from the committed document.
    /// Only strokes whose bounds intersect a dirty rect are rasterized. Used by
    /// the removal edits (undo, erase, clear, close), where the pixels under the
    /// removed strokes must be rebuilt from the remaining document.
    pub fn redraw_static(
        &mut self,
        document: &InkDocument,
        region: &DirtyRegion,
    ) -> Result<(), GfxError> {
        let Self {
            static_context,
            factory,
            origin,
            painted_area,
            ..
        } = self;
        let origin = *origin;
        rasterize(
            static_context,
            factory,
            origin,
            region,
            painted_area,
            true,
            |context, factory, clip| {
                let mut renderer = StrokeRenderer::new(context, factory, origin);
                for stroke in document.strokes() {
                    if stroke.bounds().intersects(clip) {
                        renderer.draw(stroke)?;
                    }
                }
                Ok(())
            },
        )
    }

    /// Adds one just-committed stroke to the static cache without clearing.
    ///
    /// A new stroke sits on top of everything already cached, so the commit
    /// path never has to re-tessellate the strokes underneath it — the
    /// difference between one stroke of work per commit and a full redraw of
    /// every stroke in the dirty rectangle. Translucent highlighters blend over
    /// the cached pixels exactly as if the whole document had been redrawn.
    pub fn draw_committed(
        &mut self,
        stroke: &Stroke,
        region: &DirtyRegion,
    ) -> Result<(), GfxError> {
        let Self {
            static_context,
            factory,
            origin,
            painted_area,
            ..
        } = self;
        let origin = *origin;
        rasterize(
            static_context,
            factory,
            origin,
            region,
            painted_area,
            false,
            |context, factory, _clip| {
                let mut renderer = StrokeRenderer::new(context, factory, origin);
                renderer.draw(stroke)
            },
        )
    }

    /// Redraws only the new active-stroke tail inside `region` on the active
    /// layer (the region is cleared first). The layer is an opaque RGB cache; its
    /// actual color/alpha is applied once in `present`, so incremental pieces
    /// cannot create translucent overlap seams.
    pub fn draw_active(&mut self, stroke: &Stroke, region: &DirtyRegion) -> Result<(), GfxError> {
        let previous = self.active_rendered_points;
        let full_redraw =
            previous == 0 || stroke.len() < previous || region.contains_rect(stroke.bounds());
        let rendered = if full_redraw { 0 } else { previous };
        let samples = crate::ink::geometry::smooth_samples_increment(
            stroke.points(),
            stroke.style(),
            rendered,
        );
        self.active_color = stroke.style().color;
        let active_color = self.active_color;
        let Self {
            active_context,
            factory,
            origin,
            painted_area,
            ..
        } = self;
        let origin = *origin;
        let result = rasterize(
            active_context,
            factory,
            origin,
            region,
            painted_area,
            true,
            |context, factory, _clip| {
                let mut renderer = StrokeRenderer::new(context, factory, origin);
                let opaque = crate::gfx::Rgba8 {
                    a: 255,
                    ..active_color
                };
                renderer.draw_samples(&samples, opaque)
            },
        );
        self.active_rendered_points = if result.is_ok() { stroke.len() } else { 0 };
        result
    }

    /// Clears the active layer inside `region` (cancel/leave/commit path).
    pub fn clear_active(&mut self, region: &DirtyRegion) -> Result<(), GfxError> {
        self.active_color = crate::gfx::Rgba8::new(0, 0, 0, 0);
        self.active_rendered_points = 0;
        let Self {
            active_context,
            factory,
            origin,
            painted_area,
            ..
        } = self;
        rasterize(
            active_context,
            factory,
            *origin,
            region,
            painted_area,
            true,
            |_context, _factory, _clip| Ok(()),
        )
    }

    /// Composites the static and active caches and presents one frame.
    ///
    /// `dirty` is the region that changed since the previous present; it is
    /// recorded in the returned stats (and is what the rasterizers above were
    /// clipped to). The swap chain itself is flipped with the full cached
    /// texture — with a flip-model composition swap chain only a full-buffer
    /// present is guaranteed to show the previous frame's unchanged pixels, and
    /// no CPU pixel data is transferred either way.
    pub fn present(&mut self, dirty: DirtyRegion) -> Result<PresentStats, GfxError> {
        let stats = PresentStats {
            rects: dirty.rects().len(),
            area: dirty.area(),
            painted_area: self.painted_area,
            uploaded_bytes: 0,
            region: dirty,
        };
        self.painted_area = 0;

        let destination = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: self.size.0 as f32,
            bottom: self.size.1 as f32,
        };
        let static_target = &self.static_target;
        let active_target = &self.active_target;
        let active_color = self.active_color;
        let window = &mut self.window;
        window.composition_mut().draw(|context, _factory| {
            // Fill and draw primitives on a D2D device context are void; any
            // device loss surfaces at EndDraw, which `WindowComposition::draw`
            // reports.
            unsafe {
                context.DrawBitmap(
                    static_target,
                    Some(&destination as *const D2D_RECT_F),
                    1.0,
                    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    None,
                    None,
                );
                if active_color.a != 0 {
                    // RGB is opaque in the active cache; alpha is applied once
                    // here, so incremental overlap cannot darken highlighters.
                    context.DrawBitmap(
                        active_target,
                        Some(&destination as *const D2D_RECT_F),
                        f32::from(active_color.a) / 255.0,
                        D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                        None,
                        None,
                    );
                }
            }
            Ok(())
        })?;
        self.last_present = stats.clone();
        Ok(stats)
    }
}

fn create_target(context: &ID2D1DeviceContext, size: (u32, u32)) -> Result<ID2D1Bitmap1, GfxError> {
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
        colorContext: ManuallyDrop::new(None),
    };
    let bitmap = unsafe {
        context.CreateBitmap(
            D2D_SIZE_U {
                width: size.0,
                height: size.1,
            },
            None,
            0,
            &properties,
        )
    }
    .map_err(failed("ID2D1DeviceContext::CreateBitmap"))?;
    let image: ID2D1Image = bitmap
        .cast()
        .map_err(failed("ID2D1Bitmap1 -> ID2D1Image"))?;
    unsafe { context.SetTarget(&image) };
    Ok(bitmap)
}

fn clear_target(context: &ID2D1DeviceContext) -> Result<(), GfxError> {
    let transparent = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    unsafe {
        context.BeginDraw();
        context.Clear(Some(&transparent));
        context
            .EndDraw(None, None)
            .map_err(failed("ID2D1DeviceContext::EndDraw"))?;
    }
    Ok(())
}

/// Clears (when `clear`) and redraws each rectangle of `region` on `context`,
/// then runs `draw` (clipped to the rectangle). `EndDraw` always runs so the
/// context never stays inside a BeginDraw frame after a failed primitive.
fn rasterize(
    context: &ID2D1DeviceContext,
    factory: &ID2D1Factory1,
    origin: PhysicalPointF,
    region: &DirtyRegion,
    painted_area: &mut u64,
    clear: bool,
    mut draw: impl FnMut(&ID2D1DeviceContext, &ID2D1Factory1, DirtyRect) -> Result<(), GfxError>,
) -> Result<(), GfxError> {
    if region.is_empty() {
        return Ok(());
    }
    let transparent = D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    let clear_brush = if clear {
        Some(
            unsafe { context.CreateSolidColorBrush(&transparent, None) }
                .map_err(failed("ID2D1RenderTarget::CreateSolidColorBrush(clear)"))?,
        )
    } else {
        None
    };

    unsafe {
        context.BeginDraw();
    }
    let mut result = Ok(());
    for rect in region.rects() {
        let local = local_rect(*rect, origin);
        // Clip, clear and blend calls are void on the D2D device context; a
        // device error only surfaces at EndDraw, which always runs below.
        unsafe {
            context.PushAxisAlignedClip(&local, D2D1_ANTIALIAS_MODE_ALIASED);
            if let Some(brush) = &clear_brush {
                context.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_COPY);
                context.FillRectangle(&local, brush);
                context.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
            }
        }
        let step = draw(context, factory, *rect);
        unsafe {
            context.PopAxisAlignedClip();
        }
        if let Err(error) = step {
            result = Err(error);
            break;
        }
        *painted_area += rect.area();
    }
    let end = unsafe { context.EndDraw(None, None) }.map_err(failed("ID2D1DeviceContext::EndDraw"));
    result?;
    end?;
    Ok(())
}

fn local_rect(rect: DirtyRect, origin: PhysicalPointF) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left as f32 - origin.x,
        top: rect.top as f32 - origin.y,
        right: rect.right as f32 - origin.x,
        bottom: rect.bottom as f32 - origin.y,
    }
}
