//! IPC wire protocol for `live-audio`.
//!
//! Defines the binary message format used to stream raw PCM audio from the
//! capture process to the server over stdout.
//!
//! ## Wire Format
//!
//! Same envelope as `live-capture`:
//! ```text
//! [u8:  message_type]
//! [u32 LE: payload_length]
//! [payload_length bytes: payload]
//! ```
//!
//! Audio message types use the `0x1x` range to avoid collision with video
//! message types (`0x01`–`0x02`).

use std::io;
use std::io::*;

// ── IPC Message Types ───────────────────────────────────────────────────────

/// Discriminant byte for each audio IPC message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    /// Audio format parameters (sample rate, channels, bit depth).
    /// Sent once after device initialization.
    AudioParams = 0x10,
    /// One chunk of raw PCM audio data with a wall-clock timestamp.
    AudioFrame = 0x11,
    /// Non-fatal error description (UTF-8).
    Error = 0xFF,
}

/// Audio format parameters sent once at capture start.
///
/// The server caches these to inform the frontend's `AudioContext` setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioParams {
    /// Samples per second (e.g. 48000).
    pub sample_rate: u32,
    /// Number of audio channels (e.g. 2 for stereo).
    pub channels: u8,
    /// Bits per sample (e.g. 16 for s16le).
    pub bits_per_sample: u8,
}

/// A single chunk of raw PCM audio data.
///
/// Each chunk contains a fixed number of samples (typically 480 = 10ms at
/// 48kHz) as interleaved s16le data.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// Wall-clock timestamp in microseconds since Unix epoch.
    /// Uses `SystemTime::now().duration_since(UNIX_EPOCH)` — same clock as
    /// `live-capture`'s video frame timestamps, enabling frontend A/V sync.
    pub timestamp_us: u64,
    /// Raw interleaved PCM samples (s16le, `channels` interleaved).
    pub pcm_data: Vec<u8>,
}

/// A parsed audio IPC message from the capture process.
#[derive(Debug, Clone)]
pub enum Message {
    AudioParams(AudioParams),
    AudioFrame(AudioFrame),
    Error(String),
}

// ── Serialization (write to stdout) ─────────────────────────────────────────

/// Write an `AudioParams` message.
///
/// Wire layout:
/// ```text
/// [u32 LE: sample_rate][u8: channels][u8: bits_per_sample]
/// ```
pub fn write_audio_params(w: &mut impl Write, params: &AudioParams) -> io::Result<()> {
    let payload_len: u32 = 4 + 1 + 1; // sample_rate(4) + channels(1) + bps(1)

    w.write_all(&[MessageType::AudioParams as u8])?;
    w.write_all(&payload_len.to_le_bytes())?;

    w.write_all(&params.sample_rate.to_le_bytes())?;
    w.write_all(&[params.channels])?;
    w.write_all(&[params.bits_per_sample])?;

    w.flush()
}

/// Write an `AudioFrame` message.
///
/// Wire layout:
/// ```text
/// [u64 LE: timestamp_us][raw PCM bytes]
/// ```
pub fn write_audio_frame(w: &mut impl Write, frame: &AudioFrame) -> io::Result<()> {
    let payload_len = 8 + frame.pcm_data.len(); // timestamp(8) + pcm

    w.write_all(&[MessageType::AudioFrame as u8])?;
    w.write_all(&(payload_len as u32).to_le_bytes())?;

    w.write_all(&frame.timestamp_us.to_le_bytes())?;
    w.write_all(&frame.pcm_data)?;

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

/// Read the next audio IPC message from a byte stream.
///
/// Returns `Ok(None)` on clean EOF (stream ended between messages).
/// Returns `Err` on malformed data or unexpected EOF mid-message.
pub fn read_message(r: &mut impl Read) -> io::Result<Option<Message>> {
    // Read message type (1 byte)
    let mut type_buf = [0u8; 1];
    match r.read_exact(&mut type_buf) {
        Ok(()) => {},
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => Err(e)?,
    }

    // Read payload length (4 bytes LE)
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let payload_len = u32::from_le_bytes(len_buf) as usize;

    // Read payload
    let mut payload = vec![0u8; payload_len];
    r.read_exact(&mut payload)?;

    match type_buf[0] {
        0x10 => read_audio_params_payload(&payload).map(|p| Some(Message::AudioParams(p))),
        0x11 => read_audio_frame_payload(&payload).map(|f| Some(Message::AudioFrame(f))),
        0xFF => read_error_payload(&payload).map(|e| Some(Message::Error(e))),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown message type: 0x{other:02X}"))),
    }
}

#[expect(clippy::missing_asserts_for_indexing, reason = "clippy execution flow analysis fails to see that length checks prevent out-of-bounds")]
/// Parse an `AudioParams` payload.
fn read_audio_params_payload(data: &[u8]) -> io::Result<AudioParams> {
    if data.len() < 6 {
        Err(io::Error::new(io::ErrorKind::InvalidData, "truncated AudioParams payload"))?;
    }

    let sample_rate = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let channels = data[4];
    let bits_per_sample = data[5];

    Ok(AudioParams { sample_rate, channels, bits_per_sample })
}

/// Parse an `AudioFrame` payload.
fn read_audio_frame_payload(data: &[u8]) -> io::Result<AudioFrame> {
    if data.len() < 8 {
        Err(io::Error::new(io::ErrorKind::InvalidData, "truncated AudioFrame payload"))?;
    }

    let timestamp_us = u64::from_le_bytes(
        data[0..8].try_into().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?);
    let pcm_data = data[8..].to_vec();

    Ok(AudioFrame { timestamp_us, pcm_data })
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
    fn round_trip_audio_params() {
        let params = AudioParams {
            sample_rate: 48000,
            channels: 2,
            bits_per_sample: 16,
        };

        let mut buf = Vec::new();
        write_audio_params(&mut buf, &params).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_message(&mut cursor).unwrap().unwrap();

        match msg {
            Message::AudioParams(decoded) => {
                assert_eq!(decoded, params);
            },
            other => panic!("expected AudioParams, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_audio_frame() {
        // 4 stereo s16le samples (16 bytes)
        let pcm: Vec<u8> = vec![
            0x00, 0x10, 0x00, 0x20, // L=4096, R=8192
            0xFF, 0x7F, 0x01, 0x80, // L=32767, R=-32767
            0x00, 0x00, 0x00, 0x00, // silence
            0xAB, 0xCD, 0xEF, 0x01, // arbitrary
        ];
        let frame = AudioFrame {
            timestamp_us: 1_000_000,
            pcm_data: pcm.clone(),
        };

        let mut buf = Vec::new();
        write_audio_frame(&mut buf, &frame).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_message(&mut cursor).unwrap().unwrap();

        match msg {
            Message::AudioFrame(decoded) => {
                assert_eq!(decoded.timestamp_us, 1_000_000);
                assert_eq!(decoded.pcm_data, pcm);
            },
            other => panic!("expected AudioFrame, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_error() {
        let msg_text = "device disconnected";

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
        assert!(read_message(&mut cursor).unwrap().is_none());
    }

    /// Verify that multiple messages can be read sequentially from the same stream.
    #[test]
    fn sequential_messages() {
        let mut buf = Vec::new();

        let params = AudioParams {
            sample_rate: 44100,
            channels: 1,
            bits_per_sample: 16,
        };
        write_audio_params(&mut buf, &params).unwrap();

        let frame = AudioFrame {
            timestamp_us: 500_000,
            pcm_data: vec![0x01, 0x02, 0x03, 0x04],
        };
        write_audio_frame(&mut buf, &frame).unwrap();

        write_error(&mut buf, "test error").unwrap();

        let mut cursor = Cursor::new(&buf);
        assert!(matches!(read_message(&mut cursor).unwrap(), Some(Message::AudioParams(_))));
        assert!(matches!(read_message(&mut cursor).unwrap(), Some(Message::AudioFrame(_))));
        assert!(matches!(read_message(&mut cursor).unwrap(), Some(Message::Error(_))));
        assert!(read_message(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn empty_pcm_frame() {
        let frame = AudioFrame {
            timestamp_us: 0,
            pcm_data: vec![],
        };

        let mut buf = Vec::new();
        write_audio_frame(&mut buf, &frame).unwrap();

        let mut cursor = Cursor::new(&buf);
        let msg = read_message(&mut cursor).unwrap().unwrap();

        match msg {
            Message::AudioFrame(decoded) => {
                assert_eq!(decoded.timestamp_us, 0);
                assert!(decoded.pcm_data.is_empty());
            },
            other => panic!("expected AudioFrame, got {other:?}"),
        }
    }
}
