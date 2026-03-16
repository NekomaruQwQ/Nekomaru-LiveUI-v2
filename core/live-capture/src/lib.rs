//! IPC wire protocol for `live-capture`.
//!
//! Defines the binary message format used to communicate encoded H.264 frames
//! from the capture process to the server over stdout.
//!
//! ## Wire Format
//!
//! Every message is length-prefixed:
//! ```text
//! [u8:  message_type]
//! [u32 LE: payload_length]
//! [payload_length bytes: payload]
//! ```

use std::io;
use std::io::*;

// ── H.264 NAL Unit Types ────────────────────────────────────────────────────

/// NAL unit types relevant to our H.264 baseline profile stream.
///
/// The 5-bit `nal_unit_type` field is defined in ITU-T H.264 Table 7-1.
/// We only handle the four types that appear in a baseline-profile stream
/// with no B-frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NALUnitType {
    /// Non-IDR slice (inter-predicted P-frame).
    NonIDR = 1,
    /// Instantaneous Decoder Refresh (keyframe).
    IDR = 5,
    /// Sequence Parameter Set — codec configuration.
    SPS = 7,
    /// Picture Parameter Set — picture-level parameters.
    PPS = 8,
}

impl NALUnitType {
    /// Parse NAL unit type from the first byte after a start code.
    ///
    /// The type is encoded in the lower 5 bits of the NAL header byte
    /// (H.264 spec section 7.3.1).  Returns `None` for types we don't handle.
    pub const fn from_header(header: u8) -> Option<Self> {
        match header & 0x1F {
            1 => Some(Self::NonIDR),
            5 => Some(Self::IDR),
            7 => Some(Self::SPS),
            8 => Some(Self::PPS),
            _ => None,
        }
    }
}

/// A single encoded H.264 NAL unit.
#[derive(Debug, Clone)]
pub struct NALUnit {
    /// Type of this NAL unit.
    pub unit_type: NALUnitType,
    /// Raw NAL unit data including the Annex B start code (`00 00 00 01` or `00 00 01`).
    pub data: Vec<u8>,
}

// ── IPC Message Types ───────────────────────────────────────────────────────

/// Discriminant byte for each IPC message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    /// Codec initialization parameters (SPS, PPS, resolution).
    /// Sent once after encoder init and again if parameters change.
    CodecParams = 0x01,
    /// One encoded video frame with its NAL units.
    Frame = 0x02,
    /// Non-fatal error description (UTF-8).
    /// Fatal errors are signaled by process exit instead.
    Error = 0xFF,
}

/// Codec parameters extracted from the H.264 stream.
///
/// Sent as a `0x01` message on the first IDR frame and whenever SPS/PPS change.
/// The server caches these to initialize late-joining clients.
#[derive(Debug, Clone)]
pub struct CodecParams {
    /// Sequence Parameter Set (raw NAL data without start code).
    pub sps: Vec<u8>,
    /// Picture Parameter Set (raw NAL data without start code).
    pub pps: Vec<u8>,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
}

/// A single encoded frame ready for IPC transmission.
///
/// Sent as a `0x02` message for every frame the encoder produces.
#[derive(Debug, Clone)]
pub struct FrameMessage {
    /// Wall-clock timestamp in microseconds since Unix epoch.
    pub timestamp_us: u64,
    /// Whether this frame contains an IDR NAL unit (keyframe).
    pub is_keyframe: bool,
    /// All NAL units that make up this frame.
    pub nal_units: Vec<NALUnit>,
}

/// A parsed IPC message from the capture process.
#[derive(Debug, Clone)]
pub enum Message {
    CodecParams(CodecParams),
    Frame(FrameMessage),
    Error(String),
}

// ── Serialization (write to stdout) ─────────────────────────────────────────

/// Write a `CodecParams` message.
///
/// Wire layout:
/// ```text
/// [u16 LE: width][u16 LE: height]
/// [u16 LE: sps_length][sps bytes]
/// [u16 LE: pps_length][pps bytes]
/// ```
pub fn write_codec_params(w: &mut impl Write, params: &CodecParams) -> io::Result<()> {
    debug_assert!(u16::try_from(params.width).is_ok(), "width exceeds u16 range");
    debug_assert!(u16::try_from(params.height).is_ok(), "height exceeds u16 range");
    debug_assert!(u16::try_from(params.sps.len()).is_ok(), "SPS exceeds u16 length");
    debug_assert!(u16::try_from(params.pps.len()).is_ok(), "PPS exceeds u16 length");

    let payload_len = 2 + 2 + 2 + params.sps.len() + 2 + params.pps.len();

    // Header
    w.write_all(&[MessageType::CodecParams as u8])?;
    w.write_all(&(payload_len as u32).to_le_bytes())?;

    // Payload
    w.write_all(&(params.width as u16).to_le_bytes())?;
    w.write_all(&(params.height as u16).to_le_bytes())?;
    w.write_all(&(params.sps.len() as u16).to_le_bytes())?;
    w.write_all(&params.sps)?;
    w.write_all(&(params.pps.len() as u16).to_le_bytes())?;
    w.write_all(&params.pps)?;

    w.flush()
}

/// Write a `Frame` message.
///
/// Wire layout:
/// ```text
/// [u64 LE: timestamp_us][u8: is_keyframe]
/// [u32 LE: num_nal_units]
/// for each NAL: [u8: nal_type][u32 LE: data_length][data bytes]
/// ```
pub fn write_frame(w: &mut impl Write, frame: &FrameMessage) -> io::Result<()> {
    // Pre-compute payload length to write the header
    let nal_data_len: usize = frame.nal_units.iter()
        .map(|nal| 1 + 4 + nal.data.len()) // type(1) + length(4) + data
        .sum();
    let payload_len = 8 + 1 + 4 + nal_data_len; // timestamp(8) + keyframe(1) + count(4)

    // Header
    w.write_all(&[MessageType::Frame as u8])?;
    w.write_all(&(payload_len as u32).to_le_bytes())?;

    // Payload
    w.write_all(&frame.timestamp_us.to_le_bytes())?;
    w.write_all(&[u8::from(frame.is_keyframe)])?;
    w.write_all(&(frame.nal_units.len() as u32).to_le_bytes())?;

    for nal in &frame.nal_units {
        w.write_all(&[nal.unit_type as u8])?;
        w.write_all(&(nal.data.len() as u32).to_le_bytes())?;
        w.write_all(&nal.data)?;
    }

    w.flush()
}

/// Write an `Error` message.
///
/// Wire layout: raw UTF-8 bytes of the error description.
pub fn write_error(w: &mut impl Write, message: &str) -> io::Result<()> {
    let payload = message.as_bytes();

    w.write_all(&[MessageType::Error as u8])?;
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;

    w.flush()
}

// ── Deserialization (read from stdout pipe) ─────────────────────────────────

/// Read the next IPC message from a byte stream.
///
/// Returns `Ok(None)` on clean EOF (stream ended between messages).
/// Returns `Err` on malformed data or unexpected EOF mid-message.
pub fn read_message(r: &mut impl Read) -> io::Result<Option<Message>> {
    // Read message type (1 byte)
    let mut type_buf = [0u8; 1];
    match r.read_exact(&mut type_buf) {
        Ok(()) => {},
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    // Read payload length (4 bytes LE)
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let payload_len = u32::from_le_bytes(len_buf) as usize;

    // Read payload
    let mut payload = vec![0u8; payload_len];
    r.read_exact(&mut payload)?;

    match type_buf[0] {
        0x01 => read_codec_params_payload(&payload).map(|p| Some(Message::CodecParams(p))),
        0x02 => read_frame_payload(&payload).map(|f| Some(Message::Frame(f))),
        0xFF => read_error_payload(&payload).map(|e| Some(Message::Error(e))),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown message type: 0x{other:02X}"))),
    }
}

/// Parse a `CodecParams` payload.
fn read_codec_params_payload(data: &[u8]) -> io::Result<CodecParams> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "truncated CodecParams payload");
    if data.len() < 8 { return Err(invalid()); }

    let mut pos = 0;

    let width = u16::from_le_bytes([data[pos], data[pos + 1]]) as u32;
    pos += 2;
    let height = u16::from_le_bytes([data[pos], data[pos + 1]]) as u32;
    pos += 2;

    let sps_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    if pos + sps_len > data.len() { return Err(invalid()); }
    let sps = data[pos..pos + sps_len].to_vec();
    pos += sps_len;

    if pos + 2 > data.len() { return Err(invalid()); }
    let pps_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2;
    if pos + pps_len > data.len() { return Err(invalid()); }
    let pps = data[pos..pos + pps_len].to_vec();

    Ok(CodecParams { sps, pps, width, height })
}

/// Parse a Frame payload.
fn read_frame_payload(data: &[u8]) -> io::Result<FrameMessage> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "truncated Frame payload");
    if data.len() < 13 { return Err(invalid()); } // 8 + 1 + 4

    let mut pos = 0;

    let timestamp_us = u64::from_le_bytes(
        data[pos..pos + 8].try_into().map_err(|_e| invalid())?);
    pos += 8;
    let is_keyframe = data[pos] != 0;
    pos += 1;
    let num_nals = u32::from_le_bytes(
        data[pos..pos + 4].try_into().map_err(|_e| invalid())?) as usize;
    pos += 4;

    let mut nal_units = Vec::with_capacity(num_nals);
    for _ in 0..num_nals {
        if pos + 5 > data.len() { return Err(invalid()); } // type(1) + length(4)

        let nal_type_byte = data[pos];
        pos += 1;
        let nal_data_len = u32::from_le_bytes(
            data[pos..pos + 4].try_into().map_err(|_e| invalid())?) as usize;
        pos += 4;

        if pos + nal_data_len > data.len() { return Err(invalid()); }
        let nal_data = data[pos..pos + nal_data_len].to_vec();
        pos += nal_data_len;

        let unit_type = NALUnitType::from_header(nal_type_byte)
            .ok_or_else(|| io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown NAL unit type: {nal_type_byte}")))?;

        nal_units.push(NALUnit { unit_type, data: nal_data });
    }

    Ok(FrameMessage { timestamp_us, is_keyframe, nal_units })
}

/// Parse an Error payload as UTF-8.
fn read_error_payload(data: &[u8]) -> io::Result<String> {
    String::from_utf8(data.to_vec())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_codec_params() {
        let params = CodecParams {
            sps: vec![0x67, 0x42, 0xC0, 0x1E, 0xD9, 0x00, 0xA0, 0x47, 0xFE, 0xC8],
            pps: vec![0x68, 0xCE, 0x38, 0x80],
            width: 1920,
            height: 1200,
        };

        let mut buf = Vec::new();
        write_codec_params(&mut buf, &params).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_message(&mut cursor).unwrap().unwrap();

        match msg {
            Message::CodecParams(decoded) => {
                assert_eq!(decoded.width, 1920);
                assert_eq!(decoded.height, 1200);
                assert_eq!(decoded.sps, params.sps);
                assert_eq!(decoded.pps, params.pps);
            },
            other => panic!("expected CodecParams, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_frame() {
        let frame = FrameMessage {
            timestamp_us: 16_667,
            is_keyframe: true,
            nal_units: vec![
                NALUnit {
                    unit_type: NALUnitType::SPS,
                    data: vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42],
                },
                NALUnit {
                    unit_type: NALUnitType::PPS,
                    data: vec![0x00, 0x00, 0x00, 0x01, 0x68, 0xCE],
                },
                NALUnit {
                    unit_type: NALUnitType::IDR,
                    data: vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x80, 0x40],
                },
            ],
        };

        let mut buf = Vec::new();
        write_frame(&mut buf, &frame).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_message(&mut cursor).unwrap().unwrap();

        match msg {
            Message::Frame(decoded) => {
                assert_eq!(decoded.timestamp_us, 16_667);
                assert!(decoded.is_keyframe);
                assert_eq!(decoded.nal_units.len(), 3);
                assert_eq!(decoded.nal_units[0].unit_type, NALUnitType::SPS);
                assert_eq!(decoded.nal_units[1].unit_type, NALUnitType::PPS);
                assert_eq!(decoded.nal_units[2].unit_type, NALUnitType::IDR);
                assert_eq!(decoded.nal_units[2].data, frame.nal_units[2].data);
            },
            other => panic!("expected Frame, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_error() {
        let msg_text = "capture session lost: window closed";

        let mut buf = Vec::new();
        write_error(&mut buf, msg_text).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_message(&mut cursor).unwrap().unwrap();

        match msg {
            Message::Error(decoded) => assert_eq!(decoded, msg_text),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn read_message_returns_none_on_eof() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let msg = read_message(&mut cursor).unwrap();
        assert!(msg.is_none());
    }

    /// Verify that multiple messages can be read sequentially from the same stream.
    #[test]
    fn sequential_messages() {
        let mut buf = Vec::new();

        let params = CodecParams {
            sps: vec![0x67],
            pps: vec![0x68],
            width: 640,
            height: 480,
        };
        write_codec_params(&mut buf, &params).unwrap();

        let frame = FrameMessage {
            timestamp_us: 0,
            is_keyframe: true,
            nal_units: vec![NALUnit {
                unit_type: NALUnitType::IDR,
                data: vec![0x00, 0x00, 0x01, 0x65],
            }],
        };
        write_frame(&mut buf, &frame).unwrap();

        write_error(&mut buf, "test error").unwrap();

        let mut cursor = Cursor::new(&buf);
        assert!(matches!(read_message(&mut cursor).unwrap(), Some(Message::CodecParams(_))));
        assert!(matches!(read_message(&mut cursor).unwrap(), Some(Message::Frame(_))));
        assert!(matches!(read_message(&mut cursor).unwrap(), Some(Message::Error(_))));
        assert!(read_message(&mut cursor).unwrap().is_none());
    }
}
