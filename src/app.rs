mod helper;
use helper::AppWrapper;

mod capture_selector;
use capture_selector::LiveCaptureWindowSelector;

use crate::capture::CaptureSession;
use crate::converter::NV12Converter;
use crate::encoder::H264Encoder;
use crate::encoder::H264EncoderConfig;
use crate::resample::Resampler;
use crate::stream::StreamManager;

use nkcore::euclid::*;
use nkcore::*;

use std::borrow::Cow;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use wry::WebView;
use wry::WebViewBuilder;

use winit::{
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    event_loop::EventLoop,
    window::Window,
    window::WindowButtons,
    window::WindowId,
};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::System::Com::*;

pub fn run() {
    EventLoop::<()>::new()
        .expect("failed to create event loop")
        .pipe(|event_loop| event_loop.run_app(&mut AppWrapper::<LiveApp>(None)))
        .expect("failed to run event loop");
}

#[expect(dead_code, reason = "to keep various resources alive")]
struct LiveApp {
    // LiveUI output
    main_window: Window,
    main_webview: WebView,

    // D3D11 device and resources
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,
    resampler: Resampler,

    // LiveUI main capture session and staging textures
    main_capture_selector: LiveCaptureWindowSelector,
    main_capture_hwnd: HWND,
    main_capture: Option<CaptureSession>,
    main_capture_staging_bgra8: ID3D11Texture2D,
    main_capture_staging_bgra8_rtv: ID3D11RenderTargetView,

    // Stream manager for encoded frames
    stream_manager: Arc<StreamManager>,
}

impl LiveApp {
    const STREAM_FRAME_SIZE: Size2D<u32> = Size2D::new(1920, 1200);

    fn new(event_loop: &ActiveEventLoop) -> anyhow::Result<Self> {
        let main_window = api_call! {
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Nekomaru LiveUI Web Frontend")
                    .with_inner_size(PhysicalSize::<u32>::new(1920, 1200))
                    .with_resizable(false)
                    .with_enabled_buttons(WindowButtons::CLOSE))
        }?;

        // let control_window = api_call! {
        //     event_loop.create_window(
        //         Window::default_attributes()
        //             .with_title("Nekomaru LiveUI Control Panel")
        //             .with_inner_size(LogicalSize::<u32>::new(960, 600))
        //             .with_resizable(false)
        //             .with_enabled_buttons(WindowButtons::CLOSE))
        // }?;

        let (_, device, device_context) =
            helper::create_device()
                .context("failed to create graphics context")?;
        let resampler =
            Resampler::new(&device)
                .context("failed to create resample pass")?;
        let nv12_converter =
            NV12Converter::new(&device, &device_context)
                .context("failed to create bgra-to-nv12 converter")?;

        let main_capture_staging_bgra8 =
            helper::create_texture_2d(
                &device,
                Self::STREAM_FRAME_SIZE,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                &[
                    D3D11_BIND_SHADER_RESOURCE,
                    D3D11_BIND_RENDER_TARGET,
                ])
                .context("failed to create staging texture")?;

        let main_capture_staging_bgra8_rtv =
            helper::create_rtv_for_texture_2d(&device, &main_capture_staging_bgra8)
                .context("failed to create staging texture rtv")?;

        let main_capture_staging_nv12 =
            helper::create_texture_2d(
                &device,
                Self::STREAM_FRAME_SIZE,
                DXGI_FORMAT_NV12,
                &[D3D11_BIND_RENDER_TARGET])
                .context("failed to create staging NV12 texture")?;

        // Create stream manager (60 frame buffer = ~1 second at 60fps)
        let stream_manager = Arc::new(StreamManager::new(60));
        let stream_manager_for_encoder = Arc::clone(&stream_manager);
        let stream_manager_for_protocol = Arc::clone(&stream_manager);

        let mut main_webview_header_map = wry::http::HeaderMap::new();
        main_webview_header_map.insert(
            "Access-Control-Allow-Origin",
            wry::http::HeaderValue::from_str("*")?);
        main_webview_header_map.insert(
            "Access-Control-Allow-Methods",
            wry::http::HeaderValue::from_str("GET, POST, OPTIONS")?);
        main_webview_header_map.insert(
            "Access-Control-Allow-Headers",
            wry::http::HeaderValue::from_str("*")?);

        // Create webview with custom protocol handler
        let main_webview =
            WebViewBuilder::new()
                .with_headers(main_webview_header_map)
                .with_custom_protocol("stream".to_owned(), move |_, request| {
                    handle_stream_request(&stream_manager_for_protocol, request)
                        .inspect_err(|err| log::error!("{err:?}"))
                        .unwrap_or_default()
                })
                .build(&main_window)
                .context("failed to create webview for frontend window")?;

        thread::Builder::new()
            .name("Encoding Thread".to_owned())
            .spawn({
                let device = device.clone();
                let device_context =
                    device_context.clone();
                let mut nv12_converter = nv12_converter;
                let staging_bgra8 =
                    main_capture_staging_bgra8.clone();
                let staging_nv12 =
                    main_capture_staging_nv12;
                move || {
                    log::info!("encoding thread started");

                    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                        .ok()
                        .expect("CoInitializeEx failed");

                    H264Encoder::new(&device, H264EncoderConfig {
                        frame_size: (1920, 1200).into(),
                        frame_rate: 60,
                        bitrate: 8_000_000,  // 8 Mbps
                        frame_source_callback: Box::new({
                            move || {
                                nv12_converter.convert(&staging_bgra8, &staging_nv12)
                                    .expect("failed to convert BGRA8 to NV12");
                                staging_nv12.clone()
                            }
                        }),
                        frame_target_callback: Box::new(move |nal_units| {
                            // Push encoded frame to stream manager
                            if let Err(e) = stream_manager_for_encoder.push_frame(nal_units) {
                                log::warn!("Failed to push frame to stream: {:?}", e);
                            }
                        }),
                    })
                        .expect("failed to create H.264 encoder")
                        .run();
                }
            })
            .context("failed to spawn encoding thread")?;

        main_webview.open_devtools();
        main_webview.load_url("about:blank")
            .context("failed to load frontend")?;
        main_webview.load_url("http://localhost:9688/")
            .context("failed to load frontend")?;
        main_window.request_redraw();

        Ok(Self {
            main_window,
            main_webview,
            device,
            device_context,
            resampler,
            main_capture_selector: default(),
            main_capture_hwnd: default(),
            main_capture: None,
            main_capture_staging_bgra8,
            main_capture_staging_bgra8_rtv,
            stream_manager,
        })
    }

    fn on_window_event(&mut self, window_id: WindowId, event: WindowEvent) {
        if window_id == self.main_window.id() {
            match event {
                WindowEvent::RedrawRequested => {
                    if self.main_capture_selector.update(&mut self.main_capture_hwnd) {
                        // Note that we only try to start capture once per foreground window change.
                        // If the first attempt fails, subsequent attempts will also fail for the
                        // same window.
                        self.main_capture =
                            CaptureSession::from_window(&self.device, self.main_capture_hwnd)
                                .inspect_err(|err| log::error!("failed to start capture: {err}"))
                                .ok();
                    }

                    self.resample_captured_frame()
                        .inspect_err(|err| log::error!("failed to resample captured frame: {err}"))
                        .ok();
                },
                _ => {},
            }
        }
    }

    fn resample_captured_frame(&mut self) -> anyhow::Result<()> {
        // Ensure continuous WindowEvent::RedrawRequested events.
        defer(|| self.main_window.request_redraw());

        let Some(capture_session) = &mut self.main_capture else {
            // No capture session running, skip the resampling.
            return Ok(());
        };

        let capture_result =
            capture_session
                .get_next_frame()
                .context("failed to get next frame from capture session")?;
        let Some((source_texture, source_size)) = capture_result else {
            // No new frame arrived, but it's ok. Just skip the resampling.
            return Ok(());
        };

        let target_size = Self::STREAM_FRAME_SIZE;
        let viewport =
            Self::calculate_resample_viewport(source_size, target_size);
        unsafe {
            self.device_context
                .RSSetViewports(Some(&[viewport]));
        }

        let source_view =
            helper::create_srv_for_texture_2d(&self.device, &source_texture)
                .context("failed to create rtv for source texture")?;
        self.resampler.resample(
            &self.device_context,
            &source_view,
            &self.main_capture_staging_bgra8_rtv);

        Ok(())
    }

    fn calculate_resample_viewport(
        source_size: Size2D<u32>,
        target_size: Size2D<u32>) -> D3D11_VIEWPORT {
        let scale =
            f32::min(
                target_size.width as f32 / source_size.width as f32,
                target_size.height as f32 / source_size.height as f32);
        let source_size_scaled =
            (source_size.to_f32() * scale).floor().to_u32();
        let target_offset =
            (target_size - source_size_scaled).to_vector() / 2;

        D3D11_VIEWPORT {
            TopLeftX: target_offset.x as _,
            TopLeftY: target_offset.y as _,
            Width: source_size_scaled.width as _,
            Height: source_size_scaled.height as _,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }
    }
}

/// Handle custom protocol requests for video streaming
/// Edge WebView maps http://stream.localhost/path to stream://localhost/path
fn handle_stream_request(
    manager: &Arc<StreamManager>,
    request: wry::http::Request<Vec<u8>>)
    -> std::result::Result<wry::http::Response<Cow<'static, [u8]>>, Box<dyn std::error::Error>> {
    use wry::http::Response;

    log::debug!("{:?}", request);

    // Strip "/localhost" prefix for Edge WebView routing
    let path = request.uri().path().strip_prefix("/localhost").unwrap_or(request.uri().path());

    // Handle OPTIONS preflight requests for CORS
    if request.method() == "OPTIONS" {
        return Response::builder()
            .status(200)
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Methods", "GET, OPTIONS")
            .header("Access-Control-Allow-Headers", "*")
            .header("Access-Control-Max-Age", "86400")
            .body(Cow::Borrowed(b"" as &[u8]))
            .map_err(Into::into);
    }

    match path {
        "/init" => {
            // Wait for encoder to initialize (produce first SPS/PPS)
            // Retry for up to 5 seconds with 100ms intervals
            let params = {
                let mut params_opt = None;
                for _ in 0..50 {
                    if let Some(p) = manager.get_codec_params() {
                        params_opt = Some(p);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                params_opt.ok_or("encoder not initialized after 5 seconds")?
            };

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

            // Wait for next frame (timeout = 5s)
            let frame = manager.wait_for_frame(after_seq, Duration::from_millis(5000))
                .ok_or("timeout waiting for frame")?;

            // Serialize frame as binary
            let mut buffer = Vec::new();
            serialize_stream_frame(&frame, &mut buffer)?;

            log::debug!("serving frame seq={}, keyframe={}", frame.sequence, frame.is_keyframe);

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
                .header("Access-Control-Allow-Origin", "*")
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
