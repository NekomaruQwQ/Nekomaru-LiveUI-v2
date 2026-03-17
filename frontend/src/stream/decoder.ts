import { DEBUG } from "../../debug";

export interface NALUnitData {
    type: number;
    data: Uint8Array;
}

export interface StreamFrameData {
    timestamp: number;
    nalUnits: NALUnitData[];
    isKeyframe: boolean;
}

// ── Init with retry ─────────────────────────────────────────────────────

/// Codec init params returned by the server.
interface InitParams {
    sps: string;
    pps: string;
    width: number;
    height: number;
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
async function fetchInit(streamId: string): Promise<InitParams> {
    const maxRetries = 30;
    const baseDelayMs = 250;

    for (let attempt = 0; attempt < maxRetries; attempt++) {
        const res = await fetch(`/api/v1/streams/${streamId}/init`);

        if (res.ok) {
            return await res.json() as InitParams;
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
 * H.264 decoder using WebCodecs API
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
     * Initialize the decoder by fetching codec parameters from the stream
     */
    async init() {
        console.log("H264Decoder: Starting initialization...");
        console.log("H264Decoder: Fetching codec params for stream %s", this.streamId);

        // fetchInit retries on 503 (stream starting up) with exponential backoff.
        const params = await fetchInit(this.streamId);

        console.log("H264Decoder: Received params:", {
            sps_base64_length: params.sps.length,
            pps_base64_length: params.pps.length,
            width: params.width,
            height: params.height,
        });

        let sps = base64ToUint8Array(params.sps);
        let pps = base64ToUint8Array(params.pps);
        console.log("H264Decoder: Decoded base64 - SPS size:", sps.length, "PPS size:", pps.length);
        console.log("H264Decoder: SPS bytes (first 10):", Array.from(sps.slice(0, 10)).map(b => "0x" + b.toString(16).padStart(2, "0")).join(" "));
        console.log("H264Decoder: PPS bytes (first 10):", Array.from(pps.slice(0, 10)).map(b => "0x" + b.toString(16).padStart(2, "0")).join(" "));

        const width = params.width;
        const height = params.height;

        // Strip Annex B start codes (00 00 00 01 or 00 00 01) from NAL units
        const spsOriginalLength = sps.length;
        const ppsOriginalLength = pps.length;
        sps = stripStartCode(sps);
        pps = stripStartCode(pps);
        console.log("H264Decoder: Stripped start codes - SPS: %d → %d bytes, PPS: %d → %d bytes",
            spsOriginalLength, sps.length, ppsOriginalLength, pps.length);
        console.log("H264Decoder: SPS bytes after strip (first 10):", Array.from(sps.slice(0, 10)).map(b => "0x" + b.toString(16).padStart(2, "0")).join(" "));

        // Parse profile/level from SPS
        // SPS structure: [NAL header, profile_idc, constraint_flags, level_idc, ...]
        const profile = sps[1]!;       // profile_idc (e.g., 0x42 for Baseline)
        const constraints = sps[2]!;   // constraint flags
        const level = sps[3]!;         // level_idc (e.g., 0x1f for level 3.1)

        console.log("H264Decoder: Parsed SPS - Profile: 0x%s, Constraints: 0x%s, Level: 0x%s",
            toHex(profile), toHex(constraints), toHex(level));

        // Build codec string (format: avc1.PPCCLL where PP=profile, CC=constraints, LL=level)
        // Example: "avc1.42001f" for Baseline profile, level 3.1
        const codecString = `avc1.${toHex(profile)}${toHex(constraints)}${toHex(level)}`;
        console.log("H264Decoder: Codec string:", codecString);

        // Build avcC descriptor (ISO 14496-15 format)
        const avcC = buildAvcCDescriptor(sps, pps);
        console.log("H264Decoder: Built avcC descriptor, size:", avcC.length, "bytes");
        console.log("H264Decoder: avcC bytes (first 20):", Array.from(avcC.slice(0, 20)).map(b => "0x" + b.toString(16).padStart(2, "0")).join(" "));

        this.decoder = new VideoDecoder({
            output: (frame) => this.handleFrame(frame),
            error: (e) => console.error("H264Decoder: Decoder error:", e),
        });

        const config: VideoDecoderConfig = {
            codec: codecString,
            codedWidth: width,
            codedHeight: height,
            description: avcC,
        };

        console.log("H264Decoder: Configuring VideoDecoder with:", config);
        this.decoder.configure(config);
        this.isConfigured = true;

        console.log("H264Decoder: Decoder initialized successfully: %s, %dx%d", codecString, width, height);
    }

    /**
     * Decode a frame
     */
    decodeFrame(frameData: StreamFrameData) {
        if (!this.decoder || !this.isConfigured) {
            console.error("Decoder not initialized");
            return;
        }

        if (DEBUG.debugStreamDecoder) {
            console.log(
                "H264Decoder: Decoding frame - timestamp: %d μs, NAL units: %d, keyframe: %s",
                frameData.timestamp,
                frameData.nalUnits.length,
                frameData.isKeyframe);
        }

        // Strip Annex B start codes from each NAL unit and convert to AVCC format
        // AVCC format: [4-byte length][NAL data][4-byte length][NAL data]...
        let totalSize = 0;
        const nalDataWithoutStartCodes: Uint8Array[] = [];

        for (const unit of frameData.nalUnits) {
            if (DEBUG.debugStreamDecoder) {
                console.log(
                    "H264Decoder:   NAL unit type: %d, size: %d bytes (with start code)",
                    unit.type,
                    unit.data.length);
            }

            // Strip Annex B start code from this NAL unit
            const nalData = stripStartCode(unit.data);
            if (DEBUG.debugStreamDecoder) {
                console.log(
                    "H264Decoder:     → %d bytes after stripping start code", nalData.length);
            }

            nalDataWithoutStartCodes.push(nalData);
            totalSize += 4 + nalData.length; // 4 bytes for length prefix + NAL data
        }

        // Build AVCC-format data: length-prefixed NAL units
        const combined = new Uint8Array(totalSize);
        let offset = 0;

        for (const nalData of nalDataWithoutStartCodes) {
            // Write 4-byte big-endian length prefix
            const length = nalData.length;
            combined[offset++] = (length >> 24) & 0xff;
            combined[offset++] = (length >> 16) & 0xff;
            combined[offset++] = (length >> 8) & 0xff;
            combined[offset++] = length & 0xff;

            // Write NAL data
            combined.set(nalData, offset);
            offset += nalData.length;
        }

        if (DEBUG.debugStreamDecoder) {
            console.log("H264Decoder: Combined AVCC frame data: %d bytes total", totalSize);
        }

        const chunk = new EncodedVideoChunk({
            type: frameData.isKeyframe ? "key" : "delta",
            timestamp: frameData.timestamp,
            data: combined,
        });

        if (DEBUG.debugStreamDecoder) {
            console.log("H264Decoder: Submitting EncodedVideoChunk to decoder");
        }

        this.decoder.decode(chunk);
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

// ── Codec helpers ───────────────────────────────────────────────────────

/**
 * Build avcC descriptor for H.264 decoder configuration (ISO 14496-15 format)
 */
function buildAvcCDescriptor(sps: Uint8Array, pps: Uint8Array): Uint8Array {
    const spsLength = sps.length;
    const ppsLength = pps.length;

    const avcC = new Uint8Array(
        1 +  // configurationVersion
        3 +  // AVCProfileIndication, profile_compatibility, AVCLevelIndication
        1 +  // lengthSizeMinusOne
        1 +  // numOfSequenceParameterSets
        2 + spsLength +  // SPS length (16-bit) + data
        1 +  // numOfPictureParameterSets
        2 + ppsLength    // PPS length (16-bit) + data
    );

    let offset = 0;

    // configurationVersion = 1
    avcC[offset++] = 1;

    // Copy profile/level from SPS (bytes 1-3)
    avcC[offset++] = sps[1]!;  // AVCProfileIndication
    avcC[offset++] = sps[2]!;  // profile_compatibility
    avcC[offset++] = sps[3]!;  // AVCLevelIndication

    // lengthSizeMinusOne = 0xFF (4 bytes)
    avcC[offset++] = 0xFF;

    // numOfSequenceParameterSets = 1
    avcC[offset++] = 0xE1;

    // SPS length (16-bit big-endian)
    avcC[offset++] = (spsLength >> 8) & 0xFF;
    avcC[offset++] = spsLength & 0xFF;

    // SPS data
    avcC.set(sps, offset);
    offset += spsLength;

    // numOfPictureParameterSets = 1
    avcC[offset++] = 1;

    // PPS length (16-bit big-endian)
    avcC[offset++] = (ppsLength >> 8) & 0xFF;
    avcC[offset++] = ppsLength & 0xFF;

    // PPS data
    avcC.set(pps, offset);

    return avcC;
}

/**
 * Convert base64 string to Uint8Array
 */
function base64ToUint8Array(base64: string): Uint8Array {
    const binaryString = atob(base64);
    const len = binaryString.length;
    const bytes = new Uint8Array(len);
    for (let i = 0; i < len; i++) {
        bytes[i] = binaryString.charCodeAt(i);
    }
    return bytes;
}

/**
 * Convert number to 2-digit hex string
 */
function toHex(value: number): string {
    return value.toString(16).padStart(2, "0");
}

/**
 * Strip Annex B start code (00 00 00 01 or 00 00 01) from NAL unit
 */
function stripStartCode(data: Uint8Array): Uint8Array {
    // Check for 4-byte start code (00 00 00 01)
    if (data.length >= 4 && data[0] === 0x00 && data[1] === 0x00 && data[2] === 0x00 && data[3] === 0x01) {
        return data.slice(4);
    }
    // Check for 3-byte start code (00 00 01)
    if (data.length >= 3 && data[0] === 0x00 && data[1] === 0x00 && data[2] === 0x01) {
        return data.slice(3);
    }
    // No start code found, return as-is
    return data;
}

// ── Frame parser ────────────────────────────────────────────────────────

/**
 * Parse binary stream frame data
 */
export function parseStreamFrame(buffer: Uint8Array): StreamFrameData {
    if (DEBUG.debugStreamDecoder) {
        console.log("H264Decoder(parseStreamFrame): Parsing frame from %d bytes", buffer.length);
    }
    const view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);

    let offset = 0;

    // Read timestamp (u64 little-endian)
    const timestamp = Number(view.getBigUint64(offset, true));
    offset += 8;
    if (DEBUG.debugStreamDecoder) {
        console.log("H264Decoder(parseStreamFrame): Timestamp: %d μs", timestamp);
    }

    // Read number of NAL units (u32 little-endian)
    const numNalUnits = view.getUint32(offset, true);
    offset += 4;
    if (DEBUG.debugStreamDecoder) {
        console.log("H264Decoder(parseStreamFrame): Number of NAL units: %d", numNalUnits);
    }

    const nalUnits: NALUnitData[] = [];
    let isKeyframe = false;

    for (let i = 0; i < numNalUnits; i++) {
        // Read NAL unit type (u8)
        const type = view.getUint8(offset);
        offset += 1;

        // Read data length (u32 little-endian)
        const dataLength = view.getUint32(offset, true);
        offset += 4;

        // Read data
        const data = buffer.slice(offset, offset + dataLength);
        offset += dataLength;

        if (DEBUG.debugStreamDecoder) {
            console.log(
                "H264Decoder(parseStreamFrame):   NAL unit %d: type=%d, size=%d bytes",
                i,
                type,
                dataLength);
        }

        nalUnits.push({ type, data });

        // Check if this is an IDR frame (type 5)
        if (type === 5) {
            isKeyframe = true;
        }
    }

    if (DEBUG.debugStreamDecoder) {
        console.log(
            "H264Decoder(parseStreamFrame): Parsed frame: timestamp=%d μs, nalUnits=%d, isKeyframe=%s",
            timestamp,
            nalUnits.length,
            isKeyframe);
    }

    return { timestamp, nalUnits, isKeyframe };
}
