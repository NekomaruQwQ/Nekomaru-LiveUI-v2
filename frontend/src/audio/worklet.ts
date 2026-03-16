// AudioWorklet processor for low-latency PCM playback.
//
// Runs on the audio rendering thread.  Receives s16le PCM chunks from the
// main thread via MessagePort and plays them through a ring buffer.
//
// Ring buffer capacity: ~50ms at 48kHz stereo = 2400 frames × 2 channels.
// This is small enough to keep latency low, large enough to absorb jitter
// from the HTTP polling interval (~16ms) and WLAN variability (~5ms).
//
// On underrun, outputs silence (zeros) — no glitch artifacts.

/// Ring buffer capacity in sample frames (not individual samples).
/// 50ms at 48kHz = 2400 frames.
const RING_CAPACITY_FRAMES = 2400;

/// Messages received from the main thread.
interface PcmMessage {
    type: "pcm";
    /// Interleaved s16le samples as Int16Array.
    samples: Int16Array;
    /// Number of audio channels (used to calculate frame count).
    channels: number;
}

class PcmWorkletProcessor extends AudioWorkletProcessor {
    /// Ring buffer storing f32 samples (interleaved, all channels).
    private ring: Float32Array;
    /// Number of channels (set on first PCM message).
    private channels = 2;
    /// Read position in the ring (in individual samples, not frames).
    private readPos = 0;
    /// Write position in the ring.
    private writePos = 0;
    /// How many samples are currently buffered.
    private buffered = 0;

    constructor() {
        super();
        this.ring = new Float32Array(RING_CAPACITY_FRAMES * 2);
        this.port.onmessage = (e: MessageEvent<PcmMessage>) => this.onMessage(e.data);
    }

    private onMessage(msg: PcmMessage): void {
        if (msg.type !== "pcm") return;

        this.channels = msg.channels;
        const capacity = RING_CAPACITY_FRAMES * this.channels;

        // Resize ring if channel count changed.
        if (this.ring.length !== capacity) {
            this.ring = new Float32Array(capacity);
            this.readPos = 0;
            this.writePos = 0;
            this.buffered = 0;
        }

        // Convert s16le → f32 and write to ring buffer.
        const samples = msg.samples;
        for (let i = 0; i < samples.length; i++) {
            if (this.buffered >= capacity) break; // ring full — drop overflow
            this.ring[this.writePos] = samples[i]! / 32768;
            this.writePos = (this.writePos + 1) % capacity;
            this.buffered++;
        }
    }

    process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
        const output = outputs[0];
        if (!output || output.length === 0) return true;

        const channels = output.length;
        const frameCount = output[0]!.length; // typically 128

        for (let frame = 0; frame < frameCount; frame++) {
            if (this.buffered >= channels) {
                // Read one interleaved frame from the ring.
                for (let ch = 0; ch < channels; ch++) {
                    const capacity = RING_CAPACITY_FRAMES * this.channels;
                    output[ch]![frame] = this.ring[this.readPos]!;
                    this.readPos = (this.readPos + 1) % capacity;
                    this.buffered--;
                }
            } else {
                // Underrun — output silence.
                for (let ch = 0; ch < channels; ch++) {
                    output[ch]![frame] = 0;
                }
            }
        }

        return true;
    }
}

registerProcessor("pcm-worklet-processor", PcmWorkletProcessor);
