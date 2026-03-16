// Audio streaming component.
//
// Invisible component that polls the server for PCM audio chunks and plays
// them through an AudioWorklet.  Mounts at the app root — audio is global
// (not per-stream).
//
// No A/V sync — both audio and video use wall-clock timestamps from the
// same machine, and the ~20ms latency difference (audio has no encoding
// step) is imperceptible.  Chunks are posted to the worklet immediately.
//
// Lifecycle:
//   1. Create AudioContext at the device's native sample rate
//   2. Load the PCM worklet module
//   3. Fetch /api/v1/audio/init (retry on 503 until params arrive)
//   4. Poll /api/v1/audio/chunks?after=N with adaptive timing
//   5. Post all received chunks to the worklet immediately
//   6. Handle browser autoplay policy via user interaction resume

import { useEffect } from "react";

/// Normal poll interval when chunks were received (ms).
const POLL_INTERVAL_MS = 16;

/// Shorter poll interval when the server had no new chunks — retry quickly
/// so we don't waste a full 16ms when data arrives between polls.
const POLL_FAST_MS = 4;

/// How long to wait before retrying /init when it returns 503 (ms).
const INIT_RETRY_MS = 500;

// ── Component ────────────────────────────────────────────────────────────────

/// Invisible component that streams audio from the server.
/// Renders nothing — audio output goes to AudioContext.destination.
export function AudioStream() {
    useEffect(() => {
        const abort = new AbortController();
        startAudioLoop(abort.signal);
        return () => abort.abort();
    }, []);

    return null;
}

// ── Audio loop ───────────────────────────────────────────────────────────────

interface AudioInitParams {
    sampleRate: number;
    channels: number;
    bitsPerSample: number;
}

async function startAudioLoop(signal: AbortSignal): Promise<void> {
    // Step 1: Fetch audio params (retry until ready).
    const params = await fetchInit(signal);
    if (!params || signal.aborted) return;

    console.log("AudioStream: params %dHz %dch %d-bit",
        params.sampleRate, params.channels, params.bitsPerSample);

    // Step 2: Create AudioContext + worklet.
    const ctx = new AudioContext({ sampleRate: params.sampleRate });

    // Handle browser autoplay policy — resume on first user interaction.
    if (ctx.state === "suspended") {
        const resume = () => {
            ctx.resume();
            document.removeEventListener("click", resume);
            document.removeEventListener("keydown", resume);
        };
        document.addEventListener("click", resume, { once: true });
        document.addEventListener("keydown", resume, { once: true });
    }

    try {
        // Load the worklet module.  Vite resolves `new URL(..., import.meta.url)`
        // to the correct asset path in both dev and production.
        // IMPORTANT: worklet.ts must remain self-contained (no imports) —
        // AudioWorklet scripts run outside the module system.
        await ctx.audioWorklet.addModule(new URL("./worklet.ts", import.meta.url));
    } catch (e) {
        console.error("AudioStream: Failed to load worklet module:", e);
        return;
    }

    const workletNode = new AudioWorkletNode(ctx, "pcm-worklet-processor", {
        outputChannelCount: [params.channels],
    });
    workletNode.connect(ctx.destination);

    // Step 3: Poll for audio chunks and post them to the worklet immediately.
    let lastSequence = 0;

    while (!signal.aborted) {
        try {
            const res = await fetch(
                `/api/v1/audio/chunks?after=${lastSequence}`,
                { signal });

            if (!res.ok) {
                await sleep(100);
                continue;
            }

            const buf = new Uint8Array(await res.arrayBuffer());
            const chunks = parseBinaryChunkResponse(buf);

            for (const chunk of chunks) {
                lastSequence = Math.max(lastSequence, chunk.sequence);
                postChunkToWorklet(workletNode, chunk, params.channels);
            }

            // Adaptive timing: retry quickly when server had nothing new,
            // use normal interval when chunks arrived.
            await sleep(chunks.length > 0 ? POLL_INTERVAL_MS : POLL_FAST_MS);
        } catch (e) {
            if (signal.aborted) break;
            console.error("AudioStream: poll error:", e);
            await sleep(1000);
        }
    }

    // Cleanup.
    workletNode.disconnect();
    await ctx.close();
    console.log("AudioStream: stopped");
}

// ── Init fetch ───────────────────────────────────────────────────────────────

/// Fetch audio params from /api/v1/audio/init, retrying on 503.
async function fetchInit(signal: AbortSignal): Promise<AudioInitParams | null> {
    while (!signal.aborted) {
        try {
            const res = await fetch("/api/v1/audio/init", { signal });
            if (res.status === 404) return null;  // Audio disabled on server.
            if (res.status === 503) {
                await sleep(INIT_RETRY_MS);
                continue;
            }
            if (!res.ok) {
                console.error("AudioStream: /init returned %d", res.status);
                await sleep(INIT_RETRY_MS);
                continue;
            }
            return await res.json() as AudioInitParams;
        } catch (e) {
            if (signal.aborted) return null;
            console.error("AudioStream: /init fetch error:", e);
            await sleep(INIT_RETRY_MS);
        }
    }
    return null;
}

// ── Worklet helpers ──────────────────────────────────────────────────────────

/// Convert a parsed PCM chunk to Int16Array and post to the worklet.
function postChunkToWorklet(
    node: AudioWorkletNode, chunk: ParsedChunk, channels: number,
): void {
    const samples = new Int16Array(
        chunk.pcmData.buffer, chunk.pcmData.byteOffset, chunk.pcmData.byteLength / 2);
    node.port.postMessage({ type: "pcm", samples, channels });
}

// ── Binary chunk response parser ─────────────────────────────────────────────

interface ParsedChunk {
    sequence: number;
    timestampUs: bigint;
    pcmData: Uint8Array;
}

/// Parse the binary blob returned by GET /api/v1/audio/chunks.
///
/// Layout (all little-endian):
///   [u32: num_chunks]
///   per chunk: [u32: sequence][u32: payload_length][payload bytes]
///
/// Payload per chunk: [u64 LE: timestamp_us][delta-encoded s16le PCM bytes]
///
/// Delta decoding: first sample is literal, each subsequent sample is
/// accumulated (prefix sum) to reconstruct the original PCM.
function parseBinaryChunkResponse(buf: Uint8Array): ParsedChunk[] {
    const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    let pos = 0;

    const numChunks = view.getUint32(pos, true); pos += 4;

    const chunks: ParsedChunk[] = [];
    for (let i = 0; i < numChunks; i++) {
        const sequence = view.getUint32(pos, true);  pos += 4;
        const payloadLen = view.getUint32(pos, true); pos += 4;

        // First 8 bytes of payload = timestamp_us (u64 LE).
        const timestampUs = view.getBigUint64(pos, true);
        // slice() creates an owned copy — safe to mutate in-place.
        const pcmData = buf.slice(pos + 8, pos + payloadLen);
        pos += payloadLen;

        deltaDecodePcm(pcmData);
        chunks.push({ sequence, timestampUs, pcmData });
    }

    return chunks;
}

/// Decode delta-encoded s16le PCM samples in-place.
/// First sample is literal; each subsequent sample accumulates the delta.
function deltaDecodePcm(pcmData: Uint8Array): void {
    const view = new DataView(pcmData.buffer, pcmData.byteOffset, pcmData.byteLength);
    const sampleCount = pcmData.byteLength >>> 1;
    for (let i = 1; i < sampleCount; i++) {
        const off = i * 2;
        const prev = view.getInt16(off - 2, true);
        const delta = view.getInt16(off, true);
        view.setInt16(off, (prev + delta) | 0, true);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}
