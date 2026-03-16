// Sliding window KPM calculator.
//
// Receives timestamped keystroke batches from the KPM protocol parser and
// computes a keystrokes-per-minute value over a configurable sliding window.
// The frontend polls the server for the current KPM and handles peak hold
// + decay animations entirely on its own side.

interface Entry {
    timestampUs: number;
    count: number;
}

/// Computes keystrokes-per-minute from a sliding window of recent batches.
export class KpmCalculator {
    /// Ring buffer of recent batches.
    private entries: Entry[] = [];
    /// Maximum entries to keep (window duration / batch interval).
    private readonly capacity: number;
    /// Sliding window duration in microseconds.
    private readonly windowUs: number;

    /// @param windowMs  Sliding window duration in milliseconds (e.g. 5000).
    /// @param batchMs   Batch interval in milliseconds (e.g. 50) — used to
    ///                  size the ring buffer.
    constructor(windowMs: number, batchMs: number) {
        this.windowUs = windowMs * 1000;
        this.capacity = Math.ceil(windowMs / batchMs) + 10; // small slack
    }

    /// Record a new batch from the capture process.
    pushBatch(batch: { timestampUs: number; count: number }): void {
        this.entries.push({
            timestampUs: batch.timestampUs,
            count: batch.count,
        });

        // Evict entries older than the window.
        const cutoff = batch.timestampUs - this.windowUs;
        while (this.entries.length > 0 && this.entries[0]!.timestampUs < cutoff) {
            this.entries.shift();
        }

        // Hard cap in case of clock drift or very long windows.
        while (this.entries.length > this.capacity) {
            this.entries.shift();
        }
    }

    /// Compute current KPM from the sliding window.
    ///
    /// Returns 0 if there's insufficient data (fewer than 2 entries or
    /// time span < 200ms).
    getKpm(): number {
        if (this.entries.length < 2) return 0;

        const oldest = this.entries[0]!;
        const newest = this.entries[this.entries.length - 1]!;
        const spanUs = newest.timestampUs - oldest.timestampUs;

        // Need at least 200ms of data for a meaningful extrapolation.
        if (spanUs < 200_000) return 0;

        let totalCount = 0;
        for (const entry of this.entries) {
            totalCount += entry.count;
        }

        // Extrapolate to per-minute: (count / seconds) * 60.
        const spanSeconds = spanUs / 1_000_000;
        return (totalCount / spanSeconds) * 60;
    }

    /// Clear all state (e.g. on process restart).
    reset(): void {
        this.entries = [];
    }
}
