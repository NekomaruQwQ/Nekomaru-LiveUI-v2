//! Per-stream circular frame buffer with SPS/PPS cache.
//!
//! Frames are pre-serialized on push into the binary format the frontend's
//! `parseStreamFrame()` expects:
//!
//! ```text
//! [u64 LE: timestamp_us]
//! [u32 LE: num_nal_units]
//! for each NAL: [u8: nal_type][u32 LE: data_length][data bytes]
//! ```
//!
//! The `is_keyframe` byte from the IPC wire format is NOT included — the
//! frontend infers keyframe status from NAL unit types.

use live_video::{CodecParams, FrameMessage};

// ── Types ────────────────────────────────────────────────────────────────────

/// A buffered frame with its pre-serialized payload.
pub struct BufferedFrame {
    /// Monotonically increasing sequence number (starts at 1).
    pub sequence: u32,
    /// Whether this frame contains an IDR NAL unit.
    pub is_keyframe: bool,
    /// Pre-serialized binary payload.
    pub payload: Vec<u8>,
}

// ── StreamBuffer ─────────────────────────────────────────────────────────────

/// Fixed-capacity circular buffer for encoded video frames.
///
/// Multiple HTTP clients can read concurrently without draining — reads use
/// sequence-based filtering, not pop semantics.
pub struct StreamBuffer {
    frames: Vec<Option<BufferedFrame>>,
    capacity: usize,
    write_index: usize,
    count: usize,
    next_sequence: u32,
    codec_params: Option<CodecParams>,
}

impl StreamBuffer {
    pub fn new(capacity: usize) -> Self {
        let mut frames = Vec::with_capacity(capacity);
        frames.resize_with(capacity, || None);
        Self {
            frames,
            capacity,
            write_index: 0,
            count: 0,
            next_sequence: 1,
            codec_params: None,
        }
    }

    /// Cache the latest codec parameters (SPS/PPS/resolution).
    pub fn set_codec_params(&mut self, params: CodecParams) {
        self.codec_params = Some(params);
    }

    /// Return the cached codec params, or `None` if the encoder hasn't
    /// produced its first IDR frame yet.
    pub const fn get_codec_params(&self) -> Option<&CodecParams> {
        self.codec_params.as_ref()
    }

    /// Clear all buffered state — frames, codec params, and sequence counter.
    /// Called when the underlying capture process is replaced so stale frames
    /// from the old process are never served.
    pub fn reset(&mut self) {
        for slot in &mut self.frames { *slot = None; }
        self.write_index = 0;
        self.count = 0;
        self.next_sequence = 1;
        self.codec_params = None;
    }

    /// Push a parsed frame into the circular buffer.
    ///
    /// Assigns the next sequence number and pre-serializes the frame payload
    /// so HTTP responses don't need to re-serialize on every request.
    pub fn push_frame(&mut self, frame: &FrameMessage) {
        let sequence = self.next_sequence;
        self.next_sequence += 1;

        let payload = serialize_frame_payload(frame);
        let idx = self.write_index % self.capacity;
        self.frames[idx] = Some(BufferedFrame {
            sequence,
            is_keyframe: frame.is_keyframe,
            payload,
        });

        self.write_index += 1;
        if self.count < self.capacity { self.count += 1; }
    }

    /// Return all buffered frames with `sequence > after_sequence`.
    ///
    /// When `after_sequence` is 0 (first request from a new client),
    /// non-keyframes are skipped until the first keyframe is found — the
    /// WebCodecs decoder needs an IDR frame to initialize.
    pub fn get_frames_after(&self, after_sequence: u32) -> Vec<&BufferedFrame> {
        let mut result = Vec::new();
        let start = self.write_index.wrapping_sub(self.count);
        let mut need_keyframe = after_sequence == 0;

        for i in 0..self.count {
            let raw_idx = start.wrapping_add(i);
            let idx = raw_idx % self.capacity;

            let &Some(ref frame) = &self.frames[idx] else { continue };
            if frame.sequence <= after_sequence { continue; }

            if need_keyframe {
                if !frame.is_keyframe { continue; }
                need_keyframe = false;
            }

            result.push(frame);
        }

        result
    }
}

// ── Serialization ────────────────────────────────────────────────────────────

/// Serialize a `FrameMessage` into the binary format the frontend expects.
///
/// Layout:
/// ```text
/// [u64 LE: timestamp_us]
/// [u32 LE: num_nal_units]
/// for each NAL: [u8: nal_type][u32 LE: data_length][data bytes]
/// ```
///
/// Deliberately omits `is_keyframe` — the frontend infers it from NAL types.
fn serialize_frame_payload(frame: &FrameMessage) -> Vec<u8> {
    let nal_data_size: usize = frame.nal_units.iter()
        .map(|nal| 1 + 4 + nal.data.len())
        .sum();
    let total_size = 8 + 4 + nal_data_size;

    let mut buf = Vec::with_capacity(total_size);

    // Timestamp (u64 LE)
    buf.extend_from_slice(&frame.timestamp_us.to_le_bytes());

    // Number of NAL units (u32 LE)
    buf.extend_from_slice(&(frame.nal_units.len() as u32).to_le_bytes());

    // Each NAL unit
    for nal in &frame.nal_units {
        buf.push(nal.unit_type as u8);
        buf.extend_from_slice(&(nal.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&nal.data);
    }

    buf
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use live_video::{NALUnit, NALUnitType};

    fn make_frame(is_keyframe: bool, timestamp: u64) -> FrameMessage {
        let nal_type = if is_keyframe { NALUnitType::IDR } else { NALUnitType::NonIDR };
        FrameMessage {
            timestamp_us: timestamp,
            is_keyframe,
            nal_units: vec![NALUnit {
                unit_type: nal_type,
                data: vec![0x00, 0x00, 0x01, 0x65],
            }],
        }
    }

    #[test]
    fn push_and_retrieve() {
        let mut buf = StreamBuffer::new(4);
        buf.push_frame(&make_frame(true, 1000));
        buf.push_frame(&make_frame(false, 2000));

        let frames = buf.get_frames_after(0);
        // First request: should skip to first keyframe (sequence 1).
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].sequence, 1);
        assert_eq!(frames[1].sequence, 2);
    }

    #[test]
    fn keyframe_gating_on_first_request() {
        let mut buf = StreamBuffer::new(4);
        buf.push_frame(&make_frame(false, 1000)); // seq 1 — P-frame
        buf.push_frame(&make_frame(false, 2000)); // seq 2 — P-frame
        buf.push_frame(&make_frame(true, 3000));  // seq 3 — keyframe
        buf.push_frame(&make_frame(false, 4000)); // seq 4 — P-frame

        let frames = buf.get_frames_after(0);
        // First request: skip P-frames until the IDR at seq 3.
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].sequence, 3);
        assert!(frames[0].is_keyframe);
        assert_eq!(frames[1].sequence, 4);
    }

    #[test]
    fn circular_wrap() {
        let mut buf = StreamBuffer::new(3);
        for i in 0..5 {
            buf.push_frame(&make_frame(i == 0 || i == 3, (i + 1) * 1000));
        }

        // Buffer capacity is 3, so only last 3 frames remain (seq 3, 4, 5).
        let frames = buf.get_frames_after(0);
        // Keyframe gating: seq 4 is the keyframe (i==3 → seq 4).
        assert_eq!(frames[0].sequence, 4);
        assert!(frames[0].is_keyframe);
    }

    #[test]
    fn get_after_filters_by_sequence() {
        let mut buf = StreamBuffer::new(4);
        buf.push_frame(&make_frame(true, 1000));
        buf.push_frame(&make_frame(false, 2000));
        buf.push_frame(&make_frame(false, 3000));

        let frames = buf.get_frames_after(2);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].sequence, 3);
    }

    #[test]
    fn reset_clears_everything() {
        let mut buf = StreamBuffer::new(4);
        buf.push_frame(&make_frame(true, 1000));
        buf.set_codec_params(CodecParams {
            sps: vec![0x67], pps: vec![0x68], width: 1920, height: 1080,
        });

        buf.reset();

        assert!(buf.get_codec_params().is_none());
        assert!(buf.get_frames_after(0).is_empty());

        // After reset, sequences restart at 1.
        buf.push_frame(&make_frame(true, 5000));
        assert_eq!(buf.get_frames_after(0)[0].sequence, 1);
    }

    #[test]
    fn sequence_monotonicity() {
        let mut buf = StreamBuffer::new(10);
        for i in 0..20 {
            buf.push_frame(&make_frame(i % 5 == 0, i * 1000));
        }

        let frames = buf.get_frames_after(10);
        for pair in frames.windows(2) {
            assert!(pair[1].sequence > pair[0].sequence);
        }
    }
}
