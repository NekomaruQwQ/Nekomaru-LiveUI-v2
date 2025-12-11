mod helper;
use helper::AppWrapper;

use crate::capture::CaptureSession;
use crate::converter::NV12Converter;
use crate::encoder::H264Encoder;
use crate::encoder::H264EncoderConfig;
use crate::resample::Resampler;
use crate::stream::StreamManager;

use nkcore::euclid::*;
use nkcore::*;

use std::sync::Arc;
use std::thread;

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
    main_capture: CaptureSession,
    main_capture_staging_bgra8: ID3D11Texture2D,
    main_capture_staging_bgra8_rtv: ID3D11RenderTargetView,

    // Stream manager for encoded frames
    _stream_manager: Arc<StreamManager>,
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

        let main_webview =
            WebViewBuilder::new()
                .with_url("http://localhost:9688/")
                // .with_custom_protocol("stream".to_owned(), move |_, request| {
                //     handle_stream_request(&stream_manager_for_protocol, request)
                //         .expect("failed to handle stream request")
                // })
                .build(&main_window)
                .context("failed to create webview for frontend window")?;

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

        let main_capture_target = {
            use windows::Win32::UI::WindowsAndMessaging::FindWindowA;
            api_call!(unsafe {
                FindWindowA(
                    PCSTR::default(),
                    PCSTR(c"Nekomaru LiveUI v1".as_ptr().cast()))
            })?
        };

        let main_capture =
            CaptureSession::capture_window(&device, &device_context, main_capture_target)
                .context("failed to start main capture session")?;

        let main_capture_staging_bgra8 =
            helper::create_texture(
                &device,
                Self::STREAM_FRAME_SIZE,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                &[
                    D3D11_BIND_SHADER_RESOURCE,
                    D3D11_BIND_RENDER_TARGET,
                ])
                .context("failed to create staging texture")?;

        let main_capture_staging_bgra8_rtv =
            out_var_or_err(|out| api_call!(unsafe {
                device.CreateRenderTargetView(
                    &main_capture_staging_bgra8,
                    None,
                    Some(out))
            }))?
                .context("failed to create staging texture rtv")?;

        let main_capture_staging_nv12 =
            helper::create_texture(
                &device,
                Self::STREAM_FRAME_SIZE,
                DXGI_FORMAT_NV12,
                &[D3D11_BIND_RENDER_TARGET])
                .context("failed to create staging NV12 texture")?;

        // Create stream manager (60 frame buffer = ~1 second at 60fps)
        let stream_manager = Arc::new(StreamManager::new(60));
        let stream_manager_for_encoder = Arc::clone(&stream_manager);

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

        main_window.request_redraw();

        Ok(Self {
            main_window,
            main_webview,
            device,
            device_context,
            resampler,
            main_capture,
            main_capture_staging_bgra8,
            main_capture_staging_bgra8_rtv,
            _stream_manager: stream_manager,
        })
    }

    fn on_window_event(&mut self, window_id: WindowId, event: WindowEvent) {
        if window_id == self.main_window.id() {
            match event {
                WindowEvent::RedrawRequested => {
                    // Update capture and resample to staging texture ("restock the shelf")
                    self.main_capture.update();

                    let source_texture =
                        self.main_capture.frame_buffer();

                    let viewport = D3D11_VIEWPORT {
                        TopLeftX: 0.0,
                        TopLeftY: 0.0,
                        Width: Self::STREAM_FRAME_SIZE.width as f32,
                        Height: Self::STREAM_FRAME_SIZE.height as f32,
                        MinDepth: 0.0,
                        MaxDepth: 1.0,
                    };
                    unsafe {
                        self.device_context
                            .RSSetViewports(Some(&[viewport]));                        
                    }
                    
                    let source_view =
                        out_var_or_err(|out| api_call!(unsafe {
                            self.device.CreateShaderResourceView(
                                source_texture,
                                None,
                                Some(out))
                        }))
                            .expect("failed to create shader view")
                            .expect("failed to create shader view");
                    self.resampler.resample(
                        &self.device_context,
                        &source_view,
                        &self.main_capture_staging_bgra8_rtv);

                    // CRITICAL: Flush and wait for GPU to complete resampling
                    // Flush() submits commands, sleep gives GPU time to execute them
                    unsafe { self.device_context.Flush(); }
                    thread::sleep(std::time::Duration::from_millis(5));

                    // Request next frame immediately to keep "shelf" continuously updated
                    // This creates a continuous loop: RedrawRequested → update → request_redraw → RedrawRequested...
                    self.main_window.request_redraw();
                }
                WindowEvent::CloseRequested => {},
                _ => {},
            }
        }
    }
}

/// Handle custom protocol requests for video streaming
#[cfg(false)]
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
