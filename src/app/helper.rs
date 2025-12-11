use nkcore::*;
use nkcore::euclid::Size2D;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    raw_window_handle::HasWindowHandle as _,
    raw_window_handle::RawWindowHandle,
    window::Window,
    window::WindowId,
};

use windows::core::Interface as _;
use windows::{
    Win32::Foundation::*,
    Win32::Graphics::{
        Dxgi::*,
        Direct3D::*,
        Direct3D11::*,
    },
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};

pub fn create_device() -> anyhow::Result<(IDXGIFactory6, ID3D11Device, ID3D11DeviceContext)> {
    let dxgi_factory =
        api_call!(unsafe { CreateDXGIFactory::<IDXGIFactory6>() })?;
    let dxgi_adapter =
        api_call!(unsafe {
            dxgi_factory.EnumAdapterByGpuPreference::<IDXGIAdapter>(
                0,
                DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
        })?;

    let DXGI_ADAPTER_DESC { Description: adapter_name, .. } =
        api_call!(unsafe { dxgi_adapter.GetDesc() })?;
    let adapter_name =
        unsafe { widestring::U16CString::from_ptr_str(adapter_name.as_ptr()) }
            .to_string_lossy();
    log::info!("device: {adapter_name}");

    let mut device = None;
    let mut device_context = None;
    api_call!(unsafe {
        D3D11CreateDevice(
            &dxgi_adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT |
            D3D11_CREATE_DEVICE_BGRA_SUPPORT |
            cfg!(debug_assertions)
                .then_some(D3D11_CREATE_DEVICE_DEBUG)
                .unwrap_or_default(),
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            Some(&raw mut device_context))
    })?;

    let device =
        device
            .ok_or_else(|| anyhow::anyhow!("failed to create D3D11 device"))?;
    let device_context =
        device_context
            .ok_or_else(|| anyhow::anyhow!("failed to create D3D11 device context"))?;

    let multithread = api_call!(unsafe { device.cast::<ID3D11Multithread>() })?;
    let _ = unsafe { multithread.SetMultithreadProtected(true) };

    Ok((dxgi_factory, device, device_context))
}

pub fn create_texture_2d(
    device: &ID3D11Device,
    size: Size2D<u32>,
    format: DXGI_FORMAT,
    bind_flags: &[D3D11_BIND_FLAG])
    -> anyhow::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags:
            bind_flags
                .iter()
                .map(|flag| flag.0 as u32)
                .sum(),
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };

    out_var_or_err(|out| api_call!(unsafe {
        device.CreateTexture2D(
            &raw const desc,
            None,
            Some(out))
    }))?.ok_or_else(|| anyhow::anyhow!("failed to create texture"))
}

pub fn create_srv_for_texture_2d(device: &ID3D11Device, texture: &ID3D11Texture2D)
    -> anyhow::Result<ID3D11ShaderResourceView> {
    Ok({
        out_var_or_err(|out| api_call!(unsafe {
            device.CreateShaderResourceView(
                texture,
                None,
                Some(out))
        }))?.expect("unexpected null pointer")
    })
}

pub fn create_rtv_for_texture_2d(device: &ID3D11Device, texture: &ID3D11Texture2D)
    -> anyhow::Result<ID3D11RenderTargetView> {
    Ok({
        out_var_or_err(|out| api_call!(unsafe {
            device.CreateRenderTargetView(
                texture,
                None,
                Some(out))
        }))?.expect("unexpected null pointer")
    })
}

#[expect(
    clippy::panic_in_result_fn,
    reason = "running on an unexpected platform is always an unrecoverable error")]
pub fn get_hwnd_from_window(window: &Window) -> anyhow::Result<HWND> {
    if let RawWindowHandle::Win32(hwnd) = window.window_handle()?.as_raw() {
        Ok(HWND(hwnd.hwnd.get() as _))
    } else {
        panic!("unexpected platform");
    }
}

pub struct AppWrapper<T>(pub Option<T>);

// noinspection RsSortImplTraitMembers
impl ApplicationHandler for AppWrapper<super::LiveApp> {
    fn suspended(&mut self, _: &ActiveEventLoop) {
        let _ = self.0.take();
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let app = super::LiveApp::new(event_loop).expect("fatal error creating app");
        let _ = self.0.replace(app);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        if event == WindowEvent::CloseRequested {
            event_loop.exit();
        }

        if let Some(app) = self.0.as_mut() {
            app.on_window_event(window_id, event);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Request continuous redraws for frontend window to trigger frame capture
        if let Some(app) = self.0.as_ref() {
            app.main_window.request_redraw();
        }
    }
}
