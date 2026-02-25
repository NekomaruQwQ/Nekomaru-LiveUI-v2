//! `live-capture.exe` — standalone screen capture + H.264 encoding to stdout.
//!
//! Captures a window by HWND, resamples to the target resolution, encodes with
//! NVENC, and writes binary IPC messages (see [`live_capture`]) to stdout.
//! All log output goes to stderr so stdout stays exclusively binary.
//!
//! ## Usage
//!
//! ```text
//! live-capture.exe --hwnd 0x1A2B3C --width 1920 --height 1200
//! ```

mod d3d11;
mod capture;
mod converter;
mod encoder;
mod resample;

use capture::CaptureSession;
use converter::NV12Converter;
use encoder::{H264Encoder, H264EncoderConfig};
use resample::Resampler;

use live_capture::*;

use nkcore::prelude::*;
use nkcore::prelude::euclid::Size2D;

use std::io::{BufWriter, Write as _};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::System::Com::*;

// ── Constants ───────────────────────────────────────────────────────────────

const FRAME_RATE: u32 = 60;
const BITRATE: u32 = 8_000_000; // 8 Mbps CBR

// ── CLI ─────────────────────────────────────────────────────────────────────

struct CliArgs {
    hwnd: isize,
    width: u32,
    height: u32,
}

impl CliArgs {
    fn parse() -> anyhow::Result<Self> {
        let mut args = std::env::args().skip(1);
        let mut hwnd: Option<isize> = None;
        let mut width: Option<u32> = None;
        let mut height: Option<u32> = None;

        while let Some(key) = args.next() {
            let value = args.next()
                .ok_or_else(|| anyhow::anyhow!("missing value for {key}"))?;

            match key.as_str() {
                "--hwnd" => {
                    hwnd = Some(if let Some(hex) = value.strip_prefix("0x") {
                        isize::from_str_radix(hex, 16)?
                    } else {
                        value.parse()?
                    });
                },
                "--width" => width = Some(value.parse()?),
                "--height" => height = Some(value.parse()?),
                other => anyhow::bail!("unknown argument: {other}"),
            }
        }

        let hwnd = hwnd.ok_or_else(|| anyhow::anyhow!("missing --hwnd"))?;
        let width = width.ok_or_else(|| anyhow::anyhow!("missing --width"))?;
        let height = height.ok_or_else(|| anyhow::anyhow!("missing --height"))?;

        anyhow::ensure!(
            width.is_multiple_of(16) && height.is_multiple_of(16),
            "width and height must be multiples of 16 (got {width}x{height})");
        anyhow::ensure!(hwnd != 0, "--hwnd must be non-zero");

        Ok(Self { hwnd, width, height })
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    pretty_env_logger::init();

    let args = match CliArgs::parse() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("usage: live-capture --hwnd 0x1A2B3C --width 1920 --height 1200");
            std::process::exit(1);
        }
    };

    if let Err(e) = run(args) {
        eprintln!("fatal: {e:?}");
        std::process::exit(1);
    }
}

fn run(args: CliArgs) -> anyhow::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .context("CoInitializeEx failed")?;

    let frame_size = Size2D::new(args.width, args.height);
    let hwnd = HWND(args.hwnd as _);

    log::info!("target: HWND={:#X}, {}x{}", args.hwnd, args.width, args.height);

    // Create D3D11 device
    let (_, device, device_context) =
        d3d11::create_device()
            .context("failed to create D3D11 device")?;

    // Create resampler (GPU shader for aspect-ratio-preserving scaling)
    let resampler =
        Resampler::new(&device)
            .context("failed to create resampler")?;

    // Create staging BGRA8 texture (shared between capture thread and encoding thread)
    let staging_bgra8 =
        d3d11::create_texture_2d(
            &device,
            frame_size,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            &[D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_RENDER_TARGET])
            .context("failed to create BGRA8 staging texture")?;
    let staging_bgra8_rtv =
        d3d11::create_rtv_for_texture_2d(&device, &staging_bgra8)
            .context("failed to create BGRA8 staging RTV")?;

    // Clear to dark gray so the first few frames aren't random garbage
    unsafe {
        device_context.ClearRenderTargetView(
            &staging_bgra8_rtv,
            &[0.16, 0.16, 0.16, 1.0]);
    }

    // Spawn encoding thread
    let encoding_handle = thread::Builder::new()
        .name("encoding".to_owned())
        .spawn({
            let device = device.clone();
            let device_context = device_context.clone();
            let frame_source = staging_bgra8.clone();
            move || encoding_thread(device, device_context, frame_source, frame_size)
        })
        .context("failed to spawn encoding thread")?;

    // Create capture session
    let mut capture =
        CaptureSession::from_hwnd(&device, hwnd)
            .context("failed to start capture session")?;
    log::info!("capture session started");

    // ── Capture loop ────────────────────────────────────────────────────
    // Runs on the main thread.  Continuously grabs frames from the capture
    // session and resamples them into the shared staging texture.  The
    // encoding thread reads from this texture at its own pace ("bakery model").
    loop {
        // Check if encoding thread panicked
        if encoding_handle.is_finished() {
            anyhow::bail!("encoding thread exited unexpectedly");
        }

        match capture.get_next_frame(&device_context) {
            Ok(Some(frame)) => {
                let viewport =
                    capture::calculate_resample_viewport(frame.size, frame_size);
                unsafe { device_context.RSSetViewports(Some(&[viewport])); }

                let source_srv =
                    d3d11::create_srv_for_texture_2d(&device, &frame.raw_texture)
                        .context("failed to create SRV for captured frame")?;
                resampler.resample(&device_context, &source_srv, &staging_bgra8_rtv);

                unsafe { device_context.RSSetViewports(Some(&[])); }

                // Flush GPU commands so the encoding thread sees the resampled frame.
                // The small sleep gives the GPU time to finish before the encoder reads.
                unsafe { device_context.Flush(); }
                thread::sleep(Duration::from_millis(5));
            },
            Ok(None) => {
                // No new frame ready — brief sleep to avoid busy-waiting
                thread::sleep(Duration::from_millis(1));
            },
            Err(e) => {
                log::error!("capture error: {e:?}");
                // Non-fatal: the encoder will re-encode the last good frame.
                // If the window was closed, subsequent calls will keep failing
                // and the server should kill us.
                thread::sleep(Duration::from_millis(100));
            },
        }
    }
}

// ── Encoding thread ─────────────────────────────────────────────────────────

fn encoding_thread(
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,
    frame_source: ID3D11Texture2D,
    frame_size: Size2D<u32>) {
    log::info!("encoding thread started");

    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .expect("CoInitializeEx failed on encoding thread");

    let nv12_converter =
        NV12Converter::new(&device, &device_context, frame_size.width, frame_size.height)
            .expect("failed to create NV12 converter");
    let nv12_staging =
        d3d11::create_texture_2d(
            &device,
            frame_size,
            DXGI_FORMAT_NV12,
            &[D3D11_BIND_RENDER_TARGET])
            .expect("failed to create NV12 staging texture");
    log::info!("NV12 converter and staging texture created");

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    let mut last_sps: Option<Vec<u8>> = None;
    let mut last_pps: Option<Vec<u8>> = None;

    let encoder = H264Encoder::new(&device, H264EncoderConfig {
        frame_size,
        frame_rate: FRAME_RATE,
        bitrate: BITRATE,
    }).expect("failed to create H.264 encoder");

    encoder.run(
        // Frame source: convert BGRA8 → NV12
        || {
            nv12_converter
                .convert(&frame_source, &nv12_staging)
                .expect("BGRA8 → NV12 conversion failed");
            nv12_staging.clone()
        },
        // Frame target: serialize to stdout via IPC protocol
        |nal_units: Vec<NALUnit>| {
            if nal_units.is_empty() {
                return;
            }

            // Extract SPS/PPS from IDR frames and send CodecParams if changed
            let sps = nal_units.iter().find(|u| u.unit_type == NALUnitType::SPS);
            let pps = nal_units.iter().find(|u| u.unit_type == NALUnitType::PPS);

            if let (Some(sps), Some(pps)) = (sps, pps) {
                let sps_changed = last_sps.as_ref() != Some(&sps.data);
                let pps_changed = last_pps.as_ref() != Some(&pps.data);

                if sps_changed || pps_changed {
                    let params = CodecParams {
                        sps: sps.data.clone(),
                        pps: pps.data.clone(),
                        width: frame_size.width,
                        height: frame_size.height,
                    };

                    if let Err(e) = write_codec_params(&mut writer, &params) {
                        log::error!("failed to write CodecParams: {e}");
                        return;
                    }

                    last_sps = Some(sps.data.clone());
                    last_pps = Some(pps.data.clone());
                    log::info!(
                        "sent CodecParams: {}x{}, SPS={}B, PPS={}B",
                        frame_size.width, frame_size.height,
                        params.sps.len(), params.pps.len());
                }
            }

            // Build and write Frame message
            let is_keyframe = nal_units.iter().any(|u| u.unit_type == NALUnitType::IDR);
            let frame = FrameMessage {
                // The encoder sets sample timestamps in 100ns units; we approximate
                // with wall-clock time here.  The server doesn't rely on exact values.
                timestamp_us: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as u64,
                is_keyframe,
                nal_units,
            };

            if let Err(e) = write_frame(&mut writer, &frame) {
                log::error!("failed to write Frame: {e}");
                // Stdout broken (server killed us or pipe closed) — exit cleanly
                let _ = writer.flush();
                std::process::exit(0);
            }
        });
}
