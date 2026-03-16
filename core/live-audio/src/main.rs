//! `live-audio.exe` — WASAPI audio capture to stdout.
//!
//! Captures audio from a named input device (default: "Loopback L + R") via
//! WASAPI in shared mode and writes raw PCM chunks as binary IPC messages
//! (see [`live_audio`]) to stdout.  Log output goes to stderr.
//!
//! ## Usage
//!
//! ```text
//! live-audio.exe
//! live-audio.exe --device "Loopback L + R"
//! live-audio.exe --list-devices
//! ```

#![expect(clippy::multiple_unsafe_ops_per_block, reason = "generated code")]

use live_audio::*;

use clap::Parser;
use widestring::U16CStr;

use std::io::BufWriter;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::Win32::Devices::FunctionDiscovery::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::Multimedia::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::*;

/// `WAVE_FORMAT_EXTENSIBLE` (0xFFFE) — indicates a WAVEFORMATEXTENSIBLE
/// struct with a subformat GUID.  Not exported by the `windows` crate.
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

// ── Constants ───────────────────────────────────────────────────────────────

/// Target chunk duration in milliseconds.
/// 10ms at 48kHz = 480 samples per channel.
const CHUNK_DURATION_MS: u32 = 10;

/// How long to sleep between WASAPI buffer polls (ms).
/// WASAPI shared mode typically provides ~10ms buffers, so 5ms polling
/// ensures we don't miss data while keeping CPU usage low.
const POLL_SLEEP_MS: u64 = 5;

// ── CLI ─────────────────────────────────────────────────────────────────────

/// Standalone audio capture to stdout.
#[derive(Parser)]
struct Cli {
    /// Name of the audio capture device (matched against friendly name).
    /// Required unless `--list-devices` is used.
    #[arg(long)]
    device: Option<String>,

    /// List available audio capture devices and exit.
    #[arg(long)]
    list_devices: bool,
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let cli = Cli::parse();

    // SAFETY: Called once at the start of the main thread before any COM usage.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .expect("COM initialization failed");

    if cli.list_devices {
        list_devices();
        return;
    }

    let Some(device) = cli.device else {
        log::error!("--device is required (use --list-devices to see available devices)");
        std::process::exit(1);
    };

    if let Err(e) = run_capture(&device) {
        log::error!("{e}");
        std::process::exit(1);
    }
}

// ── Device Enumeration ──────────────────────────────────────────────────────

/// Print all active audio capture devices to stderr.
fn list_devices() {
    let devices = enumerate_capture_devices();
    if devices.is_empty() {
        log::info!("no audio capture devices found");
        return;
    }
    for name in &devices {
        log::info!("  {name}");
    }
}

/// Return friendly names of all active audio capture devices.
fn enumerate_capture_devices() -> Vec<String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .expect("failed to create device enumerator");

        let collection = enumerator
            .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
                .expect("failed to enumerate audio endpoints");

        let count = collection.GetCount().unwrap_or(0);
        let mut names = Vec::with_capacity(count as usize);

        for i in 0..count {
            let Ok(device) = collection.Item(i) else { continue };
            if let Some(name) = get_device_friendly_name(&device) {
                names.push(name);
            }
        }

        names
    }
}

/// Read the friendly name from a device's property store.
fn get_device_friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let prop = store.GetValue(&PKEY_Device_FriendlyName).ok()?;

        // The friendly name is stored as a VT_LPWSTR PROPVARIANT.
        // Access the wide string pointer through the variant union.
        if prop.Anonymous.Anonymous.vt == VT_LPWSTR {
            let pwsz = prop.Anonymous.Anonymous.Anonymous.pwszVal;
            let wide = U16CStr::from_ptr_str(pwsz.0);
            Some(wide.to_string_lossy())
        } else {
            None
        }
    }
}

/// Find a capture device by friendly name (exact match).
fn find_device_by_name(name: &str) -> Result<IMMDevice, String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("failed to create device enumerator: {e}"))?;

        let collection = enumerator
            .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
            .map_err(|e| format!("failed to enumerate audio endpoints: {e}"))?;

        let count = collection.GetCount().map_err(|e| format!("GetCount failed: {e}"))?;

        for i in 0..count {
            let Ok(device) = collection.Item(i) else { continue };
            if let Some(friendly) = get_device_friendly_name(&device) {
                if friendly == name {
                    return Ok(device);
                }
            }
        }

        Err(format!("audio device not found: \"{name}\""))
    }
}

// ── Capture Loop ────────────────────────────────────────────────────────────

/// Open the named device and stream PCM audio to stdout until the pipe breaks.
fn run_capture(device_name: &str) -> Result<(), String> {
    let device = find_device_by_name(device_name)?;
    log::info!("found device: \"{device_name}\"");

    unsafe { capture_loop(&device) }
}

/// WASAPI shared-mode capture loop.
///
/// Reads audio from the device, re-chunks into fixed-size blocks, and writes
/// `AudioFrame` messages to stdout.  Exits cleanly on broken pipe (server
/// killed the process) or device error.
unsafe fn capture_loop(device: &IMMDevice) -> Result<(), String> {
    let audio_client: IAudioClient = unsafe { device
        .Activate(CLSCTX_ALL, None) }
        .map_err(|e| format!("failed to activate IAudioClient: {e}"))?;

    // Query the device's native mix format.
    let mix_format_ptr = unsafe { audio_client
        .GetMixFormat() }
        .map_err(|e| format!("GetMixFormat failed: {e}"))?;
    let mix_format = unsafe { &*mix_format_ptr };

    let sample_rate = mix_format.nSamplesPerSec;
    let channels = mix_format.nChannels;
    let native_bits = mix_format.wBitsPerSample;

    // Determine if we need f32→s16 conversion.
    // WASAPI shared mode often provides f32le (wFormatTag == WAVE_FORMAT_IEEE_FLOAT
    // or WAVE_FORMAT_EXTENSIBLE with IEEE_FLOAT subformat).
    let is_float = unsafe { is_float_format(mix_format) };

    log::info!("device format: {sample_rate}Hz, {channels}ch, {native_bits}-bit{}",
        if is_float { " (float)" } else { "" });

    if sample_rate != 48000 {
        log::warn!("device sample rate is {sample_rate}Hz, not 48000Hz — \
            audio will play at the device's native rate (no resampling)");
    }

    // Initialize in shared mode with the device's native format.
    // Buffer duration: 20ms (200_000 × 100ns units) — 2× our chunk size for safety.
    let buffer_duration: i64 = 200_000; // 20ms in 100ns units
    unsafe { audio_client
        .Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            0,
            buffer_duration,
            0,
            mix_format_ptr,
            None) }
        .map_err(|e| format!("IAudioClient::Initialize failed: {e}"))?;

    let capture_client: IAudioCaptureClient = unsafe { audio_client
        .GetService() }
        .map_err(|e| format!("failed to get IAudioCaptureClient: {e}"))?;

    // Compute chunk size: number of samples per chunk (per channel).
    let chunk_samples = (sample_rate * CHUNK_DURATION_MS / 1000) as usize;
    // Bytes per sample frame (all channels, s16le output).
    let output_frame_bytes = channels as usize * 2; // s16le = 2 bytes per sample per channel
    let chunk_bytes = chunk_samples * output_frame_bytes;

    // Write AudioParams to stdout.
    let mut writer = BufWriter::new(std::io::stdout().lock());
    let params = AudioParams {
        sample_rate,
        channels: channels as u8,
        bits_per_sample: 16, // always output s16le
    };
    write_audio_params(&mut writer, &params)
        .map_err(|e| format!("failed to write AudioParams: {e}"))?;

    log::info!("streaming: {sample_rate}Hz, {channels}ch, s16le, \
        {chunk_samples} samples/chunk ({CHUNK_DURATION_MS}ms)");

    // Start the audio capture stream.
    unsafe { audio_client.Start() }
        .map_err(|e| format!("IAudioClient::Start failed: {e}"))?;

    // Accumulation buffer for re-chunking WASAPI's variable-size buffers
    // into fixed-size chunks.
    let mut accum: Vec<u8> = Vec::with_capacity(chunk_bytes * 2);

    // Native sample frame size (input from WASAPI).
    let input_frame_bytes = (channels as usize) * (native_bits as usize / 8);

    loop {
        // Read all available frames from WASAPI.
        let frames_read = unsafe { drain_wasapi_buffer(
            &capture_client, &mut accum,
            input_frame_bytes, channels as usize, is_float) }?;

        // Emit complete chunks from the accumulation buffer.
        while accum.len() >= chunk_bytes {
            let timestamp_us = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros() as u64;

            let chunk_data: Vec<u8> = accum.drain(..chunk_bytes).collect();
            let frame = AudioFrame {
                timestamp_us,
                pcm_data: chunk_data,
            };

            if let Err(e) = write_audio_frame(&mut writer, &frame) {
                // Broken pipe means the server killed us — exit cleanly.
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    log::info!("stdout closed, exiting");
                    return Ok(());
                }
                return Err(format!("failed to write AudioFrame: {e}"));
            }
        }

        // If no frames were available, sleep briefly to avoid busy-waiting.
        if frames_read == 0 {
            std::thread::sleep(Duration::from_millis(POLL_SLEEP_MS));
        }
    }
}

/// Drain all available frames from the WASAPI capture buffer into `accum`.
///
/// Handles f32→s16le conversion if `is_float` is true.  Returns the total
/// number of sample frames read (0 if the buffer was empty).
unsafe fn drain_wasapi_buffer(
    capture_client: &IAudioCaptureClient,
    accum: &mut Vec<u8>,
    input_frame_bytes: usize,
    channels: usize,
    is_float: bool,
) -> Result<u32, String> {
    let mut total_frames = 0u32;

    loop {
        let mut buffer_ptr = std::ptr::null_mut();
        let mut frames_available = 0u32;
        let mut flags = 0u32;

        let hr = unsafe { capture_client.GetBuffer(
            &mut buffer_ptr,
            &mut frames_available,
            &mut flags,
            None,
            None) };

        if hr.is_err() || frames_available == 0 {
            if frames_available > 0 {
                let _ = unsafe { capture_client.ReleaseBuffer(frames_available) };
            }
            break;
        }

        let data = unsafe { std::slice::from_raw_parts(
            buffer_ptr, frames_available as usize * input_frame_bytes) };

        // AUDCLNT_BUFFERFLAGS_SILENT means the device produced silence —
        // write zero samples instead of reading the (possibly garbage) buffer.
        let is_silent = (flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)) != 0;

        if is_silent {
            let silent_bytes = frames_available as usize * channels * 2;
            accum.extend(std::iter::repeat_n(0u8, silent_bytes));
        } else if is_float {
            convert_f32_to_s16(data, accum, frames_available as usize * channels);
        } else if input_frame_bytes == channels * 2 {
            // Already s16le — copy directly.
            accum.extend_from_slice(data);
        } else {
            // Wider integer format — take low 16 bits per sample (LE).
            let bytes_per_sample = input_frame_bytes / channels;
            for frame_idx in 0..frames_available as usize {
                for ch in 0..channels {
                    let offset = frame_idx * input_frame_bytes + ch * bytes_per_sample;
                    accum.push(data[offset]);
                    accum.push(data[offset + 1]);
                }
            }
        }

        unsafe { capture_client.ReleaseBuffer(frames_available) }
            .map_err(|e| format!("ReleaseBuffer failed: {e}"))?;

        total_frames += frames_available;
    }

    Ok(total_frames)
}

/// Convert interleaved f32le samples to interleaved s16le samples.
///
/// Each f32 value is clamped to [-1.0, 1.0] and scaled to [-32768, 32767].
fn convert_f32_to_s16(input: &[u8], output: &mut Vec<u8>, num_samples: usize) {
    for i in 0..num_samples {
        let offset = i * 4;
        let sample_f32 = f32::from_le_bytes([
            input[offset],
            input[offset + 1],
            input[offset + 2],
            input[offset + 3],
        ]);

        let clamped = sample_f32.clamp(-1.0, 1.0);
        let sample_i16 = (clamped * 32767.0) as i16;

        output.extend_from_slice(&sample_i16.to_le_bytes());
    }
}

// ── Format Detection ────────────────────────────────────────────────────────

/// Check whether a WAVEFORMATEX describes a floating-point format.
///
/// Handles both plain `WAVE_FORMAT_IEEE_FLOAT` and `WAVE_FORMAT_EXTENSIBLE`
/// with an IEEE float subformat GUID.
unsafe fn is_float_format(fmt: &WAVEFORMATEX) -> bool {
    if fmt.wFormatTag == WAVE_FORMAT_IEEE_FLOAT as u16 {
        return true;
    }

    if fmt.wFormatTag == WAVE_FORMAT_EXTENSIBLE {
        // WAVEFORMATEXTENSIBLE is packed — copy SubFormat to avoid
        // a misaligned reference to the GUID field.
        let sub_format = unsafe {
            let ext = &*(fmt as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE);
            std::ptr::read_unaligned(std::ptr::addr_of!(ext.SubFormat))
        };
        return sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
    }

    false
}
