#![expect(
    clippy::doc_paragraphs_missing_punctuation,
    reason = "code by Claude Code")]

use crate::encoder::{NALUnit, NALUnitType};

use nkcore::prelude::*;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Codec parameters (SPS/PPS) for H.264 stream
#[derive(Debug, Clone)]
pub struct CodecParams {
    /// Sequence Parameter Set
    pub sps: Vec<u8>,
    /// Picture Parameter Set
    pub pps: Vec<u8>,
    /// Video width
    pub width: u32,
    /// Video height
    pub height: u32,
}

/// A single encoded frame with all its NAL units
#[derive(Debug, Clone)]
pub struct StreamFrame {
    /// Sequence number for this frame
    pub sequence: u64,
    /// All NAL units for this frame
    pub nal_units: Vec<NALUnit>,
    /// Timestamp in microseconds
    pub timestamp_us: u64,
    /// Whether this is a keyframe (contains IDR NAL unit)
    pub is_keyframe: bool,
}

/// Thread-safe stream manager for buffering encoded frames
pub struct StreamManager {
    /// Lock-free circular buffer for frames
    frame_queue: Arc<crossbeam::queue::ArrayQueue<StreamFrame>>,
    /// Cached codec parameters (SPS/PPS)
    codec_params: Arc<Mutex<Option<CodecParams>>>,
    /// Monotonically increasing sequence counter
    sequence_counter: AtomicU64,
    /// Start time for wall-clock timestamps
    start_time: Instant,
}

impl StreamManager {
    /// Create a new stream manager with specified buffer capacity.
    ///
    /// # Arguments
    /// * `capacity` - Maximum number of frames to buffer (e.g., 60 for 1 second at 60fps)
    pub fn new(capacity: usize) -> Self {
        Self {
            frame_queue: Arc::new(crossbeam::queue::ArrayQueue::new(capacity)),
            codec_params: Arc::new(Mutex::new(None)),
            sequence_counter: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }

    /// Push a new encoded frame to the stream.
    ///
    /// Called by the encoder after encoding each frame.
    /// If the queue is full, drops the oldest frame (live streaming behavior).
    ///
    /// # Arguments
    /// * `nal_units` - Encoded NAL units for this frame
    #[expect(clippy::similar_names, reason = "terminology")]
    pub fn push_frame(&self, nal_units: Vec<NALUnit>) -> anyhow::Result<()> {
        if nal_units.is_empty() {
            // Empty frame (encoder buffering) - skip
            return Ok(());
        }

        // Cache SPS/PPS for new clients
        let has_sps = nal_units.iter().any(|u| u.unit_type == NALUnitType::SPS);
        let has_pps = nal_units.iter().any(|u| u.unit_type == NALUnitType::PPS);

        if has_sps && has_pps {
            // Extract SPS and PPS
            let sps = nal_units
                .iter()
                .find(|u| u.unit_type == NALUnitType::SPS)
                .ok_or_else(|| anyhow::anyhow!("SPS not found despite has_sps=true"))?;
            let pps = nal_units
                .iter()
                .find(|u| u.unit_type == NALUnitType::PPS)
                .ok_or_else(|| anyhow::anyhow!("PPS not found despite has_pps=true"))?;

            // TODO: Parse actual dimensions from SPS instead of hardcoding
            let params = CodecParams {
                sps: sps.data.clone(),
                pps: pps.data.clone(),
                width: 1920,
                height: 1200,
            };

            match self.codec_params.lock() {
                Ok(mut guard) => *guard = Some(params),
                Err(e) => log::warn!("Failed to cache codec params: {e}"),
            }
        }

        // Check if this is a keyframe
        let is_keyframe = nal_units.iter().any(|u| u.unit_type == NALUnitType::IDR);

        // Get next sequence number
        let sequence = self.sequence_counter.fetch_add(1, Ordering::SeqCst);

        // Calculate wall-clock timestamp in microseconds since stream start
        let timestamp_us = self.start_time.elapsed().as_micros() as u64;

        let frame = StreamFrame {
            sequence,
            nal_units,
            timestamp_us,
            is_keyframe,
        };

        // If queue is full, drop oldest frame (live streaming behavior)
        if self.frame_queue.is_full() {
            let _ = self.frame_queue.pop();
            log::warn!("Stream queue full, dropping frame {}", sequence.saturating_sub(60));
        }

        // Push new frame
        self.frame_queue
            .push(frame)
            .map_err(|_frame| anyhow::anyhow!("Failed to push frame to queue"))?;

        Ok(())
    }

    /// Get the next frame after the specified sequence number.
    ///
    /// Blocks until a frame is available or timeout is reached.
    /// Used by the protocol handler to implement long-polling.
    ///
    /// # Arguments
    /// * `after_sequence` - Only return frames with sequence > this value
    ///   - If `after_sequence == 0`, waits for the next keyframe (required for decoder initialization)
    /// * `timeout` - Maximum time to wait for a frame
    ///
    /// # Single-Client Design
    /// This implementation assumes a single client. Old frames and non-keyframes (when required)
    /// are discarded rather than preserved.
    pub fn get_frames(&self, after_sequence: u64) -> Vec<StreamFrame> {
        let mut frames = Vec::new();

        // Drain old/invalid frames until we find a valid one
        while let Some(frame) = self.frame_queue.pop() {
            if frame.sequence <= after_sequence {
                // Old frame, discard
                log::trace!(
                    "Discarding old frame seq={}, keyframe={} (after_seq={})",
                    frame.sequence, frame.is_keyframe, after_sequence);
                continue;
            }

            if after_sequence == 0 && !frame.is_keyframe {
                // First frame must be a keyframe, discard non-keyframe
                log::trace!(
                    "Discarding non-keyframe seq={} (after_seq=0, require_keyframe=true)",
                    frame.sequence);
                continue;
            }

            log::debug!(
                "Returning frame seq={}, keyframe={}, after_seq={}",
                frame.sequence, frame.is_keyframe, after_sequence);
            frames.push(frame);
        }

        frames
    }

    /// Get cached codec parameters (SPS/PPS).
    ///
    /// Used by protocol handler to send initialization data to client.
    pub fn get_codec_params(&self) -> Option<CodecParams> {
        match self.codec_params.lock() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                log::error!("Failed to get codec params: {e}");
                None
            }
        }
    }
}
