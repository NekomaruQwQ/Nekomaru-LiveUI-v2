// Per-stream circular audio chunk buffer.
//
// Simpler than the video StreamBuffer — no keyframe gating needed because
// every PCM chunk is independently decodable.  Chunks are pre-serialized on
// push so HTTP responses just concatenate byte arrays.
//
// Pre-serialized payload format (matches what the frontend receives):
//   [u64 LE: timestamp_us][raw PCM s16le bytes]

import type { AudioParams, AudioChunk } from "./audio-protocol";

// ── Types ────────────────────────────────────────────────────────────────────

export interface BufferedChunk {
    /// Monotonically increasing sequence number (starts at 1).
    sequence: number;
    /// Pre-serialized binary payload: [u64 LE timestamp_us][PCM bytes].
    payload: Uint8Array;
}

// ── AudioBuffer ──────────────────────────────────────────────────────────────

export class AudioBuffer {
    /// Fixed-capacity circular array.
    private chunks: (BufferedChunk | undefined)[];
    /// Next write position (wraps around via modulo).
    private writeIndex = 0;
    /// How many slots are currently occupied (capped at capacity).
    private count = 0;
    /// Monotonically increasing sequence counter.
    private nextSequence = 1;
    /// Cached audio params from the latest AudioParams message.
    private audioParams: AudioParams | null = null;

    constructor(private capacity: number) {
        this.chunks = new Array<BufferedChunk | undefined>(capacity).fill(undefined);
    }

    /// Cache the audio format parameters (sample rate, channels, bit depth).
    setAudioParams(params: AudioParams): void {
        this.audioParams = params;
    }

    /// Return the cached audio params, or null if the capture process hasn't
    /// sent them yet.
    getAudioParams(): AudioParams | null {
        return this.audioParams;
    }

    /// Clear all buffered state — chunks, params, and sequence counter.
    /// Used when the audio process is restarted.
    reset(): void {
        this.chunks.fill(undefined);
        this.writeIndex = 0;
        this.count = 0;
        this.nextSequence = 1;
        this.audioParams = null;
    }

    /// Push a parsed audio chunk into the circular buffer.
    /// Pre-serializes the payload so HTTP responses avoid per-request work.
    pushChunk(chunk: AudioChunk): void {
        const sequence = this.nextSequence++;
        const payload = serializeChunkPayload(chunk);

        this.chunks[this.writeIndex % this.capacity] = { sequence, payload };

        this.writeIndex++;
        if (this.count < this.capacity) this.count++;
    }

    /// Return all buffered chunks with sequence > afterSequence.
    /// If afterSequence exceeds the buffer's max sequence (e.g. after a
    /// process restart reset the counter), returns all buffered chunks.
    getChunksAfter(afterSequence: number): BufferedChunk[] {
        // If the caller's cursor is ahead of our newest chunk, they're from
        // a previous generation — reset by returning everything we have.
        const maxSeq = this.nextSequence - 1;
        if (afterSequence > maxSeq) afterSequence = 0;

        const result: BufferedChunk[] = [];

        const start = this.writeIndex - this.count;
        for (let i = start; i < this.writeIndex; i++) {
            const chunk = this.chunks[((i % this.capacity) + this.capacity) % this.capacity];
            if (!chunk || chunk.sequence <= afterSequence) continue;
            result.push(chunk);
        }

        return result;
    }
}

// ── Serialization ────────────────────────────────────────────────────────────

/// Serialize an AudioChunk into the binary format the frontend expects.
///
/// Layout: [u64 LE: timestamp_us][raw PCM bytes]
function serializeChunkPayload(chunk: AudioChunk): Uint8Array {
    const totalSize = 8 + chunk.pcmData.length;
    const buf = new Uint8Array(totalSize);
    const view = new DataView(buf.buffer);

    view.setBigUint64(0, chunk.timestampUs, true);
    buf.set(chunk.pcmData, 8);

    return buf;
}
