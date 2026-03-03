// Server-side YouTube Music stream manager.
//
// Polls `enumerateWindows()` every 5 seconds looking for a window whose title
// starts with "YouTube Music".  When found, creates (or replaces) a crop-mode
// stream with the well-known ID "youtube-music" capturing the bottom 96px of
// the window (the playback bar).  When the window disappears, the stream is
// destroyed.
//
// Structurally parallel to `LiveWindowSelector` in selector.ts, but manages
// the "youtube-music" stream instead of "main".

import * as proc from "./process";

// ── Configuration ────────────────────────────────────────────────────────────

/// How often to re-check for the YouTube Music window (ms).
/// Slow poll — this isn't latency-sensitive.
const POLL_INTERVAL_MS = 5000;

/// Well-known stream ID for the YouTube Music playback bar.
const STREAM_ID = "youtube-music";

// ── Types ────────────────────────────────────────────────────────────────────

/// Minimal window info shape from `enumerateWindows()`.
interface WindowInfo {
    hwnd: number;
    title: string;
    width: number;
    height: number;
}

export interface YouTubeMusicStatus {
    active: boolean;
    streamId: string | null;
    currentHwnd: string | null;
}

// ── Manager ──────────────────────────────────────────────────────────────────

class YouTubeMusicManager {
    private timer: ReturnType<typeof setInterval> | null = null;

    /// The hwnd of the last-known YouTube Music window, so we can detect when
    /// the window is restarted (new hwnd) and replace the stream accordingly.
    private lastKnownHwnd: string | null = null;

    get active(): boolean {
        return this.timer !== null;
    }

    start(): void {
        if (this.timer) return;
        console.log("[ytm] started");
        // Run an immediate poll so we don't wait a full interval on startup.
        this.poll();
        this.timer = setInterval(() => this.poll(), POLL_INTERVAL_MS);
    }

    stop(): void {
        if (!this.timer) return;
        clearInterval(this.timer);
        this.timer = null;

        proc.destroyStream(STREAM_ID);
        this.lastKnownHwnd = null;
        console.log("[ytm] stopped");
    }

    status(): YouTubeMusicStatus {
        return {
            active: this.active,
            streamId: this.lastKnownHwnd ? STREAM_ID : null,
            currentHwnd: this.lastKnownHwnd,
        };
    }

    // ── Poll logic ───────────────────────────────────────────────────────

    private async poll(): Promise<void> {
        try {
            const windows = await proc.enumerateWindows() as WindowInfo[];
            const ytm = windows.find((w) => w.title === "YouTube Music - Nekomaru LiveUI v2");

            if (ytm) {
                const hwndStr = formatHwnd(ytm.hwnd);

                if (hwndStr !== this.lastKnownHwnd) {
                    console.log(`[capture:youtube-music] window detected: ${hwndStr} (${ytm.width}x${ytm.height})`);
                    // Window appeared or was restarted (new hwnd) —
                    // replaceCropStream is idempotent (creates or replaces).
                    // Crop the bottom 96px of the window (playback bar).
                    const titleBarHeight = 48;
                    const barHeight = 112;
                    const bottomMargin = 12;
                    const rightMargin = 96;
                    const minY = Math.max(0, ytm.height - barHeight - bottomMargin + titleBarHeight);
                    const maxY = Math.max(minY, ytm.height - bottomMargin + titleBarHeight);
                    proc.replaceCropStream(
                        STREAM_ID, hwndStr, 0, minY, ytm.width - rightMargin, maxY, 2);
                    this.lastKnownHwnd = hwndStr;
                    console.log(`[ytm] capturing ${hwndStr} (${ytm.width}x${ytm.height})`);
                }
            } else if (this.lastKnownHwnd) {
                // YouTube Music window disappeared — tear down the stream.
                proc.destroyStream(STREAM_ID);
                this.lastKnownHwnd = null;
                console.log("[ytm] window disappeared, stream destroyed");
            }
        } catch (e) {
            console.error("[ytm] poll failed:", e);
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function formatHwnd(hwnd: number): string {
    return `0x${hwnd.toString(16).toUpperCase()}`;
}

// ── Singleton ────────────────────────────────────────────────────────────────

export const ytmManager = new YouTubeMusicManager();
