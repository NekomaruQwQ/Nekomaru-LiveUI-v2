import { useEffect, useRef } from "react";

import { DEBUG } from "../../debug";
import { api } from "../api";
import { ChromaKeyRenderer, parseHexColor } from "./chroma-key";
import { H264Decoder, parseStreamFrame } from "./decoder";

/**
 * Video renderer for a well-known stream ID ("main" or "youtube-music").
 *
 * The stream loop owns the full decoder lifecycle: it creates a decoder,
 * fetches frames, and — when it detects a generation change — closes the
 * old decoder and creates a new one.  This lets the server replace the
 * underlying capture process (e.g. on window switch) without the component
 * remounting.
 *
 * 404 responses are treated as retriable (the server may create the stream
 * shortly), so the component can be rendered before the stream exists.
 *
 * When `chromaKey` is set (e.g. "#212121"), a WebGL2 fragment shader replaces
 * pixels matching that color with transparency.  The entire pipeline stays on
 * the GPU — no CPU readback.
 */
export function StreamRenderer({ streamId, chromaKey, pollMs = 16 }: {
    streamId: string;
    chromaKey?: string;
    /// Frame poll interval in milliseconds.  Defaults to 16 (~60 fps).
    /// Set to 1000 for low-fps streams like YouTube Music (1 fps).
    pollMs?: number;
}) {
    const canvasRef = useRef<HTMLCanvasElement>(null);

    useEffect(() => {
        console.log("StreamRenderer: Component mounted");

        const canvas = canvasRef.current;
        if (!canvas) {
            console.error("StreamRenderer: Canvas ref is null!");
            return;
        }

        // ── Build the frame renderer ─────────────────────────────────────
        // When chroma-key is active, use a WebGL2 shader that keys out the
        // target color.  Otherwise, use a plain 2D canvas drawImage path.
        let onFrame: (frame: VideoFrame) => void;
        let cleanup: (() => void) | undefined;

        if (chromaKey) {
            const renderer = new ChromaKeyRenderer(canvas, parseHexColor(chromaKey));
            onFrame = (frame) => renderer.render(frame);
            cleanup = () => renderer.dispose();
            console.log("StreamRenderer: Using WebGL chroma-key renderer (key=%s)", chromaKey);
        } else {
            const ctx = canvas.getContext("2d");
            if (!ctx) {
                console.error("StreamRenderer: Failed to get 2D context");
                return;
            }
            onFrame = (frame) => renderFrame(canvas, ctx, frame);
            console.log("StreamRenderer: Using 2D canvas renderer");
        }

        console.log("StreamRenderer: Canvas ready: %dx%d", canvas.width, canvas.height);

        const abortController = new AbortController();
        startStreamLoop(streamId, onFrame, abortController.signal, pollMs);

        return () => {
            console.log("StreamRenderer: Component unmounting, aborting stream loop");
            abortController.abort();
            cleanup?.();
        };
    }, [streamId, chromaKey, pollMs]);

    return (
        <canvas
            ref={canvasRef}
            className={`w-full object-contain ${chromaKey ? "" : "bg-[#1e1f22]"}`}
        />
    );
}

let lastFrameTime = 0;

/**
 * Render a decoded video frame to canvas.
 */
function renderFrame(canvas: HTMLCanvasElement, ctx: CanvasRenderingContext2D, frame: VideoFrame) {
    // Resize canvas if needed.
    if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
        canvas.width = frame.displayWidth;
        canvas.height = frame.displayHeight;
        console.log(
            "StreamRenderer: Canvas resized to %dx%d",
            frame.displayWidth,
            frame.displayHeight);
    }

    if (DEBUG.debugStreamRenderer) {
        console.log("StreamRenderer: Rendering frame to canvas - timestamp: %d μs", frame.timestamp);
    }
    ctx.drawImage(frame, 0, 0);

    // CRITICAL: Close frame to release GPU memory.
    frame.close();

    if (DEBUG.debugStreamRenderer) {
        console.log("StreamRenderer: Frame closed (GPU memory released)");
    }

    const now = performance.now();
    if (lastFrameTime > 0) {
        const delta = now - lastFrameTime;
        if (DEBUG.debugStreamRenderer) {
            console.log("StreamRenderer: Frame interval: %d ms", delta);
        }
    }
    lastFrameTime = now;
}

/**
 * Stream loop that owns decoder lifecycle, fetches frames, and handles
 * generation changes (decoder reinitialization) and 404s (stream not yet
 * created).
 *
 * Runs until the AbortSignal fires (component unmount / streamId change).
 *
 * @param onFrame  Callback that renders a decoded VideoFrame.  Responsible
 *                 for closing the frame after use.
 */
async function startStreamLoop(
    streamId: string,
    onFrame: (frame: VideoFrame) => void,
    signal: AbortSignal,
    pollMs: number,
): Promise<void> {
    console.log("StreamLoop: Starting stream loop");

    // Create the initial decoder.  fetchInit inside init() retries on 503
    // and 404, so this blocks until the stream's encoder has produced its
    // first IDR frame.
    let decoder = new H264Decoder(streamId, onFrame);
    try {
        await decoder.init();
    } catch (e) {
        console.error("StreamLoop: Failed to initialize decoder:", e);
        return;
    }
    if (signal.aborted) { decoder.close(); return; }

    let lastSequence = 0;
    let currentGeneration: number | null = null;
    let consecutiveErrors = 0;
    const MAX_CONSECUTIVE_ERRORS = 30;

    while (!signal.aborted) {
        try {
            if (DEBUG.debugStreamRenderer) {
                console.log("StreamLoop: Fetching frame after sequence %d", lastSequence);
            }
            const res = await api[":id"].frames.$get({
                param: { id: streamId },
                query: { after: String(lastSequence) },
            });

            // 404 = stream doesn't exist yet (server may create it soon).
            // Sleep and retry rather than counting towards fatal errors.
            if (res.status === 404) {
                await sleep(1000);
                continue;
            }

            if (!res.ok) {
                console.error("StreamLoop: Stream request failed: %d %s", res.status, res.statusText);
                await sleep(100);
                consecutiveErrors++;
                if (consecutiveErrors >= MAX_CONSECUTIVE_ERRORS) {
                    console.error("StreamLoop: Too many consecutive errors, stopping stream");
                    break;
                }
                continue;
            }

            consecutiveErrors = 0;

            // Parse the binary frame response (see server/api.ts for layout).
            const { generation, frames } = parseBinaryFrameResponse(
                new Uint8Array(await res.arrayBuffer()));

            // ── Generation change: reinitialize decoder ──────────────────
            if (currentGeneration !== null && generation !== currentGeneration) {
                console.log("StreamLoop: Generation changed %d → %d, reinitializing decoder",
                    currentGeneration, generation);
                decoder.close();
                decoder = new H264Decoder(streamId, onFrame);
                await decoder.init();
                lastSequence = 0;
            }
            currentGeneration = generation;

            for (const { sequence, payload } of frames) {
                lastSequence = Math.max(lastSequence, sequence);
                const frame = parseStreamFrame(payload);
                decoder.decodeFrame(frame);
            }
            await sleep(pollMs);

        } catch (e) {
            // AbortError is expected on cleanup — don't log or count it.
            if (signal.aborted) break;
            console.error("StreamLoop: Stream error:", e);
            consecutiveErrors++;
            if (consecutiveErrors >= MAX_CONSECUTIVE_ERRORS) {
                console.error("StreamLoop: Too many consecutive errors, stopping stream");
                break;
            }
            await sleep(1000);
        }
    }

    decoder.close();
    console.log("StreamLoop: Stream loop ended");
}

function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}

// ── Binary frame response parser ─────────────────────────────────────────

interface BinaryFrameResponse {
    generation: number;
    frames: { sequence: number; payload: Uint8Array }[];
}

/// Parse the binary blob returned by GET /:id/frames.
///
/// Layout (all little-endian):
///   [u32: generation][u32: num_frames]
///   per frame: [u32: sequence][u32: payload_length][payload bytes]
function parseBinaryFrameResponse(buf: Uint8Array): BinaryFrameResponse {
    const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    let pos = 0;

    const generation = view.getUint32(pos, true); pos += 4;
    const numFrames = view.getUint32(pos, true);  pos += 4;

    const frames: BinaryFrameResponse["frames"] = [];
    for (let i = 0; i < numFrames; i++) {
        const sequence = view.getUint32(pos, true);      pos += 4;
        const payloadLen = view.getUint32(pos, true);     pos += 4;
        // subarray: zero-copy view into the same ArrayBuffer.
        const payload = buf.subarray(pos, pos + payloadLen); pos += payloadLen;
        frames.push({ sequence, payload });
    }

    return { generation, frames };
}
