// Automatic live window selector.
//
// Polls the foreground window every 2 seconds via a one-shot
// `live-capture.exe --foreground-window` spawn.  When the foreground window
// matches the include list (and doesn't match the exclude list), and differs
// from the current capture target, the selector replaces the "main" stream
// in-place (bumping its generation counter) instead of destroying and
// recreating it.

import { captureExePath } from "./common";
import * as proc from "./process";

// ── Configuration ────────────────────────────────────────────────────────────

/// Paths or substrings that qualify a window's executable for capture.
const INCLUDE_LIST: string[] = [
    "devenv.exe",
    "C:\\Program Files\\Microsoft Visual Studio Code\\Code.exe",
    "C:\\Program Files\\JetBrains\\",
    "D:\\7-Games\\",
    "D:\\7-Games.Steam\\steamapps\\common\\",
    "E:\\Nekomaru.Games\\",
    "E:\\SteamLibrary\\steamapps\\common\\",
];

/// Paths or substrings that disqualify a window (checked case-insensitively).
const EXCLUDE_LIST: string[] = [
    "gogh.exe",
    "vtube studio.exe",
];

/// How often to poll the foreground window (ms).
const POLL_INTERVAL_MS = 2000;

/// Default capture resolution when auto-selecting a window.
const DEFAULT_WIDTH = 1920;
const DEFAULT_HEIGHT = 1200;

/// Well-known stream ID managed by the selector.
const STREAM_ID = "main";

// ── Types ────────────────────────────────────────────────────────────────────

/// JSON shape returned by `live-capture.exe --foreground-window`.
interface ForegroundWindowInfo {
    hwnd: number;
    pid: number;
    title: string;
    executable_path: string;
}

export interface SelectorStatus {
    active: boolean;
    currentStreamId: string | null;
    currentHwnd: string | null;
}

// ── Selector ─────────────────────────────────────────────────────────────────

/// Tracks the auto-capture state and manages the polling timer.
class LiveWindowSelector {
    private timer: ReturnType<typeof setInterval> | null = null;

    /// The hwnd of the last foreground window we observed, used to avoid
    /// redundant executable-path lookups when the foreground hasn't changed.
    private lastForegroundHwnd: string | null = null;

    /// The hwnd we are currently capturing.  Compared against new foreground
    /// windows to decide whether to switch.
    private lastCaptureHwnd: string | null = null;

    get active(): boolean {
        return this.timer !== null;
    }

    start(): void {
        if (this.timer) return; // already running
        console.log("[selector] started");
        this.timer = setInterval(() => this.poll(), POLL_INTERVAL_MS);
    }

    stop(): void {
        if (!this.timer) return;
        clearInterval(this.timer);
        this.timer = null;

        // Kill the stream we were managing.
        proc.destroyStream(STREAM_ID);
        console.log(`[selector] destroyed stream ${STREAM_ID}`);

        this.lastForegroundHwnd = null;
        this.lastCaptureHwnd = null;
        console.log("[selector] stopped");
    }

    status(): SelectorStatus {
        return {
            active: this.active,
            currentStreamId: this.active ? STREAM_ID : null,
            currentHwnd: this.lastCaptureHwnd,
        };
    }

    // ── Poll logic ───────────────────────────────────────────────────────

    private async poll(): Promise<void> {
        const info = await getForegroundWindow();
        if (!info) return;

        const hwndStr = formatHwnd(info.hwnd);

        // No change in foreground window — nothing to do.
        if (hwndStr === this.lastForegroundHwnd) return;
        this.lastForegroundHwnd = hwndStr;

        // Log foreground change (title masked for privacy, same as original).
        console.log(`[selector] foreground: *** (${info.executable_path})`);

        if (!shouldCapture(info.executable_path)) return;

        // Already capturing this window.
        if (hwndStr === this.lastCaptureHwnd) return;

        // ── Switch capture ───────────────────────────────────────────────
        // replaceStream is idempotent — creates the "main" stream if it
        // doesn't exist, or kills the old process + bumps generation.
        proc.replaceStream(STREAM_ID, hwndStr, DEFAULT_WIDTH, DEFAULT_HEIGHT);
        this.lastCaptureHwnd = hwndStr;
        console.log(`[selector] capturing ${hwndStr} → stream ${STREAM_ID}`);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Spawn `live-capture.exe --foreground-window` and parse the JSON result.
/// Returns null if the process fails or the foreground window is null.
async function getForegroundWindow(): Promise<ForegroundWindowInfo | null> {
    try {
        const child = Bun.spawn([captureExePath, "--foreground-window"], {
            stdout: "pipe",
            stderr: "pipe",
        });

        const stdout = await new Response(child.stdout).text();
        await child.exited;

        const parsed = JSON.parse(stdout);
        // live-capture outputs JSON `null` when no foreground window exists.
        return parsed as ForegroundWindowInfo | null;
    } catch (e) {
        console.error("[selector] failed to get foreground window:", e);
        return null;
    }
}

/// Format a numeric hwnd as a 0x hex string, matching the format used by
/// the process manager and API.
function formatHwnd(hwnd: number): string {
    return `0x${hwnd.toString(16).toUpperCase()}`;
}

/// Determines whether a window's executable path qualifies for capture.
/// Mirrors the original Rust `should_capture` logic: must match at least one
/// include entry and must not match any exclude entry.
function shouldCapture(executablePath: string): boolean {
    const included = INCLUDE_LIST.some((pattern) =>
        executablePath.includes(pattern));
    const excluded = EXCLUDE_LIST.some((pattern) =>
        executablePath.toLowerCase().includes(pattern));
    return included && !excluded;
}

// ── Singleton ────────────────────────────────────────────────────────────────

export const selector = new LiveWindowSelector();
