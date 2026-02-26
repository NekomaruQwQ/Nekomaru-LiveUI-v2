import { useEffect, useRef } from 'react';

import { DEBUG } from '../../debug';
import { api } from '../api';
import { H264Decoder, parseStreamFrame } from './decoder';

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
 */
export function StreamRenderer({ streamId }: { streamId: string }) {
    const canvasRef = useRef<HTMLCanvasElement>(null);

    useEffect(() => {
        console.log('StreamRenderer: Component mounted');

        const canvas = canvasRef.current;
        if (!canvas) {
            console.error('StreamRenderer: Canvas ref is null!');
            return;
        }

        const ctx = canvas.getContext('2d');
        if (!ctx) {
            console.error('StreamRenderer: Failed to get 2D context');
            return;
        }

        console.log('StreamRenderer: Canvas ready: %dx%d', canvas.width, canvas.height);

        const abortController = new AbortController();
        startStreamLoop(streamId, canvas, ctx, abortController.signal);

        return () => {
            console.log('StreamRenderer: Component unmounting, aborting stream loop');
            abortController.abort();
        };
    }, [streamId]);

    return (
        <canvas
            ref={canvasRef}
            className="w-full bg-[#1e1f22] object-contain"
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
            'StreamRenderer: Canvas resized to %dx%d',
            frame.displayWidth,
            frame.displayHeight);
    }

    if (DEBUG.debugStreamRenderer) {
        console.log('StreamRenderer: Rendering frame to canvas - timestamp: %d μs', frame.timestamp);
    }
    ctx.drawImage(frame, 0, 0);

    // CRITICAL: Close frame to release GPU memory.
    frame.close();

    if (DEBUG.debugStreamRenderer) {
        console.log('StreamRenderer: Frame closed (GPU memory released)');
    }

    const now = performance.now();
    if (lastFrameTime > 0) {
        const delta = now - lastFrameTime;
        if (DEBUG.debugStreamRenderer) {
            console.log('StreamRenderer: Frame interval: %d ms', delta);
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
 */
async function startStreamLoop(
    streamId: string,
    canvas: HTMLCanvasElement,
    ctx: CanvasRenderingContext2D,
    signal: AbortSignal,
): Promise<void> {
    console.log('StreamLoop: Starting stream loop');

    // Create the initial decoder.  fetchInit inside init() retries on 503
    // and 404, so this blocks until the stream's encoder has produced its
    // first IDR frame.
    let decoder = new H264Decoder(streamId, (frame) => renderFrame(canvas, ctx, frame));
    try {
        await decoder.init();
    } catch (e) {
        console.error('StreamLoop: Failed to initialize decoder:', e);
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
                console.log('StreamLoop: Fetching frame after sequence %d', lastSequence);
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
                console.error('StreamLoop: Stream request failed: %d %s', res.status, res.statusText);
                await sleep(100);
                consecutiveErrors++;
                if (consecutiveErrors >= MAX_CONSECUTIVE_ERRORS) {
                    console.error('StreamLoop: Too many consecutive errors, stopping stream');
                    break;
                }
                continue;
            }

            consecutiveErrors = 0;

            // Narrow from the response union to the success case (confirmed by res.ok).
            const data = await res.json() as {
                generation: number;
                frames: { sequence: number; data: string }[];
            };

            // ── Generation change: reinitialize decoder ──────────────────
            if (currentGeneration !== null && data.generation !== currentGeneration) {
                console.log('StreamLoop: Generation changed %d → %d, reinitializing decoder',
                    currentGeneration, data.generation);
                decoder.close();
                decoder = new H264Decoder(streamId, (frame) => renderFrame(canvas, ctx, frame));
                await decoder.init();
                lastSequence = 0;
            }
            currentGeneration = data.generation;

            for (const frameInfo of data.frames) {
                lastSequence = Math.max(lastSequence, frameInfo.sequence);
                const frameDat = Uint8Array.fromBase64(frameInfo.data);
                const frame = parseStreamFrame(frameDat);
                decoder.decodeFrame(frame);
            }
            await sleep(16);

        } catch (e) {
            // AbortError is expected on cleanup — don't log or count it.
            if (signal.aborted) break;
            console.error('StreamLoop: Stream error:', e);
            consecutiveErrors++;
            if (consecutiveErrors >= MAX_CONSECUTIVE_ERRORS) {
                console.error('StreamLoop: Too many consecutive errors, stopping stream');
                break;
            }
            await sleep(1000);
        }
    }

    decoder.close();
    console.log('StreamLoop: Stream loop ended');
}

function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}
