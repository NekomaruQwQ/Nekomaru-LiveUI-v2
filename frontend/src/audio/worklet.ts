// AudioWorklet processor for low-latency PCM playback.
//
// Runs on the audio rendering thread.  Receives s16le PCM chunks from the
// main thread via MessagePort and plays them through a ring buffer.
//
// Ring buffer capacity: ~200ms at 48kHz stereo = 9600 frames × 2 channels.
// Large enough to absorb HTTP polling jitter (~25-35ms effective interval),
// occasional GC pauses, and bursty chunk delivery — while keeping latency
// well under the 250ms streaming target.
//
// Pre-buffering: the worklet outputs silence until the ring has accumulated
// at least PRE_BUFFER_FRAMES of audio (~100ms).  This is a one-shot gate —
// once the threshold is reached, playback starts and never re-enters the
// pre-buffering state.  Without this, the ring starts near-empty on first
// connect and any HTTP jitter causes underruns (audible chopping).
//
// On underrun after priming, outputs silence (zeros) — no glitch artifacts.

/// Ring buffer capacity in sample frames (not individual samples).
/// 200ms at 48kHz = 9600 frames.
const RING_CAPACITY_FRAMES = 9600;

/// Pre-buffer threshold in frames.  The worklet outputs silence until this
/// many frames have been buffered, then starts playback permanently.
/// 100ms at 48kHz = 4800 frames.
const PRE_BUFFER_FRAMES = 4800;

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
    /// One-shot pre-buffer gate.  Once the ring reaches PRE_BUFFER_FRAMES,
    /// this flips to true and stays true forever.
    private primed = false;

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

        // Drop the entire chunk if it won't fit — preserves PCM continuity
        // within kept chunks instead of truncating mid-sample.
        const samples = msg.samples;
        if (samples.length > capacity - this.buffered) return;

        // Convert s16le → f32 and write to ring buffer.
        for (let i = 0; i < samples.length; i++) {
            // biome-ignore lint/style/noNonNullAssertion: index bounded by samples.length
            this.ring[this.writePos] = samples[i]! / 32768;
            this.writePos = (this.writePos + 1) % capacity;
            this.buffered++;
        }
    }

    override process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
        const output = outputs[0];
        if (!output || output.length === 0) return true;

        const channels = output.length;
        // biome-ignore lint/style/noNonNullAssertion: output[0] guaranteed by length check above
        const frameCount = output[0]!.length; // typically 128
        const capacity = RING_CAPACITY_FRAMES * this.channels;

        // One-shot pre-buffer gate: output silence until the ring has
        // accumulated enough audio to absorb startup jitter.
        if (!this.primed) {
            if (this.buffered >= PRE_BUFFER_FRAMES * this.channels) {
                this.primed = true;
            } else {
                // Fill output with silence while pre-buffering.
                for (let ch = 0; ch < channels; ch++) {
                    // biome-ignore lint/style/noNonNullAssertion: ch bounded by output.length
                    output[ch]!.fill(0);
                }
                return true;
            }
        }

        for (let frame = 0; frame < frameCount; frame++) {
            if (this.buffered >= channels) {
                // Read one interleaved frame from the ring.
                for (let ch = 0; ch < channels; ch++) {
                    // biome-ignore lint/style/noNonNullAssertion: ch bounded by output.length, readPos bounded by capacity
                    output[ch]![frame] = this.ring[this.readPos]!;
                    this.readPos = (this.readPos + 1) % capacity;
                    this.buffered--;
                }
            } else {
                // Underrun — output silence.
                for (let ch = 0; ch < channels; ch++) {
                    // biome-ignore lint/style/noNonNullAssertion: ch bounded by output.length
                    output[ch]![frame] = 0;
                }
            }
        }

        return true;
    }
}

registerProcessor("pcm-worklet-processor", PcmWorkletProcessor);
