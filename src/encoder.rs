#![expect(
    non_upper_case_globals,
    reason = "false positive on windows-rs constants")]

use nkcore::euclid::*;
use nkcore::*;

use windows::core::*;
use windows::{
    Win32::Graphics::Direct3D11::*,
    Win32::Media::MediaFoundation::*,
    Win32::System::Com::*,
    Win32::System::Variant::*,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;

pub fn print_mft_supported_input_types(transform: &IMFTransform) -> anyhow::Result<()> {
    log::info!("--- Supported Input Types ---");
    for i in 0..100 {
        match unsafe { transform.GetInputAvailableType(0, i) } {
            Ok(media_type) => {
                log::info!("Input type #{}:", i);
                print_media_type(&media_type);
            }
            Err(e) if e.code() == MF_E_NO_MORE_TYPES => {
                log::info!("Total input types: {}", i);
                break;
            }
            Err(e) => {
                log::warn!("Failed to get input type #{}: {:?}", i, e);
                break;
            }
        }
    }
    Ok(())
}

/// Print all supported media types for the given MFT (encoder)
pub fn print_mft_supported_output_types(transform: &IMFTransform) -> anyhow::Result<()> {
    log::info!("--- Supported Output Types ---");
    for i in 0..100 {
        match unsafe { transform.GetOutputAvailableType(0, i) } {
            Ok(media_type) => {
                log::info!("Output type #{}:", i);
                print_media_type(&media_type);
            }
            Err(e) if e.code() == MF_E_NO_MORE_TYPES => {
                log::info!("Total output types: {}", i);
                break;
            }
            Err(e) => {
                log::warn!("Failed to get output type #{}: {:?}", i, e);
                break;
            }
        }
    }
    Ok(())
}

/// Print details of a media type
fn print_media_type(media_type: &IMFMediaType) {
    // Get major type
    if let Ok(major_type) = api_call!(unsafe { media_type.GetGUID(&MF_MT_MAJOR_TYPE) }) {
        log::info!(
            "  Major type: {}",
            name_of_media_type(major_type)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("{major_type:?}")));
    }

    // Get subtype (format)
    if let Ok(subtype) = api_call!(unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }) {
        log::info!(
            "  Subtype: {}",
            name_of_video_format(subtype)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("{subtype:?}")));
    }

    // Get frame size if available
    if let Ok(frame_size) = api_call!(unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }) {
        let width = (frame_size >> 32) as u32;
        let height = (frame_size & 0xFFFFFFFF) as u32;
        log::info!("  Frame size: {}x{}", width, height);
    }

    // Get frame rate if available
    if let Ok(frame_rate) = api_call!(unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }) {
        let numerator = (frame_rate >> 32) as u32;
        let denominator = (frame_rate & 0xFFFFFFFF) as u32;
        if denominator > 0 {
            log::info!("  Frame rate: {}/{} ({:.2} fps)", numerator, denominator, numerator as f64 / denominator as f64);
        }
    }

    // Get interlace mode if available
    if let Ok(interlace) = api_call!(unsafe { media_type.GetUINT32(&MF_MT_INTERLACE_MODE) }) {
        log::info!("  Interlace mode: {:?}", interlace);
    }
}

fn name_of_media_type(guid: GUID) -> Option<&'static str> {
    Some(match guid {
        MFMediaType_Video => "Video",
        MFMediaType_Audio => "Audio",
        _ => None?,
    })
}

fn name_of_video_format(guid: GUID) -> Option<&'static str> {
    Some(match guid {
        MFVideoFormat_NV12 => "NV12",
        MFVideoFormat_H264 => "H.264",
        MFVideoFormat_HEVC => "HEVC",
        MFVideoFormat_RGB32 => "RGB32",
        MFVideoFormat_ARGB32 => "ARGB32",
        _ => None?,
    })
}

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
    /// Timestamp in microseconds
    pub timestamp_us: u64,
}

/// H.264 encoder using Windows Media Foundation
pub struct H264Encoder {
    mf_dxgi_manager: IMFDXGIDeviceManager,
    mf_transform: IMFTransform,
    frame_size: Size2D<u32>,
    frame_count: u64,
}

impl H264Encoder {
    /// Create a new H.264 encoder.
    ///
    /// # Arguments
    /// * `device` - D3D11 device for DXGI integration
    /// * `width` - Video width
    /// * `height` - Video height
    /// * `frame_rate` - Target frame rate (e.g., 60)
    /// * `bitrate` - Target bitrate in bits per second (e.g., 8_000_000 for 8 Mbps)
    #[expect(clippy::panic_in_result_fn, reason = "asserts for valid input")]
    pub fn new(
        device: &ID3D11Device,
        frame_size: Size2D<u32>,
        frame_rate: u32,
        bitrate: u32)
        -> anyhow::Result<Self> {
        assert!(frame_size.width.is_multiple_of(16), "width must be multiple of 16");
        assert!(frame_size.height.is_multiple_of(16), "height must be multiple of 16");

        // Initialize Media Foundation
        api_call!(unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) })?;

        // Find H.264 encoder transform
        let mf_transform =
            Self::find_h264_encoder(&api_call!(device.cast())?)
                .context("failed to find H264Encoder")?;
        let mf_attributes =
            api_call!(unsafe { mf_transform.GetAttributes() })?;

        // Unlock async MFT so we can use it with polling
        api_call!(unsafe { mf_attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1) })
            .context("failed to unlock async MFT")?;
        log::info!("MFT async unlocked");

        // Print supported media types for debugging (after unlock)
        if let Err(e) = print_mft_supported_output_types(&mf_transform) {
            log::warn!("Failed to query MFT supported types: {e:?}");
        }

        // Configure output type (H.264) BEFORE setting D3D manager
        // Get enumerated type and set required parameters
        let output_type = unsafe { mf_transform.GetOutputAvailableType(0, 0)? };

        // Set required parameters
        api_call!(unsafe { output_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate) })?;

        let frame_size_val =
            ((frame_size.width as u64) << 32) |
            (frame_size.height as u64);
        api_call!(unsafe { output_type.SetUINT64(&MF_MT_FRAME_SIZE, frame_size_val) })?;

        let frame_rate_ratio = ((frame_rate as u64) << 32) | 1u64;
        api_call!(unsafe { output_type.SetUINT64(&MF_MT_FRAME_RATE, frame_rate_ratio) })?;

        log::info!("Attempting to set output type ({}x{} @ {}fps, {} bps)...",
            frame_size.width, frame_size.height, frame_rate, bitrate);
        api_call!(unsafe { mf_transform.SetOutputType(0, &output_type, 0) })
            .context("failed to set output type")?;
        log::info!("Output type set successfully!");

        // Print supported input types after output type is set
        if let Err(e) = print_mft_supported_input_types(&mf_transform) {
            log::warn!("Failed to query MFT supported types: {e:?}");
        }

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
        api_call!(unsafe { mf_dxgi_manager.ResetDevice(device, reset_token) })?;

        // NOW set D3D manager after both types are configured
        api_call!(unsafe {
            mf_transform.ProcessMessage(
                MFT_MESSAGE_SET_D3D_MANAGER,
                mf_dxgi_manager_as_unknown.as_raw().addr())
        }).context("failed to set D3D manager")?;
        log::info!("DXGI device manager attached to encoder");

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

        // Query output stream info to check if we need to allocate samples
        let output_stream_info = api_call!(unsafe {
            mf_transform.GetOutputStreamInfo(0)
        })?;

        log::info!("Output stream info:");
        log::info!("  cbSize: {}", output_stream_info.cbSize);
        log::info!("  cbAlignment: {}", output_stream_info.cbAlignment);
        log::info!("  dwFlags: 0x{:08X}", output_stream_info.dwFlags);
        log::info!("  MFT_OUTPUT_STREAM_PROVIDES_SAMPLES: {}",
            (output_stream_info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0);
        log::info!("  MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES: {}",
            (output_stream_info.dwFlags & MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0);

        log::info!(
            "H.264 encoder initialized ({}x{} @ {}fps, {} bps)",
            frame_size.width,
            frame_size.height,
            frame_rate,
            bitrate);

        Ok(Self {
            mf_dxgi_manager,
            mf_transform,
            frame_size,
            frame_count: 0,
        })
    }

    /// Find a hardware H.264 encoder transform
    fn find_h264_encoder(dxgi_device: &IDXGIDevice) -> anyhow::Result<IMFTransform> {
        static INPUT_TYPE: MFT_REGISTER_TYPE_INFO = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };

        static OUTPUT_TYPE: MFT_REGISTER_TYPE_INFO = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };

        // --- Get the LUID from the Device ---
        let dxgi_adapter = api_call!(unsafe { dxgi_device.GetAdapter() })?;
        let dxgi_adapter_desc = api_call!(unsafe { dxgi_adapter.GetDesc() })?;

        // Search for hardware encoder (async)
        let mut out_activate = std::ptr::null_mut();
        let mut out_count = 0u32;
        api_call!(unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT,
                Some(&raw const INPUT_TYPE),
                Some(&raw const OUTPUT_TYPE),
                &raw mut out_activate,
                &raw mut out_count)
        })?;

        if out_activate.is_null() || out_count == 0 {
            anyhow::bail!("no hardware H.264 encoder found");
        }

        log::info!("found {} hardware H.264 encoder(s)", out_count);
        defer(|| unsafe { CoTaskMemFree(Some(out_activate.cast())) });

        // Convert the raw pointer to a slice so we can iterate safely
        let activates = unsafe { std::slice::from_raw_parts(out_activate, out_count as usize) };

        log::info!("scanning {} hardware H.264 encoder(s) for matching adapter...", out_count);
        for (index, activate) in activates.iter().enumerate() {
            // Check if the pointer is valid
            let activate =
                activate
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("null activate pointer at index {}", index))?;

            // Hardware MFTs expose MFT_ENUM_ADAPTER_LUID (as UINT64).
            // This tells us which physical GPU this encoder belongs to.
            // let mft_luid_val = api_call!(unsafe { activate.GetUINT64(&MFT_ENUM_ADAPTER_LUID) })?;
            //
            // // Convert UINT64 back to LUID struct for comparison
            // // LUID is essentially { LowPart: u32, HighPart: i32 }
            // let mft_luid = LUID {
            //     LowPart: mft_luid_val as u32,
            //     HighPart: (mft_luid_val >> 32) as i32,
            // };
            //
            // log::info!(
            //     "encoder index {} has LUID {{ LowPart: {}, HighPart: {} }}",
            //     index,
            //     mft_luid.LowPart,
            //     mft_luid.HighPart);
            // log::info!(
            //     "current adapter LUID {{ LowPart: {}, HighPart: {} }}",
            //     dxgi_adapter_desc.AdapterLuid.LowPart,
            //     dxgi_adapter_desc.AdapterLuid.HighPart);
            //
            // if mft_luid == dxgi_adapter_desc.AdapterLuid {
            //     log::info!("Found matching encoder at index {}", index);
            //     return Ok(api_call!(unsafe {
            //         activate.ActivateObject::<IMFTransform>()
            //     })?);
            // }

            let mut buf = [0u16; 256];
            let mut len = 0u32;
            api_call!(unsafe {
                activate.GetString(
                    &MFT_FRIENDLY_NAME_Attribute,
                    &mut buf,
                    Some(&raw mut len))
            })?;

            let name = unsafe {
                widestring::U16Str::from_ptr(
                    buf.as_ptr(),
                    len as _)
            };

            let name = name.to_string_lossy();
            log::info!("Encoder #{}: '{}'", index, name);

            // Activate this encoder
            if name.to_ascii_lowercase().contains("nvidia") {
                log::info!("Selecting encoder #{} ('{}')", index, name);
                return Ok(api_call!(unsafe {
                    activate.ActivateObject::<IMFTransform>()
                })?);
            }
        }

        anyhow::bail!("no matching hardware H.264 encoder found for the current adapter");
    }

    /// Configure input media type (NV12)
    #[cfg(false)]
    fn configure_input_type(
        transform: &IMFTransform,
        frame_size: Size2D<u32>,
        frame_rate: u32)
        -> anyhow::Result<()> {
        let input_type = api_call!(unsafe { MFCreateMediaType() })?;

        api_call!(unsafe { input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) })
            .with_context(|| context!("failed to set input major type to video"))?;
        api_call!(unsafe { input_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12) })
            .with_context(|| context!("failed to set input subtype to NV12"))?;

        let frame_size =
            ((frame_size.width as u64) << 32) |
            (frame_size.height as u64);
        api_call!(unsafe { input_type.SetUINT64(&MF_MT_FRAME_SIZE, frame_size) })
            .with_context(|| context!("failed to set input frame size"))?;

        let frame_rate_ratio = ((frame_rate as u64) << 32) | 1u64;
        api_call!(unsafe { input_type.SetUINT64(&MF_MT_FRAME_RATE, frame_rate_ratio) })
            .with_context(|| context!("failed to set input frame rate"))?;

        api_call!(unsafe { transform.SetInputType(0, &input_type, 0) })
            .with_context(|| context!("failed to set encoder input type"))?;
        Ok(())
    }

    /// Configure output media type (H.264) with low-latency settings
    #[cfg(false)]
    fn configure_output_type(
        transform: &IMFTransform,
        frame_size: Size2D<u32>,
        frame_rate: u32,
        bitrate: u32)
        -> anyhow::Result<()> {
        let output_type = api_call!(unsafe { MFCreateMediaType() })?;

        let frame_size_packed =
            ((frame_size.width as u64) << 32) | (frame_size.height as u64);
        let frame_rate_packed =
            ((frame_rate as u64) << 32) | 1u64;

        api_call!(unsafe { output_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) })
            .context("failed to set output major type")?;
        api_call!(unsafe { output_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264) })
            .context("failed to set output subtype to H.264")?;
        api_call!(unsafe { output_type.SetUINT64(&MF_MT_FRAME_SIZE, frame_size_packed) })
            .context("failed to set output frame size")?;
        api_call!(unsafe { output_type.SetUINT64(&MF_MT_FRAME_RATE, frame_rate_packed) })
            .context("failed to set output frame rate")?;
        api_call!(unsafe { output_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate) })
            .context("failed to set output bitrate")?;

        // Baseline profile for maximum compatibility
        api_call!(unsafe { output_type.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32) })
            .context("failed to set H.264 profile to baseline")?;

        // Hardware encoders fail if you don't explicitly say "Progressive"
        api_call!(unsafe {
            output_type.SetUINT32(
                &MF_MT_INTERLACE_MODE,
                MFVideoInterlace_Progressive.0 as u32)
        })
            .context("failed to set interlace mode to progressive")?;

        // --- [FIX] Explicitly Set Profile and Level ---
        // eAVEncH264VProfile_High = {54041196-23BB-45F7-9684-8073A642E325}
        // eAVEncH264VLevel5_1     = 51 (decimal)

        // // Note: You might need to define these GUIDs/Constants if windows-rs
        // // doesn't export them conveniently in the version you are using.
        // // eAVEncH264VProfile_High
        // api_call!(unsafe {
        //     output_type.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as _)
        // })?;
        //
        // // eAVEncH264VLevel5_1 (Enum value 51)
        // // This unlocks 4K resolution and higher macroblock throughput.
        // api_call!(unsafe {
        //     output_type.SetUINT32(&MF_MT_MPEG2_LEVEL, eAVEncH264VLevel5_1.0 as _)
        // })?;

        api_call!(unsafe { transform.SetOutputType(0, &output_type, 0) })
            .context("failed to set encoder output type")?;


        api_call!(unsafe { transform.SetOutputType(0, &output_type, 0) })
            .context("failed to set encoder output type")?;

        // Configure low-latency settings via ICodecAPI
        let codec_api = api_call!(transform.cast::<ICodecAPI>())?;

        for (api, value) in [
            // No B-frames for low latency
            (CODECAPI_AVEncMPVDefaultBPictureCount, VARIANT::from(0)),
            // GOP size = 2 seconds
            (CODECAPI_AVEncMPVGOPSize, VARIANT::from(frame_rate * 2)),
            // Low latency mode
            (CODECAPI_AVLowLatencyMode, VARIANT::from(true)),
            // CBR rate control
            (CODECAPI_AVEncCommonRateControlMode,
                VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32)),
        ] {
            api_call!(unsafe { codec_api.SetValue(&api, &value) })
                .context("failed to set codec API value")?;
        }

        Ok(())
    }

    /// Encode a single NV12 frame to H.264 NAL units.
    ///
    /// # Arguments
    /// * `nv12_texture` - Input NV12 texture
    /// * `timestamp_us` - Timestamp in microseconds
    ///
    /// # Returns
    /// Vector of encoded NAL units. May be empty if encoder is buffering.
    pub fn encode_frame(
        &mut self,
        nv12_texture: &ID3D11Texture2D,
        timestamp_us: u64)
        -> anyhow::Result<Vec<NALUnit>> {
        // Create DXGI surface buffer from texture
        let buffer = api_call!(unsafe {
            MFCreateDXGISurfaceBuffer(
                &ID3D11Texture2D::IID,
                nv12_texture,
                0,
                false)
        })?;

        // Create MF sample
        let sample = api_call!(unsafe { MFCreateSample() })?;

        // Add buffer to sample
        api_call!(unsafe { sample.AddBuffer(&buffer) })?;

        // Set sample time (convert μs to 100ns units)
        api_call!(unsafe { sample.SetSampleTime((timestamp_us * 10) as i64) })?;

        // Set sample duration (frame duration at target fps)
        let duration_100ns = (1_000_000 * 10) / 60;  // ~16.666ms for 60fps
        api_call!(unsafe { sample.SetSampleDuration(duration_100ns) })?;

        // Feed sample to encoder
        api_call!(unsafe { self.mf_transform.ProcessInput(0, &sample, 0) })?;

        // Drain encoded output
        let nal_units = self.drain_encoder_output(timestamp_us)?;

        self.frame_count += 1;

        Ok(nal_units)
    }

    /// Drain encoded NAL units from encoder
    fn drain_encoder_output(&self, timestamp_us: u64) -> anyhow::Result<Vec<NALUnit>> {
        let mut nal_units = Vec::new();

        loop {
            let mut output_buffers = [MFT_OUTPUT_DATA_BUFFER::default()];
            let mut status = 0u32;
            let result = unsafe {
                self.mf_transform.ProcessOutput(
                    0,
                    &mut output_buffers,
                    &raw mut status)
            };

            match result {
                Ok(_) => {
                    if let Some(sample) = output_buffers[0].pSample.take() {
                        // Convert to contiguous buffer and parse NAL units
                        match api_call!(unsafe { sample.ConvertToContiguousBuffer() })
                            .with_context(|| context!("converting to contiguous buffer"))
                        {
                            Ok(buffer) => {
                                match Self::parse_nal_units_from_buffer(&buffer, timestamp_us) {
                                    Ok(units) => nal_units.extend(units),
                                    Err(e) => {
                                        log::warn!("Failed to parse NAL units: {e:?}");
                                        // Continue - skip this frame's NAL units
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to convert buffer: {e:?}");
                                // Continue - skip this frame
                            }
                        }
                    }
                }
                Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                    // Normal condition - encoder needs more input
                    break;
                }
                Err(err) => {
                    log::error!("ProcessOutput failed: {:?}", err);
                    break;
                }
            }
        }

        Ok(nal_units)
    }

    /// Parse NAL units from encoder output buffer
    fn parse_nal_units_from_buffer(buffer: &IMFMediaBuffer, timestamp_us: u64)
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
                    timestamp_us,
                });
            }

            i = next_start;
        }

        Ok(nal_units)
    }
}

/// RAII guard for IMFMediaBuffer::Lock/Unlock
struct BufferLock<'a> {
    buffer: &'a IMFMediaBuffer,
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

        Ok(Self { buffer, ptr, len: len as _ })
    }

    const fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for BufferLock<'_> {
    fn drop(&mut self) {
        unsafe {
            // Ignore error - we're in Drop, can't propagate
            let _ = self.buffer.Unlock();
        }
    }
}
