// KPM (keystrokes-per-minute) capture process manager.
//
// Singleton that spawns live-kpm.exe, wires stdout through the
// KpmProtocolParser into a KpmCalculator, and forwards stderr to
// the server logger.  Always enabled — no env gating needed.

import type { Subprocess } from "bun";

import { KpmCalculator } from "./kpm-buffer";
import { KpmProtocolParser } from "./kpm-protocol";
import { kpmExePath } from "./common";
import { createLogger, isCaptureLogHead, writeCaptureGroup } from "./log";

const MODULE = "server::kpm";
const log = createLogger(MODULE);

/// Batch interval passed to live-kpm.exe (milliseconds).
const BATCH_INTERVAL_MS = 50;

/// Sliding window duration for KPM calculation (milliseconds).
const WINDOW_DURATION_MS = 5000;

/// Logical stream ID for log markers.
const LOG_STREAM_ID = "kpm";

// ── Manager ──────────────────────────────────────────────────────────────────

class KpmManager {
    private process: Subprocess | null = null;
    readonly calculator = new KpmCalculator(WINDOW_DURATION_MS, BATCH_INTERVAL_MS);

    get active(): boolean {
        return this.process !== null;
    }

    start(): void {
        if (this.process) return;

        const proc = Bun.spawn(
            [kpmExePath, "--batch-interval", String(BATCH_INTERVAL_MS)],
            { stdout: "pipe", stderr: "pipe" });
        this.process = proc;

        // Wire stdout → KpmProtocolParser → KpmCalculator
        const parser = new KpmProtocolParser(LOG_STREAM_ID, (batch) => {
            this.calculator.pushBatch(batch);
        });

        pipeStdout(proc, parser);
        pipeStderr(proc);

        proc.exited.then((code) => {
            log.info(`process exited with code ${code}`);
            this.process = null;
            this.calculator.reset();
        });

        log.info(`started (batch: ${BATCH_INTERVAL_MS}ms, window: ${WINDOW_DURATION_MS}ms)`);
    }

    stop(): void {
        if (!this.process) return;

        this.process.kill();
        this.process = null;
        this.calculator.reset();

        log.info("stopped");
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Read stdout chunks from the child process and feed them to the parser.
async function pipeStdout(proc: Subprocess, parser: KpmProtocolParser): Promise<void> {
    try {
        const reader = proc.stdout.getReader();
        while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            parser.feed(value);
        }
    } catch {
        log.info("stdout closed");
    }
}

/// Forward stderr lines from the Rust child process, using the same
/// grouped log rendering as the capture processes.
async function pipeStderr(proc: Subprocess): Promise<void> {
    let group: string[] = [];
    let flushTimer: ReturnType<typeof setTimeout> | null = null;
    const FLUSH_DELAY_MS = 10;

    function flush(): void {
        if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }
        if (group.length > 0) {
            writeCaptureGroup(LOG_STREAM_ID, group);
            group = [];
        }
    }

    function pushLine(line: string): void {
        if (isCaptureLogHead(line)) flush();
        group.push(line);
        if (flushTimer) clearTimeout(flushTimer);
        flushTimer = setTimeout(flush, FLUSH_DELAY_MS);
    }

    try {
        const reader = proc.stderr.getReader();
        const decoder = new TextDecoder();
        let pending = "";
        while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            pending += decoder.decode(value, { stream: true });
            for (let nl = pending.indexOf("\n"); nl !== -1; nl = pending.indexOf("\n")) {
                const line = pending.slice(0, nl);
                pending = pending.slice(nl + 1);
                if (line.length > 0) pushLine(line);
            }
        }
        if (pending.length > 0) pushLine(pending);
        flush();
    } catch {
        flush();
    }
}

// ── Singleton ────────────────────────────────────────────────────────────────

export const kpmManager = new KpmManager();
