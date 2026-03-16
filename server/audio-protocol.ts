// Incremental binary parser for the live-audio IPC wire protocol.
//
// live-audio.exe writes length-prefixed binary messages to stdout.
// Same envelope format as live-capture: [u8 type][u32 LE len][payload].
//
// Audio message types use the 0x1x range to avoid collision with video:
//   0x10 = AudioParams (sample rate, channels, bit depth)
//   0x11 = AudioFrame  (timestamp + raw PCM s16le)
//   0xFF = Error        (UTF-8 string)

import { createStreamLogger } from "./log";

// ── Types ────────────────────────────────────────────────────────────────────

/// Audio format parameters sent once at capture start.
export interface AudioParams {
    sampleRate: number;
    channels: number;
    bitsPerSample: number;
}

/// One chunk of raw PCM audio with a wall-clock timestamp.
export interface AudioChunk {
    /// Timestamp in microseconds since Unix epoch (same clock as video frames).
    timestampUs: bigint;
    /// Raw interleaved PCM samples (s16le).
    pcmData: Uint8Array;
}

/// A parsed audio IPC message.
export type AudioIpcMessage =
    | { type: "audio_params"; params: AudioParams }
    | { type: "audio_frame"; chunk: AudioChunk }
    | { type: "error"; message: string };

// ── Message type discriminants ───────────────────────────────────────────────

const MSG_AUDIO_PARAMS = 0x10;
const MSG_AUDIO_FRAME  = 0x11;
const MSG_ERROR        = 0xFF;

/// Minimum header size: type(1) + payload_length(4).
const HEADER_SIZE = 5;

// ── Parser ───────────────────────────────────────────────────────────────────

/// Push-based incremental parser for the audio IPC protocol.
///
/// Call `feed(chunk)` each time new data arrives from stdout.
/// Complete messages are emitted via the callback passed to the constructor.
export class AudioProtocolParser {
    /// Internal accumulation buffer.
    private buffer = new Uint8Array(0);
    private callback: (msg: AudioIpcMessage) => void;
    private streamId: string;

    constructor(streamId: string, callback: (msg: AudioIpcMessage) => void) {
        this.streamId = streamId;
        this.callback = callback;
    }

    /// Append new data from stdout and parse all complete messages.
    feed(chunk: Uint8Array): void {
        this.buffer = concatUint8(this.buffer, chunk);

        // Greedy parse loop: consume as many complete messages as possible.
        while (true) {
            if (this.buffer.length < HEADER_SIZE) break;

            const view = new DataView(
                this.buffer.buffer,
                this.buffer.byteOffset,
                this.buffer.byteLength);

            const payloadLength = view.getUint32(1, /* littleEndian */ true);
            const totalLength = HEADER_SIZE + payloadLength;

            if (this.buffer.length < totalLength) break;

            const messageType = this.buffer[0]!;
            const payload = this.buffer.slice(HEADER_SIZE, totalLength);

            // Advance past the consumed message.
            this.buffer = this.buffer.subarray(totalLength);

            const msg = parsePayload(this.streamId, messageType, payload);
            if (msg) this.callback(msg);
        }
    }
}

// ── Payload parsers ──────────────────────────────────────────────────────────

function parsePayload(streamId: string, type: number, payload: Uint8Array): AudioIpcMessage | null {
    switch (type) {
        case MSG_AUDIO_PARAMS: return parseAudioParams(payload);
        case MSG_AUDIO_FRAME:  return parseAudioFrame(payload);
        case MSG_ERROR:        return parseError(payload);
        default:
            createStreamLogger(streamId, "server::audio_protocol")
                .error(`unknown message type 0x${type.toString(16)}`);
            return null;
    }
}

/// Parse an AudioParams payload.
///
/// Wire layout: [u32 LE: sample_rate][u8: channels][u8: bits_per_sample]
function parseAudioParams(data: Uint8Array): AudioIpcMessage {
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    const sampleRate = view.getUint32(0, true);
    const channels = data[4]!;
    const bitsPerSample = data[5]!;
    return { type: "audio_params", params: { sampleRate, channels, bitsPerSample } };
}

/// Parse an AudioFrame payload.
///
/// Wire layout: [u64 LE: timestamp_us][raw PCM bytes]
function parseAudioFrame(data: Uint8Array): AudioIpcMessage {
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    const timestampUs = view.getBigUint64(0, true);
    const pcmData = data.slice(8);
    return { type: "audio_frame", chunk: { timestampUs, pcmData } };
}

/// Parse an Error payload (raw UTF-8).
function parseError(data: Uint8Array): AudioIpcMessage {
    const message = new TextDecoder().decode(data);
    return { type: "error", message };
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Concatenate two Uint8Arrays into a new one.
function concatUint8(a: Uint8Array, b: Uint8Array): Uint8Array {
    if (a.length === 0) return b;
    const result = new Uint8Array(a.length + b.length);
    result.set(a, 0);
    result.set(b, a.length);
    return result;
}
