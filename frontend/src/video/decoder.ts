import { DEBUG } from "../../debug";

// ── Init with retry ─────────────────────────────────────────────────────

/// Codec init params returned by the server (pre-built for WebCodecs).
interface CodecParams {
    codec: string;
    width: number;
    height: number;
    /// Base64-encoded AVCDecoderConfigurationRecord (avcC).
    description: string;
}

/// Fetch codec initialization params, retrying on 503 and 404.
///
/// 503: the capture process is initializing (encoder hasn't produced its
///      first IDR frame yet).
/// 404: the stream doesn't exist yet — the server may create it shortly
///      (e.g. the auto-selector hasn't picked a window yet).
///
/// Retries with exponential backoff (capped at 2s) for up to 30 attempts,
/// giving the server plenty of time to create and initialize the stream.
async function fetchInit(streamId: string): Promise<CodecParams> {
    const maxRetries = 30;
    const baseDelayMs = 250;

    for (let attempt = 0; attempt < maxRetries; attempt++) {
        const res = await fetch(`/api/v1/streams/${streamId}/init`);

        if (res.ok) {
            return await res.json() as CodecParams;
        }

        // Retriable: stream not yet created (404) or encoder still starting (503).
        if (res.status === 404 || res.status === 503) {
            const delay = baseDelayMs * Math.min(2 ** attempt, 8);
            await new Promise((r) => setTimeout(r, delay));
            continue;
        }

        throw new Error(`Failed to fetch codec params: ${res.status} ${res.statusText}`);
    }

    throw new Error("Timed out waiting for codec params");
}

// ── H.264 Decoder ───────────────────────────────────────────────────────

/**
 * H.264 decoder using WebCodecs API.
 *
 * The server provides pre-built codec configuration (avcC descriptor +
 * codec string) and AVCC-formatted frame payloads, so this class has
 * zero H.264 format knowledge — it's a thin WebCodecs wrapper.
 */
export class H264Decoder {
    private decoder: VideoDecoder | null = null;
    private streamId: string;
    private onFrame: (frame: VideoFrame) => void;
    private isConfigured = false;

    constructor(streamId: string, onFrame: (frame: VideoFrame) => void) {
        this.streamId = streamId;
        this.onFrame = onFrame;
    }

    /**
     * Initialize the decoder by fetching codec configuration from the server.
     */
    async init() {
        console.log("H264Decoder: Fetching init for stream %s", this.streamId);

        const params = await fetchInit(this.streamId);
        const avcC = base64ToUint8Array(params.description);

        console.log("H264Decoder: Configuring decoder: %s, %dx%d, avcC=%d bytes",
            params.codec, params.width, params.height, avcC.length);

        this.decoder = new VideoDecoder({
            output: (frame) => this.handleFrame(frame),
            error: (e) => console.error("H264Decoder: Decoder error:", e),
        });

        this.decoder.configure({
            codec: params.codec,
            codedWidth: params.width,
            codedHeight: params.height,
            description: avcC,
        });
        this.isConfigured = true;

        console.log("H264Decoder: Decoder initialized successfully");
    }

    /**
     * Decode an AVCC frame payload from the server.
     *
     * The `avccData` is directly feedable to `EncodedVideoChunk` — no
     * format conversion needed.
     */
    decodeFrame(timestamp: number, isKeyframe: boolean, avccData: Uint8Array) {
        if (!this.decoder || !this.isConfigured) {
            console.error("Decoder not initialized");
            return;
        }

        if (DEBUG.debugStreamDecoder) {
            console.log(
                "H264Decoder: Decoding frame - timestamp: %d μs, keyframe: %s, size: %d bytes",
                timestamp, isKeyframe, avccData.length);
        }

        this.decoder.decode(new EncodedVideoChunk({
            type: isKeyframe ? "key" : "delta",
            timestamp,
            data: avccData,
        }));
    }

    private handleFrame(frame: VideoFrame) {
        if (DEBUG.debugStreamDecoder) {
            console.log(
                "H264Decoder: Frame decoded! %s %dx%d, timestamp: %d μs",
                frame.format,
                frame.displayWidth,
                frame.displayHeight,
                frame.timestamp);
        }
        this.onFrame(frame);
    }

    close() {
        if (this.decoder) {
            this.decoder.close();
            this.decoder = null;
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

function base64ToUint8Array(base64: string): Uint8Array {
    const binaryString = atob(base64);
    const len = binaryString.length;
    const bytes = new Uint8Array(len);
    for (let i = 0; i < len; i++) {
        bytes[i] = binaryString.charCodeAt(i);
    }
    return bytes;
}
