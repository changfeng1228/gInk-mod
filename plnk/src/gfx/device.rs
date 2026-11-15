//! Native graphics device shared by the window layers.
//!
//! One `GraphicsDevice` owns the D3D11 device that backs Direct2D and the DXGI
//! composition swap chains, the Direct2D factory/device and the
//! DirectComposition desktop device. All of these are thread-affine; the struct
//! is deliberately `!Send`/`!Sync` so it can only be used from the UI thread
//! that created it (plan §6.2).

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use windows::Win32::Foundation::{E_FAIL, HWND};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1CreateFactory,
    ID2D1Device, ID2D1DeviceContext, ID2D1Factory1,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice2, IDCompositionDesktopDevice, IDCompositionTarget,
    IDCompositionVisual2,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGIOutput,
    IDXGISwapChain1,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::core::{BOOL, Error as WindowsError, HRESULT, IUnknown, Interface};

/// A whole-device or whole-window graphics failure.
///
/// These errors are reported to the user with the operation that failed and
/// the underlying Windows error message; they are never panics.
#[derive(Debug)]
pub enum GfxError {
    /// A Windows graphics/composition call failed. `context` names the call so
    /// the user can tell which stage failed.
    Operation {
        context: &'static str,
        source: WindowsError,
    },
    /// A window or composition surface was given a non-positive size.
    ZeroSize { width: u32, height: u32 },
}

impl fmt::Display for GfxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation { context, source } => {
                write!(
                    formatter,
                    "graphics operation failed while {context}: {source}"
                )
            }
            Self::ZeroSize { width, height } => write!(
                formatter,
                "window or surface size must be positive (got {width}x{height})"
            ),
        }
    }
}

impl Error for GfxError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Operation { source, .. } => Some(source),
            Self::ZeroSize { .. } => None,
        }
    }
}

pub(crate) fn failed(context: &'static str) -> impl FnOnce(WindowsError) -> GfxError {
    move |source| GfxError::Operation { context, source }
}

/// Presentation timestamps reported by the DirectComposition device.
#[derive(Clone, Copy, Debug)]
pub struct CompositionFrameStats {
    /// QPC timestamp of the last frame composed by DWM for this device.
    pub last_frame_time: i64,
    /// QPC ticks per second, used to convert the timestamps to durations.
    pub time_frequency: i64,
    /// The composition rate DWM currently reports for this device, in Hz.
    pub current_refresh_hz: f64,
    /// DWM's estimate of the next composition frame (QPC ticks).
    pub next_estimated_frame_time: i64,
}

/// The single native graphics device of the process (UI-thread owned).
pub struct GraphicsDevice {
    d2d_factory: ID2D1Factory1,
    d2d_device: ID2D1Device,
    d3d_device: ID3D11Device,
    dxgi_factory: IDXGIFactory2,
    dcomp: IDCompositionDesktopDevice,
    _com: ComApartment,
    _not_send: PhantomData<Rc<()>>,
}

impl GraphicsDevice {
    /// Creates the D3D11 device, the Direct2D factory/device and the
    /// DirectComposition desktop device.
    pub fn new() -> Result<Self, GfxError> {
        let com = ComApartment::initialize()?;

        let mut d3d_device: Option<ID3D11Device> = None;
        unsafe {
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                None,
                None,
            )
        }
        .map_err(failed("D3D11CreateDevice"))?;
        let d3d_device = d3d_device.ok_or_else(|| GfxError::Operation {
            context: "D3D11CreateDevice (no device returned)",
            source: WindowsError::from_hresult(E_FAIL),
        })?;

        let dxgi_device: IDXGIDevice = d3d_device
            .cast()
            .map_err(failed("ID3D11Device -> IDXGIDevice"))?;
        let adapter =
            unsafe { dxgi_device.GetAdapter() }.map_err(failed("IDXGIDevice::GetAdapter"))?;
        let dxgi_factory: IDXGIFactory2 = unsafe { adapter.GetParent() }
            .map_err(failed("IDXGIAdapter::GetParent(IDXGIFactory2)"))?;

        let d2d_factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
                .map_err(failed("D2D1CreateFactory"))?;
        let d2d_device = unsafe { d2d_factory.CreateDevice(&dxgi_device) }
            .map_err(failed("ID2D1Factory1::CreateDevice"))?;

        let d2d_unknown: IUnknown = d2d_device
            .cast()
            .map_err(failed("ID2D1Device -> IUnknown"))?;
        let dcomp: IDCompositionDesktopDevice =
            unsafe { DCompositionCreateDevice2(Some(&d2d_unknown)) }
                .map_err(failed("DCompositionCreateDevice2"))?;

        Ok(Self {
            d2d_factory,
            d2d_device,
            d3d_device,
            dxgi_factory,
            dcomp,
            _com: com,
            _not_send: PhantomData,
        })
    }

    pub(crate) fn create_device_context(&self) -> Result<ID2D1DeviceContext, GfxError> {
        unsafe {
            self.d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
        }
        .map_err(failed("ID2D1Device::CreateDeviceContext"))
    }

    /// The Direct2D factory, shared by the drawing code that needs path
    /// geometries and stroke styles (toolbar icons).
    pub(crate) fn factory(&self) -> &ID2D1Factory1 {
        &self.d2d_factory
    }

    pub(crate) fn create_swap_chain(
        &self,
        width: u32,
        height: u32,
    ) -> Result<IDXGISwapChain1, GfxError> {
        if width == 0 || height == 0 {
            return Err(GfxError::ZeroSize { width, height });
        }
        let description = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let d3d_unknown: IUnknown = self
            .d3d_device
            .cast()
            .map_err(failed("ID3D11Device -> IUnknown"))?;
        unsafe {
            self.dxgi_factory.CreateSwapChainForComposition(
                &d3d_unknown,
                &description,
                None::<&IDXGIOutput>,
            )
        }
        .map_err(failed("IDXGIFactory2::CreateSwapChainForComposition"))
    }

    pub(crate) fn compose(&self) -> &IDCompositionDesktopDevice {
        &self.dcomp
    }

    pub(crate) fn create_composition_target(
        &self,
        hwnd: HWND,
    ) -> Result<IDCompositionTarget, GfxError> {
        unsafe { self.dcomp.CreateTargetForHwnd(hwnd, true) }
            .map_err(failed("IDCompositionDevice::CreateTargetForHwnd"))
    }

    pub(crate) fn create_visual(&self) -> Result<IDCompositionVisual2, GfxError> {
        unsafe { self.dcomp.CreateVisual() }.map_err(failed("IDCompositionDevice::CreateVisual"))
    }

    pub(crate) fn commit(&self) -> Result<(), GfxError> {
        unsafe { self.dcomp.Commit() }.map_err(failed("IDCompositionDevice::Commit"))
    }

    /// Reads DWM's presentation timestamps for this composition device. Used
    /// by the animation acceptance test and the decision-gate probe; it is
    /// never part of the animation rendering path.
    pub fn frame_statistics(&self) -> Result<CompositionFrameStats, GfxError> {
        let statistics = unsafe { self.dcomp.GetFrameStatistics() }
            .map_err(failed("IDCompositionDevice::GetFrameStatistics"))?;
        let rate = statistics.currentCompositionRate;
        let current_refresh_hz = if rate.Denominator != 0 {
            f64::from(rate.Numerator) / f64::from(rate.Denominator)
        } else {
            0.0
        };
        Ok(CompositionFrameStats {
            last_frame_time: statistics.lastFrameTime,
            time_frequency: statistics.timeFrequency,
            current_refresh_hz,
            next_estimated_frame_time: statistics.nextEstimatedFrameTime,
        })
    }
}

/// Balances `CoInitializeEx` on the UI thread.
struct ComApartment {
    uninitialize_on_drop: bool,
}

impl ComApartment {
    fn initialize() -> Result<Self, GfxError> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            // S_OK (first initialization) and S_FALSE (already initialized)
            // must both be balanced by CoUninitialize.
            Ok(Self {
                uninitialize_on_drop: true,
            })
        } else {
            Err(GfxError::Operation {
                context: "CoInitializeEx(APARTMENTTHREADED)",
                source: WindowsError::from_hresult(HRESULT(result.0)),
            })
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize_on_drop {
            // `GraphicsDevice` is !Send, so this drop runs on the same UI
            // thread that called CoInitializeEx.
            unsafe { CoUninitialize() };
        }
    }
}
