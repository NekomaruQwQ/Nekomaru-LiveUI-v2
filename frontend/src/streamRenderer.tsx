import { css } from '@emotion/css';
import { useEffect, useRef } from 'react';

import { DEBUG } from '../debug';
import { api } from './api';
import { H264Decoder, parseStreamFrame } from './streamDecoder';

/**
 * Video renderer component that decodes and displays H.264 stream
 */
export function StreamRenderer({ streamId }: { streamId: string }) {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const decoderRef = useRef<H264Decoder | null>(null);

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

        const decoder = new H264Decoder(streamId, (frame: VideoFrame) => {
            renderFrame(canvas, ctx, frame);
        });

        decoderRef.current = decoder;

        // Initialize and start stream loop
        console.log('StreamRenderer: Initializing decoder...');

        decoder
            .init()
            .then(() => {
                console.log('StreamRenderer: Decoder initialized, starting stream loop');
                startStreamLoop(decoder, streamId);
            })
            .catch((e) => {
                console.error('StreamRenderer: Failed to initialize decoder:', e);
            });

        return () => {
            console.log('StreamRenderer: Component unmounting, closing decoder');
            decoder.close();
        };
    }, [streamId]);

    return (
        <canvas
            ref={canvasRef}
            className={css({
                width: '100%',
                backgroundColor: '#292929',
                objectFit: 'contain',
                borderRadius: 12,
            })}
        />
    );
}

let lastFrameTime = 0;

/**
 * Render a decoded video frame to canvas
 */
function renderFrame(canvas: HTMLCanvasElement, ctx: CanvasRenderingContext2D, frame: VideoFrame) {
    // Resize canvas if needed
    if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
        canvas.width = frame.displayWidth;
        canvas.height = frame.displayHeight;
        console.log(
            'StreamRenderer: Canvas resized to %dx%d',
            frame.displayWidth,
            frame.displayHeight);
    }

    // Draw frame
    if (DEBUG.debugStreamRenderer) {
        console.log('StreamRenderer: Rendering frame to canvas - timestamp: %d μs', frame.timestamp);
    }
    ctx.drawImage(frame, 0, 0);

    // CRITICAL: Close frame to release GPU memory
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
 * Stream loop that fetches and decodes frames
 */
async function startStreamLoop(decoder: H264Decoder, streamId: string) {
    console.log('StreamLoop: Starting stream loop');

    let lastSequence = 0;
    let consecutiveErrors = 0;
    const MAX_CONSECUTIVE_ERRORS = 10;
    let frameCount = 0;

    while (true) {
        try {
            if (DEBUG.debugStreamRenderer) {
                console.log('StreamLoop: Fetching frame after sequence %d', lastSequence);
            }
            const res = await api[":id"].frames.$get({
                param: { id: streamId },
                query: { after: String(lastSequence) },
            });

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
            const data = await res.json() as { frames: { sequence: number; data: string }[] };
            for (const frameInfo of data.frames) {
                const sequence = frameInfo.sequence;
                lastSequence = Math.max(lastSequence, sequence);
                const frameDat = Uint8Array.fromBase64(frameInfo.data);
                const frame = parseStreamFrame(frameDat);
                decoder.decodeFrame(frame);
                frameCount++;
            }
            await sleep(16);

        } catch (e) {
            console.error('StreamLoop: Stream error:', e);
            consecutiveErrors++;
            if (consecutiveErrors >= MAX_CONSECUTIVE_ERRORS) {
                console.error('StreamLoop: Too many consecutive errors, stopping stream');
                break;
            }
            await sleep(1000);
        }
    }

    console.log('StreamLoop: Stream loop ended');
}

/**
 * Sleep helper
 */
function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}
