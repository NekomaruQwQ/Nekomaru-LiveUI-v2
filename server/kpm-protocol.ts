// JSON line parser for live-kpm.exe stdout.
//
// Each stdout line is a JSON object: {"t":1710590400123456,"c":5}
// where t = timestamp in microseconds, c = keystroke count.
//
// This is much simpler than the binary parsers for audio/video — just
// accumulate text, split on newlines, parse JSON.

export interface KpmBatch {
    timestampUs: number;
    count: number;
}

export type KpmCallback = (batch: KpmBatch) => void;

/// Incremental JSON line parser for the KPM binary protocol.
///
/// Accumulates stdout chunks as text, splits on newlines, and parses
/// each complete line as a JSON batch object.
export class KpmProtocolParser {
    private pending = "";

    constructor(
        private readonly streamId: string,
        private readonly onBatch: KpmCallback,
    ) {}

    /// Feed raw bytes from the child process's stdout.
    feed(chunk: Uint8Array): void {
        this.pending += new TextDecoder().decode(chunk);

        for (
            let nl = this.pending.indexOf("\n");
            nl !== -1;
            nl = this.pending.indexOf("\n")
        ) {
            const line = this.pending.slice(0, nl).trim();
            this.pending = this.pending.slice(nl + 1);

            if (line.length === 0) continue;

            try {
                const obj = JSON.parse(line) as { t: number; c: number };
                this.onBatch({
                    timestampUs: obj.t,
                    count: obj.c,
                });
            } catch {
                // Malformed line — skip (could be partial write or log noise).
            }
        }
    }
}
