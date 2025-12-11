mod helper;
use helper::*;

use crate::capture::CaptureSession;
use crate::encoding_thread::EncodingThread;
use crate::encoding_thread::CaptureFrame;
use crate::stream::StreamManager;

use nkcore::euclid::*;
use nkcore::*;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use wry::WebView;
use wry::WebViewBuilder;

use winit::{
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::EventLoop,
    event_loop::ActiveEventLoop,
    window::Window,
    window::WindowId,
    window::WindowButtons,
};

use windows::core::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Direct3D11::*;

use crate::resample::ResamplePass;

pub fn run() {
    EventLoop::<()>::new()
        .expect("failed to create event loop")
        .pipe(|event_loop| event_loop.run_app(&mut AppWrapper::<LiveApp>(None)))
        .expect("failed to run event loop");
}

#[expect(dead_code, reason = "to keep various resources alive")]
struct LiveApp {
    main_capture: CaptureSession,

    // Encoding pipeline
    encoding_thread: EncodingThread,
    stream_manager: Arc<StreamManager>,

    // D3D11 device and context
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,

    // Staging texture for copying captured frames
    staging_texture: ID3D11Texture2D,
    staging_texture_rtv: ID3D11RenderTargetView,
    resample_pass: ResamplePass,

    frontend_window: Window,
    frontend_webview: WebView,

    // control_window: Window,
}

impl LiveApp {
    const STREAM_SIZE: Size2D<u32> = Size2D::new(1920, 1200);

    fn new(event_loop: &ActiveEventLoop) -> anyhow::Result<Self> {
        use windows::Win32::UI::WindowsAndMessaging::FindWindowA;

        let (dxgi_factory, device, device_context) =
            create_device()
                .context("failed to create graphics context")?;
        let resample_pass =
            ResamplePass::new(&device)
                .context("failed to create resample pass")?;
        // Create staging texture for copying captured frames
        let staging_texture =
            create_staging_texture(&device, Self::STREAM_SIZE)
                .context("failed to create staging texture")?;
        let staging_texture_rtv =
            out_var_or_err(|out| api_call!(unsafe {
                device.CreateRenderTargetView(
                    &staging_texture,
                    None,
                    Some(out))
            }))?.ok_or_else(|| anyhow::anyhow!("failed to create RTV for staging texture"))?;

        // Start main capture session (capture LiveUI window)
        let main_capture_target = api_call!(unsafe {
            FindWindowA(
                PCSTR::default(),
                PCSTR(c"Nekomaru LiveUI v1".as_ptr().cast()))
        })?;
        let main_capture =
            CaptureSession::capture_window(&device, &device_context, main_capture_target)
                .context("failed to start main capture session")?;

        // Create stream manager (shared between encoding thread and protocol handler)
        let stream_manager = Arc::new(StreamManager::new(60));  // 60 frame buffer (~1 second)
        let stream_manager_for_protocol = Arc::clone(&stream_manager);

        let frontend_window = api_call! {
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Nekomaru LiveUI Web Frontend")
                    .with_inner_size(PhysicalSize::<u32>::new(1920, 1200))
                    .with_resizable(false)
                    .with_enabled_buttons(WindowButtons::CLOSE))
        }?;

        // Add custom protocol handler for video streaming
        let frontend_webview =
            WebViewBuilder::new()
                .with_url("http://localhost:9688/")
                .with_custom_protocol("stream".to_owned(), move |_, request| {
                    handle_stream_request(&stream_manager_for_protocol, request)
                        .expect("failed to handle stream request")
                })
                .build(&frontend_window)
                .context("failed to create webview for frontend window")?;


        // Start encoding thread
        let encoding_thread =
            EncodingThread::new(
                device.clone(),
                device_context.clone(),
                Self::STREAM_SIZE,
                Arc::clone(&stream_manager))
            .context("failed to create encoding thread")?;

        // let control_window = api_call! {
        //     event_loop.create_window(
        //         Window::default_attributes()
        //             .with_title("Nekomaru LiveUI Control Panel")
        //             .with_inner_size(LogicalSize::<u32>::new(960, 600))
        //             .with_resizable(false)
        //             .with_enabled_buttons(WindowButtons::CLOSE))
        // }?;

        Ok(Self {
            main_capture,
            encoding_thread,
            stream_manager,
            device,
            device_context,
            resample_pass,
            staging_texture,
            staging_texture_rtv,
            frontend_window,
            frontend_webview,
        })
    }

    fn on_window_event(&mut self, window_id: WindowId, event: WindowEvent) {
        match window_id {
            id if id == self.frontend_window.id() => {
                self.main_capture.update();

                // Copy frame and send to encoding thread
                let source_texture = self.main_capture.frame_buffer();
                let source_view =
                    out_var_or_err(|out| api_call!(unsafe {
                        self.device.CreateShaderResourceView(
                            source_texture,
                            None,
                            Some(out))
                    }))
                        .expect("failed to create shader view")
                        .expect("failed to create shader view");
                self.resample_pass.resample(
                    &self.device_context,
                    &source_view,
                    &self.staging_texture_rtv);

                // Get timestamp
                let timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(duration) => duration.as_micros() as u64,
                    Err(e) => {
                        log::warn!("System time error: {e:?}");
                        0  // Fallback to 0, encoder will still work
                    }
                };

                // Send to encoding thread (non-blocking)
                self.encoding_thread.send_frame(CaptureFrame {
                    texture: self.staging_texture.clone(),
                    timestamp_us: timestamp,
                });
            }
            _ => {}
        }
    }
}

/// Create a staging texture for copying captured frames
fn create_staging_texture(device: &ID3D11Device, size: Size2D<u32>)
    -> anyhow::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.width,
        Height: size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,  // Match capture format
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags:
            D3D11_BIND_RENDER_TARGET.0 as u32 |
                D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };

    let mut texture = None;
    api_call!(unsafe {
        device.CreateTexture2D(
            &raw const desc,
            None,
            Some(&raw mut texture))
    }).with_context(|| context!("creating staging texture for encoding"))?;

    texture.ok_or_else(|| anyhow::anyhow!("staging texture is null"))
}

/// Handle custom protocol requests for video streaming
fn handle_stream_request(
    manager: &Arc<StreamManager>,
    request: wry::http::Request<Vec<u8>>)
    -> std::result::Result<wry::http::Response<Cow<'static, [u8]>>, Box<dyn std::error::Error>> {
    use wry::http::Response;

    let path = request.uri().path();

    match path {
        "/init" => {
            // Return SPS/PPS as JSON
            let params = manager.get_codec_params()
                .ok_or("encoder not initialized")?;

            let response = serde_json::json!({
                "sps": base64_encode(&params.sps),
                "pps": base64_encode(&params.pps),
                "width": params.width,
                "height": params.height,
            });

            Response::builder()
                .header("Content-Type", "application/json")
                .header("Access-Control-Allow-Origin", "*")
                .body(Cow::Owned(response.to_string().into_bytes()))
                .map_err(Into::into)
        }

        "/stream" => {
            // Long-polling endpoint for next frame
            let query = request.uri().query().unwrap_or("");
            let after_seq = parse_query_param(query, "after").unwrap_or(0);

            // Wait for next frame (timeout = 100ms)
            let frame = manager.wait_for_frame(after_seq, Duration::from_millis(100))
                .ok_or("timeout waiting for frame")?;

            // Serialize frame as binary
            let mut buffer = Vec::new();
            serialize_stream_frame(&frame, &mut buffer)?;

            Response::builder()
                .header("Content-Type", "application/octet-stream")
                .header("X-Sequence", frame.sequence.to_string())
                .header("X-Timestamp", frame.timestamp_us.to_string())
                .header("X-Keyframe", frame.is_keyframe.to_string())
                .header("Access-Control-Allow-Origin", "*")
                .header("Access-Control-Expose-Headers", "X-Sequence,X-Timestamp,X-Keyframe")
                .body(Cow::Owned(buffer))
                .map_err(Into::into)
        }

        _ => {
            Response::builder()
                .status(404)
                .body(Cow::Borrowed(b"" as &[u8]))
                .map_err(Into::into)
        }
    }
}

/// Serialize stream frame to binary format
fn serialize_stream_frame(
    frame: &crate::stream::StreamFrame,
    buffer: &mut Vec<u8>)
    -> anyhow::Result<()> {
    // Write timestamp (u64 little-endian)
    buffer.extend_from_slice(&frame.timestamp_us.to_le_bytes());

    // Write number of NAL units (u32 little-endian)
    buffer.extend_from_slice(&(frame.nal_units.len() as u32).to_le_bytes());

    // Write each NAL unit
    for unit in &frame.nal_units {
        // Write NAL unit type (u8)
        buffer.push(unit.unit_type as u8);

        // Write data length (u32 little-endian)
        buffer.extend_from_slice(&(unit.data.len() as u32).to_le_bytes());

        // Write data
        buffer.extend_from_slice(&unit.data);
    }

    Ok(())
}

/// Base64 encode data
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Parse query parameter from query string
fn parse_query_param(query: &str, key: &str) -> Option<u64> {
    query.split('&')
        .find_map(|pair| {
            let mut parts = pair.split('=');
            if parts.next()? == key {
                parts.next()?.parse().ok()
            } else {
                None
            }
        })
}