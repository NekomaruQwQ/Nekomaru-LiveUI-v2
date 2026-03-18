//! Circular audio chunk buffer.
//!
//! Simpler than the video buffer — no keyframe gating needed because every
//! PCM chunk is independently decodable.  Chunks are pre-serialized on push.
//!
//! Pre-serialized payload: `[u64 LE: timestamp_us][raw PCM s16le bytes]`.

use live_audio::{AudioChunk, AudioParams};

// ── Types ────────────────────────────────────────────────────────────────────

pub struct BufferedChunk {
    pub sequence: u32,
    pub payload: Vec<u8>,
}

// ── AudioBuffer ──────────────────────────────────────────────────────────────

pub struct AudioBuffer {
    chunks: Vec<Option<BufferedChunk>>,
    capacity: usize,
    write_index: usize,
    count: usize,
    next_sequence: u32,
    audio_params: Option<AudioParams>,
}

impl AudioBuffer {
    pub fn new(capacity: usize) -> Self {
        let mut chunks = Vec::with_capacity(capacity);
        chunks.resize_with(capacity, || None);
        Self {
            chunks,
            capacity,
            write_index: 0,
            count: 0,
            next_sequence: 1,
            audio_params: None,
        }
    }

    pub const fn set_audio_params(&mut self, params: AudioParams) {
        self.audio_params = Some(params);
    }

    pub const fn get_audio_params(&self) -> Option<&AudioParams> {
        self.audio_params.as_ref()
    }

    pub fn reset(&mut self) {
        for slot in &mut self.chunks { *slot = None; }
        self.write_index = 0;
        self.count = 0;
        self.next_sequence = 1;
        self.audio_params = None;
    }

    pub fn push_chunk(&mut self, chunk: &AudioChunk) {
        let sequence = self.next_sequence;
        self.next_sequence += 1;

        // Pre-serialize: [u64 LE timestamp][PCM bytes]
        let mut payload = Vec::with_capacity(8 + chunk.pcm_data.len());
        payload.extend_from_slice(&chunk.timestamp_us.to_le_bytes());
        payload.extend_from_slice(&chunk.pcm_data);

        let idx = self.write_index % self.capacity;
        self.chunks[idx] = Some(BufferedChunk { sequence, payload });

        self.write_index += 1;
        if self.count < self.capacity { self.count += 1; }
    }

    /// Return all chunks with `sequence > after_sequence`.
    ///
    /// If `after_sequence` exceeds the buffer's max sequence (e.g., after a
    /// process restart), returns all buffered chunks (generation reset).
    pub fn get_chunks_after(&self, mut after_sequence: u32) -> Vec<&BufferedChunk> {
        let max_seq = self.next_sequence.saturating_sub(1);
        if after_sequence > max_seq {
            after_sequence = 0;
        }

        let mut result = Vec::new();
        let start = self.write_index.wrapping_sub(self.count);

        for i in 0..self.count {
            let raw_idx = start.wrapping_add(i);
            let idx = raw_idx % self.capacity;
            let &Some(ref chunk) = &self.chunks[idx] else { continue };
            if chunk.sequence <= after_sequence { continue; }
            result.push(chunk);
        }

        result
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(ts: u64) -> AudioChunk {
        AudioChunk {
            timestamp_us: ts,
            pcm_data: vec![0x01, 0x02, 0x03, 0x04],
        }
    }

    #[test]
    fn push_and_retrieve() {
        let mut buf = AudioBuffer::new(4);
        buf.push_chunk(&make_chunk(1000));
        buf.push_chunk(&make_chunk(2000));

        let chunks = buf.get_chunks_after(0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].sequence, 1);
        assert_eq!(chunks[1].sequence, 2);
    }

    #[test]
    fn generation_reset() {
        let mut buf = AudioBuffer::new(4);
        buf.push_chunk(&make_chunk(1000));
        buf.push_chunk(&make_chunk(2000));

        // Caller has sequence 100 (from previous generation) — should get everything.
        let chunks = buf.get_chunks_after(100);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn circular_wrap() {
        let mut buf = AudioBuffer::new(3);
        for i in 0..5 {
            buf.push_chunk(&make_chunk((i + 1) * 1000));
        }

        // Only last 3 remain (seq 3, 4, 5).
        let chunks = buf.get_chunks_after(0);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].sequence, 3);
    }

    #[test]
    fn reset_clears_everything() {
        let mut buf = AudioBuffer::new(4);
        buf.push_chunk(&make_chunk(1000));
        buf.set_audio_params(AudioParams { sample_rate: 48000, channels: 2, bits_per_sample: 16 });

        buf.reset();

        assert!(buf.get_audio_params().is_none());
        assert!(buf.get_chunks_after(0).is_empty());

        buf.push_chunk(&make_chunk(5000));
        assert_eq!(buf.get_chunks_after(0)[0].sequence, 1);
    }
}
