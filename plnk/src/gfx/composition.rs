//! DirectComposition visual trees and DWM-driven visual animations.
//!
//! The window layers draw into a premultiplied-alpha DXGI composition swap
//! chain and attach it to a DirectComposition visual. Animations are expressed
//! as `IDCompositionAnimation` polynomials: the program sets start, end,
//! duration and easing once and DWM evaluates the curve on every composition
//! frame, without any per-frame application callback (plan §6.1/§6.3).

use std::mem::ManuallyDrop;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Image,
};
use windows::Win32::Graphics::DirectComposition::{
    IDCompositionAnimation, IDCompositionDesktopDevice, IDCompositionTarget, IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{DXGI_PRESENT, IDXGISurface, IDXGISwapChain1};
use windows::core::{IUnknown, Interface};

use crate::gfx::Rgba8;
use crate::gfx::device::{GfxError, GraphicsDevice, failed};
use crate::platform::dpi::PhysicalPoint;

/// Easing curves supported by the DirectComposition animation primitive.
///
/// Each variant is encoded as cubic polynomial segments evaluated by DWM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Easing {
    /// Constant velocity.
    #[default]
    Linear,
    /// Fast start, slow arrival: `1 - (1 - t)^3`.
    EaseOutCubic,
    /// Symmetric acceleration/deceleration.
    EaseInOutCubic,
}

/// A swap-chain backed visual attached to one top-level window.
pub(crate) struct WindowComposition {
    dcomp: IDCompositionDesktopDevice,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
    swap_chain: IDXGISwapChain1,
    context: ID2D1DeviceContext,
    _target_bitmap: ID2D1Bitmap1,
    // DirectComposition evaluates the animation objects while they run, so the
    // interfaces must stay alive until the next animation replaces them.
    animations: Vec<IDCompositionAnimation>,
    width: u32,
    height: u32,
}

impl WindowComposition {
    pub(crate) fn new(
        device: &GraphicsDevice,
        hwnd: HWND,
        width: u32,
        height: u32,
    ) -> Result<Self, GfxError> {
        let swap_chain = device.create_swap_chain(width, height)?;
        let context = device.create_device_context()?;
        let target_bitmap = create_target_bitmap(&context, &swap_chain)?;
        let target_image: ID2D1Image = target_bitmap
            .cast()
            .map_err(failed("ID2D1Bitmap1 -> ID2D1Image"))?;
        unsafe { context.SetTarget(&target_image) };

        let dcomp = device.compose().clone();
        let target = device.create_composition_target(hwnd)?;
        let visual = device.create_visual()?;
        let content: IUnknown = swap_chain
            .cast()
            .map_err(failed("IDXGISwapChain1 -> IUnknown"))?;
        unsafe {
            visual
                .SetContent(&content)
                .map_err(failed("IDCompositionVisual::SetContent"))?;
            target
                .SetRoot(&visual)
                .map_err(failed("IDCompositionTarget::SetRoot"))?;
        }
        device.commit()?;

        Ok(Self {
            dcomp,
            _target: target,
            visual,
            swap_chain,
            context,
            _target_bitmap: target_bitmap,
            animations: Vec::new(),
            width,
            height,
        })
    }

    pub(crate) fn fill(&mut self, color: Rgba8) -> Result<(), GfxError> {
        let transparent = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        let brush_color = D2D1_COLOR_F {
            r: f32::from(color.r) / 255.0,
            g: f32::from(color.g) / 255.0,
            b: f32::from(color.b) / 255.0,
            a: f32::from(color.a) / 255.0,
        };
        let area = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: self.width as f32,
            bottom: self.height as f32,
        };
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&transparent));
            let brush = self
                .context
                .CreateSolidColorBrush(&brush_color, None)
                .map_err(failed("ID2D1DeviceContext::CreateSolidColorBrush"))?;
            self.context.FillRectangle(&area, &brush);
            self.context
                .EndDraw(None, None)
                .map_err(failed("ID2D1DeviceContext::EndDraw"))?;
        }
        unsafe {
            self.swap_chain
                .Present(0, DXGI_PRESENT(0))
                .ok()
                .map_err(failed("IDXGISwapChain1::Present"))?;
        }
        Ok(())
    }

    /// Places the visual at a fixed offset without animating.
    pub(crate) fn set_offset(&mut self, offset: PhysicalPoint) -> Result<(), GfxError> {
        unsafe {
            self.visual
                .SetOffsetX2(offset.x as f32)
                .map_err(failed("IDCompositionVisual::SetOffsetX"))?;
            self.visual
                .SetOffsetY2(offset.y as f32)
                .map_err(failed("IDCompositionVisual::SetOffsetY"))?;
            self.dcomp
                .Commit()
                .map_err(failed("IDCompositionDevice::Commit"))?;
        }
        Ok(())
    }

    /// Animates the visual from `from` to `to`. DWM evaluates the curve; the
    /// application receives no per-frame callbacks.
    pub(crate) fn animate_offset(
        &mut self,
        from: PhysicalPoint,
        to: PhysicalPoint,
        duration: Duration,
        easing: Easing,
    ) -> Result<(), GfxError> {
        let seconds = duration.as_secs_f64();
        if seconds <= 0.0 {
            return self.set_offset(to);
        }
        let x_animation =
            build_animation(&self.dcomp, from.x as f32, to.x as f32, seconds, easing)?;
        let y_animation =
            build_animation(&self.dcomp, from.y as f32, to.y as f32, seconds, easing)?;
        unsafe {
            self.visual
                .SetOffsetX(&x_animation)
                .map_err(failed("IDCompositionVisual::SetOffsetX(animation)"))?;
            self.visual
                .SetOffsetY(&y_animation)
                .map_err(failed("IDCompositionVisual::SetOffsetY(animation)"))?;
            self.dcomp
                .Commit()
                .map_err(failed("IDCompositionDevice::Commit"))?;
        }
        // Keep the animation objects alive for the duration DWM evaluates them.
        self.animations.clear();
        self.animations.push(x_animation);
        self.animations.push(y_animation);
        Ok(())
    }
}

fn build_animation(
    dcomp: &IDCompositionDesktopDevice,
    from: f32,
    to: f32,
    seconds: f64,
    easing: Easing,
) -> Result<IDCompositionAnimation, GfxError> {
    let animation = unsafe { dcomp.CreateAnimation() }
        .map_err(failed("IDCompositionDevice::CreateAnimation"))?;
    let delta = f64::from(to) - f64::from(from);
    let squared = seconds * seconds;
    let cubed = squared * seconds;
    unsafe {
        match easing {
            Easing::Linear => {
                animation
                    .AddCubic(0.0, from, (delta / seconds) as f32, 0.0, 0.0)
                    .map_err(failed("IDCompositionAnimation::AddCubic"))?;
            }
            Easing::EaseOutCubic => {
                // progress(t) = 3t - 3t^2 + t^3 over [0, seconds]
                animation
                    .AddCubic(
                        0.0,
                        from,
                        (3.0 * delta / seconds) as f32,
                        (-3.0 * delta / squared) as f32,
                        (delta / cubed) as f32,
                    )
                    .map_err(failed("IDCompositionAnimation::AddCubic"))?;
            }
            Easing::EaseInOutCubic => {
                // First half: 4t^3; second half: 1 - 4(1-t)^3.
                animation
                    .AddCubic(0.0, from, 0.0, 0.0, (4.0 * delta / cubed) as f32)
                    .map_err(failed("IDCompositionAnimation::AddCubic"))?;
                let middle = f64::from(from) + delta / 2.0;
                animation
                    .AddCubic(
                        seconds / 2.0,
                        middle as f32,
                        (3.0 * delta / seconds) as f32,
                        (-6.0 * delta / squared) as f32,
                        (4.0 * delta / cubed) as f32,
                    )
                    .map_err(failed("IDCompositionAnimation::AddCubic"))?;
            }
        }
        animation
            .End(seconds, to)
            .map_err(failed("IDCompositionAnimation::End"))?;
    }
    Ok(animation)
}

fn create_target_bitmap(
    context: &ID2D1DeviceContext,
    swap_chain: &IDXGISwapChain1,
) -> Result<ID2D1Bitmap1, GfxError> {
    let back_buffer: IDXGISurface =
        unsafe { swap_chain.GetBuffer(0) }.map_err(failed("IDXGISwapChain1::GetBuffer"))?;
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        colorContext: ManuallyDrop::new(None),
    };
    unsafe { context.CreateBitmapFromDxgiSurface(&back_buffer, Some(&properties)) }
        .map_err(failed("ID2D1DeviceContext::CreateBitmapFromDxgiSurface"))
}
