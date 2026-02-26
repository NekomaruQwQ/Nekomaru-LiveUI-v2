// Per-stream circular frame buffer with SPS/PPS cache.
//
// Each active capture stream gets one StreamBuffer.  Frames are stored in a
// fixed-capacity circular array with monotonic sequence numbers.  Multiple
// HTTP clients can read from the same buffer without draining it (unlike the
// old Rust StreamManager which used a pop-based ArrayQueue).
//
// Frames are pre-serialized on push into the binary format the frontend's
// parseStreamFrame() expects:
//   [u64 LE: timestamp_us]
//   [u32 LE: num_nal_units]
//   for each NAL: [u8: nal_type][u32 LE: data_length][data bytes]
//
// This matches the old Rust serialize_stream_frame() in src/app.rs:436-460.
// Notably, the is_keyframe byte from the IPC wire format is NOT included —
// the frontend infers keyframe status from NAL unit types.

import type { CodecParams, FrameMessage } from "./protocol";

// ── Types ────────────────────────────────────────────────────────────────────

export interface BufferedFrame {
    /// Monotonically increasing sequence number (starts at 1).
    sequence: number;
    /// Whether this frame contains an IDR NAL unit.
    isKeyframe: boolean;
    /// Pre-serialized binary payload in the format parseStreamFrame() expects.
    payload: Uint8Array;
}

// ── StreamBuffer ─────────────────────────────────────────────────────────────

export class StreamBuffer {
    /// Fixed-capacity circular array.
    private frames: (BufferedFrame | undefined)[];
    /// Next write position (wraps around via modulo).
    private writeIndex = 0;
    /// How many slots are currently occupied (capped at capacity).
    private count = 0;
    /// Monotonically increasing sequence counter.
    private nextSequence = 1;
    /// Cached codec params from the latest CodecParams message.
    private codecParams: CodecParams | null = null;

    constructor(private capacity: number) {
        this.frames = new Array<BufferedFrame | undefined>(capacity).fill(undefined);
    }

    /// Cache the latest codec parameters (SPS/PPS/resolution).
    /// Called when a CodecParams IPC message arrives.
    setCodecParams(params: CodecParams): void {
        this.codecParams = params;
    }

    /// Return the cached codec params, or null if the encoder hasn't produced
    /// its first IDR frame yet.
    getCodecParams(): CodecParams | null {
        return this.codecParams;
    }

    /// Clear all buffered state — frames, codec params, and sequence counter.
    /// Used by replaceStream() when the underlying capture process is swapped
    /// so stale frames from the old process are never served.
    reset(): void {
        this.frames.fill(undefined);
        this.writeIndex = 0;
        this.count = 0;
        this.nextSequence = 1;
        this.codecParams = null;
    }

    /// Push a parsed frame into the circular buffer.
    ///
    /// Assigns the next sequence number and pre-serializes the frame payload
    /// so HTTP responses don't need to re-serialize on every request.
    pushFrame(frame: FrameMessage): void {
        const sequence = this.nextSequence++;
        const payload = serializeFramePayload(frame);

        this.frames[this.writeIndex % this.capacity] = {
            sequence,
            isKeyframe: frame.isKeyframe,
            payload,
        };

        this.writeIndex++;
        if (this.count < this.capacity) this.count++;
    }

    /// Return all buffered frames with sequence > afterSequence.
    ///
    /// When afterSequence is 0 (first request from a new client), non-keyframes
    /// are skipped until the first keyframe is found — the WebCodecs decoder
    /// needs an IDR frame to initialize.
    getFramesAfter(afterSequence: number): BufferedFrame[] {
        const result: BufferedFrame[] = [];

        // Scan the circular buffer from oldest to newest.
        const start = this.writeIndex - this.count;
        let needKeyframe = afterSequence === 0;

        for (let i = start; i < this.writeIndex; i++) {
            const frame = this.frames[((i % this.capacity) + this.capacity) % this.capacity];
            if (!frame || frame.sequence <= afterSequence) continue;

            // First request: skip non-keyframes until we find an IDR.
            if (needKeyframe) {
                if (!frame.isKeyframe) continue;
                needKeyframe = false;
            }

            result.push(frame);
        }

        return result;
    }
}

// ── Serialization ────────────────────────────────────────────────────────────

/// Serialize a FrameMessage into the binary format the frontend expects.
///
/// Layout:
///   [u64 LE: timestamp_us]
///   [u32 LE: num_nal_units]
///   for each NAL: [u8: nal_type][u32 LE: data_length][data bytes]
///
/// This deliberately omits the is_keyframe byte — the IPC wire format includes
/// it, but the HTTP response format (and the frontend parser) does not.
function serializeFramePayload(frame: FrameMessage): Uint8Array {
    // Pre-compute total size.
    let nalDataSize = 0;
    for (const nal of frame.nalUnits) {
        nalDataSize += 1 + 4 + nal.data.length; // type(1) + length(4) + data
    }
    const totalSize = 8 + 4 + nalDataSize; // timestamp(8) + count(4) + nals

    const buf = new Uint8Array(totalSize);
    const view = new DataView(buf.buffer);
    let pos = 0;

    // Timestamp (u64 LE)
    view.setBigUint64(pos, frame.timestampUs, true); pos += 8;

    // Number of NAL units (u32 LE)
    view.setUint32(pos, frame.nalUnits.length, true); pos += 4;

    // Each NAL unit
    for (const nal of frame.nalUnits) {
        buf[pos] = nal.unitType;                            pos += 1;
        view.setUint32(pos, nal.data.length, true);         pos += 4;
        buf.set(nal.data, pos);                             pos += nal.data.length;
    }

    return buf;
}
