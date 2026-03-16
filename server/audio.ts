// Audio capture process manager.
//
// Singleton that spawns live-audio.exe, wires stdout through the
// AudioProtocolParser into an AudioBuffer, and forwards stderr to
// the server logger.  One global audio source — not per-window.

import type { Subprocess } from "bun";

import { AudioBuffer } from "./audio-buffer";
import { AudioProtocolParser } from "./audio-protocol";
import { audioExePath, audioDeviceName } from "./common";
import { createLogger, isCaptureLogHead, writeCaptureGroup } from "./log";

const MODULE = "server::audio";
const log = createLogger(MODULE);

/// Audio buffer capacity: 100 chunks = ~1 second at 10ms/chunk.
const AUDIO_BUFFER_CAPACITY = 100;

/// Logical stream ID for log markers.  Not a video stream — just used for
/// consistent log formatting with the capture stderr forwarding.
const LOG_STREAM_ID = "audio";

// ── Manager ──────────────────────────────────────────────────────────────────

class AudioManager {
    private process: Subprocess | null = null;
    readonly buffer = new AudioBuffer(AUDIO_BUFFER_CAPACITY);

    get active(): boolean {
        return this.process !== null;
    }

    start(): void {
        if (this.process) return;

        const proc = Bun.spawn([audioExePath, "--device", audioDeviceName], {
            stdout: "pipe",
            stderr: "pipe",
        });
        this.process = proc;

        // Wire stdout → AudioProtocolParser → AudioBuffer
        const parser = new AudioProtocolParser(LOG_STREAM_ID, (msg) => {
            switch (msg.type) {
                case "audio_params":
                    this.buffer.setAudioParams(msg.params);
                    log.info(`audio params: ${msg.params.sampleRate}Hz, ` +
                        `${msg.params.channels}ch, ${msg.params.bitsPerSample}-bit`);
                    break;
                case "audio_frame":
                    this.buffer.pushChunk(msg.chunk);
                    break;
                case "error":
                    log.error(`capture error: ${msg.message}`);
                    break;
            }
        });

        pipeStdout(proc, parser);
        pipeStderr(proc);

        proc.exited.then((code) => {
            log.info(`process exited with code ${code}`);
            this.process = null;
            this.buffer.reset();  // Clear stale state for clean restart.
        });

        log.info("started");
    }

    stop(): void {
        if (!this.process) return;

        this.process.kill();
        this.process = null;
        this.buffer.reset();

        log.info("stopped");
    }

    status(): { active: boolean } {
        return { active: this.active };
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Read stdout chunks from the child process and feed them to the parser.
async function pipeStdout(proc: Subprocess, parser: AudioProtocolParser): Promise<void> {
    try {
        const reader = proc.stdout.getReader();
        while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            parser.feed(value);
        }
    } catch {
        // Expected when the process is killed.
        log.info("stdout closed");
    }
}

/// Forward stderr lines from the Rust child process, using the same
/// grouped log rendering as the video capture process.
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

export const audioManager = new AudioManager();
