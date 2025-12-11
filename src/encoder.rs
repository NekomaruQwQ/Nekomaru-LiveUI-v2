#![expect(
    non_upper_case_globals,
    reason = "false positive on windows-rs constants")]

mod debug;
mod helper;

use nkcore::euclid::*;
use nkcore::*;

use std::time::*;

use windows::core::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Variant::VARIANT;

const ENCODER_FRAME_RATE: u32 = 60;
const ENCODER_BITRATE: u32 = 8_000_000; // 8 Mbps

/// NAL unit types for H.264
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NALUnitType {
    /// Non-IDR slice
    NonIDR = 1,
    /// Instantaneous Decoder Refresh (keyframe)
    IDR = 5,
    /// Sequence Parameter Set
    SPS = 7,
    /// Picture Parameter Set
    PPS = 8,
}

impl NALUnitType {
    /// Parse NAL unit type from NAL unit header byte
    const fn from_header(header: u8) -> Option<Self> {
        match header & 0x1F {
            1 => Some(Self::NonIDR),
            5 => Some(Self::IDR),
            7 => Some(Self::SPS),
            8 => Some(Self::PPS),
            _ => None,
        }
    }
}

/// Encoded H.264 NAL unit
#[derive(Debug, Clone)]
pub struct NALUnit {
    /// Type of this NAL unit
    pub unit_type: NALUnitType,
    /// Raw NAL unit data (including start code)
    pub data: Vec<u8>,
}

pub struct H264Encoder {
    config: H264EncoderConfig,
    mf_dxgi_manager: IMFDXGIDeviceManager,
    mf_transform: IMFTransform,
    mf_event_generator: IMFMediaEventGenerator,
    frame_count: u64,
    time_elapsed: Duration,
    time_of_last_frame: SystemTime,
}

pub struct H264EncoderConfig {
    pub frame_size: Size2D<u32>,
    pub frame_rate: u32,
    pub bitrate: u32,
    pub frame_source_callback: Box<dyn FnMut() -> ID3D11Texture2D + Send + Sync + 'static>,
    pub frame_target_callback: Box<dyn FnMut(Vec<NALUnit>) + Send + Sync + 'static>,
}

impl H264Encoder {
    #[expect(clippy::panic_in_result_fn, reason = "asserts for valid input")]
    pub fn new(device: &ID3D11Device, config: H264EncoderConfig) -> anyhow::Result<Self> {
        assert!(
            config.frame_size.width.is_multiple_of(16) &&
            config.frame_size.height.is_multiple_of(16),
            "frame size must be multiple of 16");
        log::info!(
            "creating H.264 encoder with config: {}x{} @ {}fps, {} bps",
            config.frame_size.width,
            config.frame_size.height,
            config.frame_rate,
            config.bitrate);

        // Initialize Media Foundation
        api_call!(unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) })?;

        // Find H.264 encoder transform
        let mf_transform =
            helper::find_h264_encoder(&api_call!(device.cast())?)
                .context("failed to find H264Encoder")?;
        let mf_attributes =
            api_call!(unsafe { mf_transform.GetAttributes() })?;

        // Get event generator interface for async MFT event handling
        let mf_event_generator =
            api_call!(mf_transform.cast::<IMFMediaEventGenerator>())
                .context("failed to get IMFMediaEventGenerator interface")?;

        // Unlock async MFT
        api_call!(unsafe { mf_attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1) })
            .context("failed to unlock async MFT")?;
        log::info!("MFT async unlocked");

        // Print supported media types for debugging (after unlock)
        debug::print_mft_supported_output_types(&mf_transform);

        // Configure output type (H.264) BEFORE setting D3D manager
        // Get enumerated type and set required parameters
        let output_type = unsafe { mf_transform.GetOutputAvailableType(0, 0)? };

        // Set required parameters
        api_call!(unsafe { output_type.SetUINT32(&MF_MT_AVG_BITRATE, config.bitrate) })?;

        let frame_size_val =
            ((config.frame_size.width as u64) << 32) |
            (config.frame_size.height as u64);
        api_call!(unsafe { output_type.SetUINT64(&MF_MT_FRAME_SIZE, frame_size_val) })?;

        let frame_rate_ratio = ((config.frame_rate as u64) << 32) | 1u64;
        api_call!(unsafe { output_type.SetUINT64(&MF_MT_FRAME_RATE, frame_rate_ratio) })?;

        // Set Baseline profile for maximum compatibility with WebCodecs
        // Baseline profile inherently disables B-frames
        api_call!(unsafe {
            output_type.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)
        })?;
        log::info!("H.264 profile set to Baseline (no B-frames)");

        log::info!("Setting output type (H.264)...");
        api_call!(unsafe { mf_transform.SetOutputType(0, &output_type, 0) })
            .context("failed to set output type")?;
        log::info!("Output type set successfully!");

        // Print supported input types after output type is set
        debug::print_mft_supported_input_types(&mf_transform);

        // Configure input type (NV12) - use enumerated type #1 (NV12)
        let input_type = unsafe { mf_transform.GetInputAvailableType(0, 1)? };

        log::info!("Setting input type (NV12)...");
        api_call!(unsafe { mf_transform.SetInputType(0, &input_type, 0) })
            .context("failed to set input type")?;
        log::info!("Input type set successfully!");

        // Create DXGI Device Manager AFTER types are configured
        let mut reset_token = 0u32;
        let mf_dxgi_manager =
            out_var_or_err(|out| api_call!(unsafe {
                MFCreateDXGIDeviceManager(&raw mut reset_token, out)
            }))?
                .ok_or_else(|| anyhow::anyhow!("DXGI device manager is null"))?;
        let mf_dxgi_manager_as_unknown =
            api_call!(mf_dxgi_manager.cast::<IUnknown>())?;

        // Register D3D11 device with DXGI manager
        api_call!(unsafe { mf_dxgi_manager.ResetDevice(device, reset_token) })
            .context("failed to register D3D11 device with DXGI manager")?;

        // NOW set D3D manager after both types are configured
        api_call!(unsafe {
            mf_transform.ProcessMessage(
                MFT_MESSAGE_SET_D3D_MANAGER,
                mf_dxgi_manager_as_unknown.as_raw().addr())
        }).context("failed to set D3D manager")?;
        log::info!("DXGI device manager attached to encoder");

        // Configure low-latency settings via ICodecAPI (after types and D3D manager are set)
        log::info!("Configuring low-latency codec settings...");
        let codec_api = api_call!(mf_transform.cast::<ICodecAPI>())?;

        for (name, api, value) in [
            // No B-frames for low latency (B-frames add 2+ frame latency)
            ("B-frame count", &CODECAPI_AVEncMPVDefaultBPictureCount, VARIANT::from(0u32)),
            // GOP size = 2 seconds (120 frames at 60fps for fast recovery)
            ("GOP size", &CODECAPI_AVEncMPVGOPSize, VARIANT::from(config.frame_rate * 2)),
            // Low latency mode
            ("Low latency mode", &CODECAPI_AVLowLatencyMode, VARIANT::from(true)),
            // CBR rate control (constant bitrate for predictable latency)
            ("Rate control mode", &CODECAPI_AVEncCommonRateControlMode,
                VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32)),
        ] {
            match api_call!(unsafe { codec_api.SetValue(api, &value) }) {
                Ok(_) => log::info!("  {} set successfully", name),
                Err(e) => log::warn!("  Failed to set {} (encoder may not support this setting): {:?}", name, e),
            }
        }

        // Start streaming
        api_call!(unsafe {
            mf_transform.ProcessMessage(
                MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
                0)
        })?;

        api_call!(unsafe {
            mf_transform.ProcessMessage(
                MFT_MESSAGE_NOTIFY_START_OF_STREAM,
                0)
        })?;

        log::info!("H.264 encoder ready");

        Ok(Self {
            config,
            mf_dxgi_manager,
            mf_transform,
            mf_event_generator,
            frame_count: 0,
            time_elapsed: Duration::ZERO,
            time_of_last_frame: SystemTime::now(),
        })
    }

    pub fn run(mut self) {
        const POLL_TIMEOUT: u32 = 5;
        loop {
            // **NOTE**: As we are on a separate thread, we perform a blocking wait for new events.
            let event = match unsafe { self.mf_event_generator.GetEvent(default()) } {
                Ok(event) => event,
                Err(err) if err.code() == MF_E_NO_EVENTS_AVAILABLE =>
                    continue,
                Err(err) => {
                    log::warn!("failed to poll encoder event: {err:?}");
                    continue;
                }
            };

            let event_type = match api_call!(unsafe { event.GetType() }) {
                Ok(event_type) => MF_EVENT_TYPE(event_type as _),
                Err(err) => {
                    log::warn!("failed to poll encoder event: {err:?}");
                    continue;
                }
            };

            match event_type {
                METransformDrainComplete => {
                    log::trace!("METransformDrainComplete received");
                },
                METransformNeedInput => {
                    log::trace!("METransformNeedInput received");
                    self.process_input()
                        .context(context!("{} failed", pretty_name::of_method!(Self::process_input)))
                        .unwrap();
                },
                METransformHaveOutput => {
                    log::trace!("METransformHaveOutput received");
                    self.process_output()
                        .context(context!("{} failed", pretty_name::of_method!(Self::process_output)))
                        .unwrap();
                }
                _ => {
                    log::trace!("Unhandled MFT event type: {:?}", event_type);
                }
            }
        }

        // We don't care about any errors during shutdown.

        let _ = api_call!(unsafe {
            self.mf_transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
        });
    }

    fn process_input(&mut self) -> anyhow::Result<()> {
        while SystemTime::now()
            .duration_since(self.time_of_last_frame)
            .context("unexpected time drift")?
            .as_secs_f64() < (1.0 / self.config.frame_rate as f64) {
            // Sleep a bit to avoid busy-waiting
            std::thread::sleep(Duration::from_millis(1));
        }

        let now = SystemTime::now();
        let elapsed =
            now
                .duration_since(self.time_of_last_frame)
                .context("unexpected time drift")?;
        self.frame_count += 1;
        self.time_elapsed += elapsed;
        self.time_of_last_frame = now;
        log::debug!("encoding frame #{} at {}s {}ms",
            self.frame_count,
            self.time_elapsed.as_secs(),
            self.time_elapsed.subsec_millis());

        log::trace!("feeding frame to encoder...");
        // Create DXGI surface buffer from texture
        let frame_texture = (self.config.frame_source_callback)();
        let buffer = api_call!(unsafe {
            MFCreateDXGISurfaceBuffer(
                &ID3D11Texture2D::IID,
                &frame_texture,
                0,
                false)
        })?;

        let sample = api_call!(unsafe { MFCreateSample() })?;
        api_call!(unsafe { sample.AddBuffer(&buffer) })?;

        // Set sample time (convert μs to 100ns units)
        let timestamp_us = self.time_elapsed.as_micros() as u64;
        api_call!(unsafe { sample.SetSampleTime((timestamp_us * 10) as i64) })?;

        // Set sample duration (frame duration at target fps)
        let duration_100ns = elapsed.as_micros() as i64 * 10;
        api_call!(unsafe { sample.SetSampleDuration(duration_100ns) })?;

        // Feed sample to encoder
        match unsafe { self.mf_transform.ProcessInput(0, &sample, 0) } {
            Ok(_) => {
                log::trace!("feeding succeeded");
            },
            Err(e) if e.code() == MF_E_NOTACCEPTING => {
                Err(e).context("encoder not accepting input - should not happen after METransformNeedInput")?;
            },
            Err(e) => {
                Err(e).context("failed to process input sample")?;
            }
        }

        Ok(())
    }

    fn process_output(&mut self) -> anyhow::Result<()> {
        let mut output_buffers = [MFT_OUTPUT_DATA_BUFFER::default()];
        let mut status = 0u32;

        match unsafe {
            self.mf_transform.ProcessOutput(
                0,
                &mut output_buffers,
                &raw mut status)
        } {
            Ok(()) => {
                if let Some(sample) = output_buffers[0].pSample.take() {
                    let buffer = api_call!(unsafe { sample.ConvertToContiguousBuffer() })?;
                    let nal_units = Self::parse_nal_units_from_buffer(&buffer)?;

                    // Debug: Log NAL unit info to verify encoding is working
                    if !nal_units.is_empty() {
                        log::debug!("encoded {} NAL unit(s):", nal_units.len());
                        for (i, unit) in nal_units.iter().enumerate() {
                            log::debug!("  NAL #{}: type={:?}, size={} bytes",
                                i, unit.unit_type, unit.data.len());
                        }
                    }

                    (self.config.frame_target_callback)(nal_units);
                }
            }
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                anyhow::bail!("unexpected MF_E_TRANSFORM_NEED_MORE_INPUT during ProcessOutput - should not happen after METransformHaveOutput");
            },
            Err(e) => {
                Err(e)?;
            },
            _ => {},
        }

        Ok(())
    }

    /// Parse NAL units from encoder output buffer
    fn parse_nal_units_from_buffer(buffer: &IMFMediaBuffer)
        -> anyhow::Result<Vec<NALUnit>> {
        // Lock buffer to access raw data
        let buffer_lock = BufferLock::lock(buffer)?;
        let data = buffer_lock.as_slice();

        let mut nal_units = Vec::new();
        let mut i = 0;

        while i < data.len() {
            // Look for start code (00 00 00 01 or 00 00 01)
            let start_code_len = if i + 3 < data.len()
                && data[i] == 0x00
                && data[i + 1] == 0x00
                && data[i + 2] == 0x00
                && data[i + 3] == 0x01 {
                4
            } else if i + 2 < data.len()
                && data[i] == 0x00
                && data[i + 1] == 0x00
                && data[i + 2] == 0x01 {
                3
            } else {
                i += 1;
                continue;
            };

            // Parse NAL unit header
            let nal_header_pos = i + start_code_len;
            if nal_header_pos >= data.len() {
                break;
            }

            let nal_header = data[nal_header_pos];
            let nal_type = NALUnitType::from_header(nal_header);

            // Find next start code
            let mut next_start = data.len();
            for j in (nal_header_pos + 1)..data.len() {
                if j + 3 < data.len()
                    && data[j] == 0x00
                    && data[j + 1] == 0x00
                    && data[j + 2] == 0x00
                    && data[j + 3] == 0x01 {
                    next_start = j;
                    break;
                }
                if j + 2 < data.len()
                    && data[j] == 0x00
                    && data[j + 1] == 0x00
                    && data[j + 2] == 0x01 {
                    next_start = j;
                    break;
                }
            }

            // Extract NAL unit data (including start code)
            if let Some(unit_type) = nal_type {
                let data = data[i..next_start].to_vec();
                nal_units.push(NALUnit {
                    unit_type,
                    data,
                });
            }

            i = next_start;
        }

        Ok(nal_units)
    }
}

/// RAII guard for IMFMediaBuffer::Lock/Unlock
struct BufferLock<'a> {
    mf_buffer: &'a IMFMediaBuffer,
    ptr: *mut u8,
    len: usize,
}

impl<'a> BufferLock<'a> {
    fn lock(buffer: &'a IMFMediaBuffer) -> anyhow::Result<Self> {
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;

        api_call!(unsafe {
            buffer.Lock(
                &raw mut ptr,
                None,
                Some(&raw mut len))
        })?;

        Ok(Self { mf_buffer: buffer, ptr, len: len as _ })
    }

    const fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for BufferLock<'_> {
    fn drop(&mut self) {
        unsafe {
            // Ignore error - we're in Drop, can't propagate
            let _ = self.mf_buffer.Unlock();
        }
    }
}
