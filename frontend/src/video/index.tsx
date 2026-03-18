import { useEffect, useRef } from "react";

import { DEBUG } from "../../debug";
import { openWebSocket, wsMessages } from "../ws";
import { ChromaKeyRenderer, parseHexColor } from "./chroma-key";
import { H264Decoder } from "./decoder";

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
export function StreamRenderer({ streamId, chromaKey }: {
    streamId: string;
    chromaKey?: string;
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
        startStreamLoop(streamId, onFrame, abortController.signal);

        return () => {
            console.log("StreamRenderer: Component unmounting, aborting stream loop");
            abortController.abort();
            cleanup?.();
        };
    }, [streamId, chromaKey]);

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
 * Stream loop that owns decoder lifecycle, receives frames via WebSocket,
 * and handles generation changes (decoder reinitialization).
 *
 * Runs until the AbortSignal fires (component unmount / streamId change).
 * Reconnects with exponential backoff on disconnect.
 *
 * @param onFrame  Callback that renders a decoded VideoFrame.  Responsible
 *                 for closing the frame after use.
 */
async function startStreamLoop(
    streamId: string,
    onFrame: (frame: VideoFrame) => void,
    signal: AbortSignal,
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

    const INITIAL_DELAY_MS = 100;
    const MAX_DELAY_MS = 5000;
    let delay = INITIAL_DELAY_MS;

    // Outer reconnect loop.
    while (!signal.aborted) {
        try {
            const ws = await openWebSocket(
                `/api/v1/ws/video/${streamId}`, signal);
            if (signal.aborted) break;

            // Send cursor so the server sends a catch-up batch.
            ws.send(JSON.stringify({ after: lastSequence }));
            delay = INITIAL_DELAY_MS; // Reset backoff on successful connect.

            // Inner message loop — process frames until WS closes.
            for await (const data of wsMessages(ws, signal)) {
                const { generation, frames } = parseBinaryFrameResponse(
                    new Uint8Array(data));

                // Generation change: reinitialize decoder.
                if (currentGeneration !== null && generation !== currentGeneration) {
                    console.log("StreamLoop: Generation changed %d → %d, reinitializing decoder",
                        currentGeneration, generation);
                    decoder.close();
                    decoder = new H264Decoder(streamId, onFrame);
                    while (!signal.aborted) {
                        try {
                            await decoder.init();
                            break;
                        } catch (e) {
                            console.warn("StreamLoop: Reinit failed, retrying:", e);
                            await sleep(1000);
                        }
                    }
                    if (signal.aborted) break;
                    lastSequence = 0;
                }
                currentGeneration = generation;

                for (const { sequence, timestamp, isKeyframe, data: frameData } of frames) {
                    lastSequence = Math.max(lastSequence, sequence);
                    decoder.decodeFrame(timestamp, isKeyframe, frameData);
                }
            }
        } catch {
            if (signal.aborted) break;
        }

        // Backoff before reconnecting.
        if (!signal.aborted) {
            console.log("StreamLoop: WS disconnected, reconnecting in %dms", delay);
            await sleep(delay);
            delay = Math.min(delay * 2, MAX_DELAY_MS);
        }
    }

    decoder.close();
    console.log("StreamLoop: Stream loop ended");
}


function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}

// ── Binary frame response parser ─────────────────────────────────────────

interface FrameEntry {
    sequence: number;
    timestamp: number;
    isKeyframe: boolean;
    /// AVCC payload — directly feedable to EncodedVideoChunk.data.
    data: Uint8Array;
}

interface BinaryFrameResponse {
    generation: number;
    frames: FrameEntry[];
}

/// Parse the binary blob returned by GET /:id/frames.
///
/// Layout (all little-endian):
///   [u32: generation][u32: num_frames]
///   per frame:
///     [u32: sequence][u64: timestamp_us][u8: is_keyframe]
///     [u32: avcc_payload_length][avcc bytes]
function parseBinaryFrameResponse(buf: Uint8Array): BinaryFrameResponse {
    const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    let pos = 0;

    const generation = view.getUint32(pos, true); pos += 4;
    const numFrames = view.getUint32(pos, true);  pos += 4;

    const frames: FrameEntry[] = [];
    for (let i = 0; i < numFrames; i++) {
        const sequence = view.getUint32(pos, true);                pos += 4;
        const timestamp = Number(view.getBigUint64(pos, true));    pos += 8;
        const isKeyframe = view.getUint8(pos) !== 0;               pos += 1;
        const dataLen = view.getUint32(pos, true);                 pos += 4;
        // subarray: zero-copy view into the same ArrayBuffer.
        const data = buf.subarray(pos, pos + dataLen);             pos += dataLen;
        frames.push({ sequence, timestamp, isKeyframe, data });
    }

    return { generation, frames };
}
