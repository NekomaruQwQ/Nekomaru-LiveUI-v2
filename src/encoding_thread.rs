use nkcore::euclid::*;
use nkcore::*;

use std::sync::Arc;
use std::sync::mpsc;
use std::thread::Builder as ThreadBuilder;
use std::thread::JoinHandle as ThreadHandle;

use windows::Win32::{
    Graphics::Dxgi::Common::*,
    Graphics::Direct3D11::*,
    System::Com::*,
};

use crate::converter::FormatConverter;
use crate::encoder::H264Encoder;
use crate::stream::StreamManager;

/// A captured frame to be sent to the encoding thread
pub struct CaptureFrame {
    /// Texture containing the captured frame (BGRA format)
    pub texture: ID3D11Texture2D,
    /// Timestamp in microseconds
    pub timestamp_us: u64,
}

/// Encoding thread handle
///
/// Runs encoding pipeline on a separate thread to avoid blocking the UI thread.
/// The thread owns the encoder, converter, and NV12 staging texture.
pub struct EncodingThread {
    frame_sender: mpsc::Sender<CaptureFrame>,
    thread_handle: Option<ThreadHandle<()>>,
}

impl EncodingThread {
    /// Create and start a new encoding thread.
    ///
    /// # Arguments
    /// * `device` - D3D11 device (cloned for thread)
    /// * `device_context` - D3D11 device context (cloned for thread)
    /// * `width` - Video width
    /// * `height` - Video height
    /// * `stream_manager` - Shared stream manager for pushing encoded frames
    pub fn new(
        device: ID3D11Device,
        device_context: ID3D11DeviceContext,
        frame_size: Size2D<u32>,
        stream_manager: Arc<StreamManager>)
        -> anyhow::Result<Self> {
        let (frame_sender, frame_receiver) = mpsc::channel::<CaptureFrame>();
        let thread_handle =
            ThreadBuilder::new()
                .name("EncodingThread".to_string())
                .spawn(move || {
                    encoding_thread_main(
                        device,
                        device_context,
                        frame_size,
                        stream_manager,
                        frame_receiver)
                        .expect("failed to initialize encoding thread")
                })?;

        Ok(Self {
            frame_sender,
            thread_handle: Some(thread_handle),
        })
    }

    /// Send a captured frame to the encoding thread for processing.
    ///
    /// Non-blocking. If the channel is full or disconnected, logs a warning and drops the frame.
    ///
    /// # Arguments
    /// * `frame` - Captured frame with texture and timestamp
    pub fn send_frame(&self, frame: CaptureFrame) {
        if let Err(e) = self.frame_sender.send(frame) {
            log::warn!("Failed to send frame to encoding thread: {e:?}");
        }
    }
}

impl Drop for EncodingThread {
    fn drop(&mut self) {
        // Thread will exit when sender is dropped (channel closes)
        // Wait for thread to finish
        if let Some(handle) = self.thread_handle.take() {
            if let Err(e) = handle.join() {
                log::error!("Encoding thread panicked: {e:?}");
            }
        }
    }
}

/// Main function for the encoding thread
fn encoding_thread_main(
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,
    frame_size: Size2D<u32>,
    stream_manager: Arc<StreamManager>,
    frame_receiver: mpsc::Receiver<CaptureFrame>)
    -> anyhow::Result<()> {
    log::info!("Encoding thread started");

    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;

    // Use fixed encoder size (1920x1200) regardless of capture size
    let encoder_size = Size2D::new(1920, 1200);

    log::info!("Initializing encoder with fixed size: {}x{}", encoder_size.width, encoder_size.height);

    // Initialize encoder
    let mut encoder =
        H264Encoder::new(&device, encoder_size, 60, 8_000_000)
            .context("failed to create H.264 encoder")?;

    // Initialize format converter
    let mut converter =
        FormatConverter::new(&device, &device_context)
            .context("failed to create format converter")?;

    // Create NV12 staging texture (reused for all frames)
    let nv12_texture =
        create_nv12_texture(&device, encoder_size)
        .context("failed to create NV12 staging texture")?;

    log::info!("encoding thread initialized, waiting for frames...");

    // Process frames until channel closes
    while let Ok(frame) = frame_receiver.recv() {
        // Convert resampled BGRA -> NV12
        if let Err(e) = converter.convert(&frame.texture, &nv12_texture) {
            log::warn!("format conversion failed: {e:?}");
            continue;  // Skip this frame
        }

        // Encode to H.264
        match encoder.encode_frame(&nv12_texture, frame.timestamp_us) {
            Ok(nal_units) if !nal_units.is_empty() => {
                // Push to stream manager
                if let Err(e) = stream_manager.push_frame(nal_units) {
                    log::warn!("failed to push frame to stream: {e:?}");
                }
            }
            Ok(_) => {
                // Empty NAL units (encoder buffering) - this is normal
            }
            Err(e) => {
                log::error!("frame encoding failed: {e:?}");
                // Continue - try to recover with next frame
            }
        }
    }

    log::info!("encoding thread stopped");
    Ok(())
}

fn create_nv12_texture(device: &ID3D11Device, size: Size2D<u32>) -> anyhow::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
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

/// Creates a BGRA render target texture with RTV and SRV
fn create_bgra_render_target(
    device: &ID3D11Device,
    size: Size2D<u32>)
    -> anyhow::Result<(ID3D11Texture2D, ID3D11RenderTargetView, ID3D11ShaderResourceView)> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };

    let texture = out_var_or_err(|out| api_call!(unsafe {
        device.CreateTexture2D(&raw const desc, None, Some(out))
    }))?.ok_or_else(|| anyhow::anyhow!("failed to create BGRA texture"))?;

    let rtv = out_var_or_err(|out| api_call!(unsafe {
        device.CreateRenderTargetView(&texture, None, Some(out))
    }))?.ok_or_else(|| anyhow::anyhow!("failed to create RTV"))?;

    let srv = out_var_or_err(|out| api_call!(unsafe {
        device.CreateShaderResourceView(&texture, None, Some(out))
    }))?.ok_or_else(|| anyhow::anyhow!("failed to create SRV"))?;

    Ok((texture, rtv, srv))
}

/// Creates a shader resource view for a texture
fn create_shader_resource_view(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D)
    -> anyhow::Result<ID3D11ShaderResourceView> {
    out_var_or_err(|out| api_call!(unsafe {
        device.CreateShaderResourceView(texture, None, Some(out))
    }))?.ok_or_else(|| anyhow::anyhow!("failed to create SRV"))
}
