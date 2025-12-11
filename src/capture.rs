use nkcore::euclid::*;
use nkcore::tap::*;
use nkcore::*;
use nkcore::anyhow::anyhow;
use windows::core::Interface as _;
use windows::{
    Graphics::*,
    Graphics::Capture::*,
    Graphics::DirectX::*,
    Graphics::DirectX::Direct3D11::*,
    UI::*,
    Win32::Foundation::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::Graphics::Dxgi::*,
    Win32::Graphics::Direct3D11::*,
    Win32::System::WinRT::Direct3D11::*,
};

pub struct CaptureSession {
    d3d11_device: ID3D11Device,
    winrt_device: IDirect3DDevice,
    frame_pool: Direct3D11CaptureFramePool,
    frame_pool_size: SizeInt32,

    #[expect(unused, reason = "To keep the GraphicsCaptureSession object alive")]
    session: GraphicsCaptureSession,
}

impl CaptureSession {
    pub fn new(device: &ID3D11Device, capture_item: &GraphicsCaptureItem) -> anyhow::Result<Self> {
        let winrt_device =
            Self::create_winrt_device_from_d3d11_device(device)
                .context("failed to create WinRT device from D3D11 device")?;

        let frame_pool_size = SizeInt32 { Width: 1, Height: 1 };
        let frame_pool = api_call! {
            Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                frame_pool_size)
        }?;

        let session = api_call!(frame_pool.CreateCaptureSession(capture_item))?;
        api_call!(session.SetIsCursorCaptureEnabled(false))?;
        api_call!(session.StartCapture())?;

        Ok(Self {
            d3d11_device: device.clone(),
            winrt_device,
            frame_pool,
            frame_pool_size,
            session,
        })
    }

    pub fn from_window(device: &ID3D11Device, window_handle: HWND) -> anyhow::Result<Self> {
        let capture_item =
            api_call!(GraphicsCaptureItem::TryCreateFromWindowId(WindowId {
                Value: window_handle.0 as _,
            }))?;
        Self::new(device, &capture_item)
    }

    pub fn get_next_frame(&mut self)
        -> anyhow::Result<Option<(ID3D11Texture2D, Size2D<u32>)>> {
        let mut last_frame = None;
        while let Ok(frame) = self.frame_pool.TryGetNextFrame() {
            last_frame = Some(frame);
        }

        let Some(frame) = last_frame else {
            return Ok(None);
        };

        let frame_size = frame.ContentSize()?;

        if frame_size != self.frame_pool_size {
            self.frame_pool_size = frame_size;
            self.frame_pool.Recreate(
                &self.winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                frame_size)?;
            log::info!(
                "capturing frame pool resized to {}x{}",
                frame_size.Width,
                frame_size.Height);

            // Skip this frame since we just resized.
            return Ok(None);
        }

        #[expect(clippy::multiple_unsafe_ops_per_block)]
        let frame = unsafe {
            frame
                .pipe(|frame| api_call!(frame.Surface()))?
                .pipe(|frame| api_call!(frame.cast::<IDirect3DDxgiInterfaceAccess>()))?
                .pipe(|frame| api_call!(frame.GetInterface::<ID3D11Texture2D>()))?
        };

        let frame_size =
            Size2D::new(
                frame_size.Width as u32,
                frame_size.Height as u32);

        Ok(Some((frame, frame_size)))
    }

    fn create_winrt_device_from_d3d11_device(device: &ID3D11Device)
        -> anyhow::Result<IDirect3DDevice> {
        Ok(device
            .pipe(|device| api_call!(device.cast::<IDXGIDevice>()))?
            .pipe(|device| api_call!(unsafe { CreateDirect3D11DeviceFromDXGIDevice(&device) }))?
            .pipe(|device| api_call!(device.cast::<IDirect3DDevice>()))?)
    }

    fn create_texture(device: &ID3D11Device, format: DXGI_FORMAT, size: SizeInt32)
        -> anyhow::Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.Width as _,
            Height: size.Height as _,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as _,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        Ok({
            out_var_or_err(|out| api_call!(unsafe {
                device.CreateTexture2D(
                    &raw const desc,
                    None,
                    Some(out))
            }))?.expect("unexpected null pointer")
        })
    }
}
