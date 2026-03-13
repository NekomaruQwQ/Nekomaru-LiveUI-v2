// Automatic live window selector.
//
// Polls the foreground window every 2 seconds via a one-shot
// `live-capture.exe --foreground-window` spawn.  When the foreground window
// matches the include list (and doesn't match the exclude list), and differs
// from the current capture target, the selector replaces the "main" stream
// in-place (bumping its generation counter) instead of destroying and
// recreating it.
//
// Each preset is a flat `string[]` of pattern entries.  Entries are
// include rules by default; `@exclude` prefix marks an exclusion rule.
// Include entries may carry a `@mode` prefix (e.g. `@code devenv.exe`)
// that is pushed as the `$liveMode` computed string on capture switch.
// The full pattern format is `[@mode] <exePath>[@<windowTitle>]`.
// If no `@` separator is present in the body, only the executable path
// is matched (backward-compatible).  When both parts are given, both
// must match (AND).  The title part is always compared case-insensitively.
//
// Include/exclude config is persisted to data/selector-config.json — loaded
// on server start, written on every setConfig() call.  Falls back to
// hardcoded defaults if the file is missing or corrupt.

import * as path from "node:path";
import { captureExePath, dataDir } from "./common";
import { createLogger, createStreamLogger } from "./log";
import { loadJson, saveJson } from "./persist";
import * as proc from "./process";
import { setComputed, clearComputed } from "./strings";

/// Well-known stream ID managed by the selector.
const STREAM_ID = "main";

const MODULE = "server::selector";
const log = createLogger(MODULE);
const streamLog = createStreamLogger(STREAM_ID, MODULE);

// ── Configuration ────────────────────────────────────────────────────────────

/// Default preset name used when no config file exists or when loading legacy format.
const DEFAULT_PRESET_NAME = "default";

/// Default presets map.  Contains a single "default" preset with hardcoded patterns.
/// Entries are include by default; `@exclude` marks exclusion rules.
const DEFAULT_PRESETS: Record<string, string[]> = {
    [DEFAULT_PRESET_NAME]: [
        "@code devenv.exe",
        "@code C:/Program Files/Microsoft Visual Studio Code/Code.exe",
        "@code C:/Program Files/JetBrains/",
        "@game D:/7-Games/",
        "@game D:/7-Games.Steam/steamapps/common/",
        "@game E:/Nekomaru-Games/",
        "@game E:/SteamLibrary/steamapps/common/",
        "@exclude gogh.exe",
        "@exclude vtube studio.exe",
    ],
};

/// Persistence path for the include/exclude config.
const configPath = path.join(dataDir, "selector-config.json");

/// How often to poll the foreground window (ms).
const POLL_INTERVAL_MS = 2000;

/// Default capture resolution when auto-selecting a window.
const DEFAULT_WIDTH = 1920;
const DEFAULT_HEIGHT = 1200;

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
    currentTitle: string | null;
}

/// Top-level config shape persisted to disk.  Contains a named active preset
/// and a map of all available presets (each preset is a flat pattern list).
export interface PresetConfig {
    preset: string;
    presets: Record<string, string[]>;
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

    /// Title of the window we are currently capturing, captured at switch time.
    /// Pushed to the computed string store as "$captureWindowTitle".
    private lastCaptureTitle: string | null = null;

    /// Active preset name and all available presets — editable at runtime via the API.
    private preset: string = DEFAULT_PRESET_NAME;
    private presets: Record<string, string[]> = structuredClone(DEFAULT_PRESETS);

    get active(): boolean {
        return this.timer !== null;
    }

    start(): void {
        if (this.timer) return; // already running
        log.info("started");
        setComputed("$captureMode", "auto");
        this.timer = setInterval(() => this.poll(), POLL_INTERVAL_MS);
    }

    stop(): void {
        if (!this.timer) return;
        clearInterval(this.timer);
        this.timer = null;

        // Kill the stream we were managing.
        proc.destroyStream(STREAM_ID);
        streamLog.info("destroyed stream");

        this.lastForegroundHwnd = null;
        this.lastCaptureHwnd = null;
        this.lastCaptureTitle = null;
        clearComputed("$captureWindowTitle");
        clearComputed("$captureMode");
        clearComputed("$liveMode");
        log.info("stopped");
    }

    status(): SelectorStatus {
        return {
            active: this.active,
            currentStreamId: this.active ? STREAM_ID : null,
            currentHwnd: this.lastCaptureHwnd,
            currentTitle: this.lastCaptureTitle,
        };
    }

    getConfig(): PresetConfig {
        return { preset: this.preset, presets: structuredClone(this.presets) };
    }

    /// Replace the full preset config and persist to disk.
    async setConfig(config: PresetConfig): Promise<void> {
        this.preset = config.preset;
        this.presets = structuredClone(config.presets);
        await this.persist();
        log.info(`config updated: preset="${this.preset}", ${Object.keys(this.presets).length} preset(s)`);
    }

    /// Switch the active preset by name.  Throws if the preset doesn't exist.
    async setPreset(name: string): Promise<void> {
        if (!(name in this.presets)) throw new Error(`preset "${name}" not found`);
        this.preset = name;
        await this.persist();
        log.info(`switched to preset "${name}"`);
    }

    /// Load persisted config from disk, replacing the hardcoded defaults.
    /// Falls back silently to defaults if the file is missing or corrupt.
    /// Migrates the legacy `{ include, exclude }` per-preset format to the
    /// flat `string[]` format, prepending `@exclude` to former exclude entries.
    async loadPersistedConfig(): Promise<void> {
        const saved = await loadJson<any>(configPath, null);
        if (!saved || !saved.presets) return;

        this.preset = saved.preset ?? DEFAULT_PRESET_NAME;

        // Migrate legacy format: { include: string[], exclude: string[] } → string[]
        const presets: Record<string, string[]> = {};
        for (const [name, value] of Object.entries(saved.presets)) {
            if (Array.isArray(value)) {
                presets[name] = value as string[];
            } else if (value && typeof value === "object" && "include" in value) {
                const legacy = value as { include: string[]; exclude: string[] };
                presets[name] = [
                    ...legacy.include,
                    ...legacy.exclude.map((e: string) => `@exclude ${e}`),
                ];
                log.info(`migrated legacy preset "${name}" to flat format`);
            }
        }

        this.presets = presets;
        log.info(`loaded config from disk: preset="${this.preset}", ${Object.keys(this.presets).length} preset(s)`);
    }

    /// Persist the current preset config to disk.
    private async persist(): Promise<void> {
        await saveJson(configPath, { preset: this.preset, presets: this.presets });
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
        log.info(`foreground: *** (${info.executable_path})`);

        const result = this.shouldCapture(info.executable_path, info.title);
        if (!result) return;

        // Already capturing this window.
        if (hwndStr === this.lastCaptureHwnd) return;

        // ── Switch capture ───────────────────────────────────────────────
        // replaceStream is idempotent — creates the "main" stream if it
        // doesn't exist, or kills the old process + bumps generation.
        proc.replaceStream(STREAM_ID, hwndStr, DEFAULT_WIDTH, DEFAULT_HEIGHT);
        this.lastCaptureHwnd = hwndStr;
        this.lastCaptureTitle = info.title;
        setComputed("$captureWindowTitle", info.title);

        // Push or clear the live mode based on the matched include pattern's @mode tag.
        if (result.mode) {
            setComputed("$liveMode", result.mode);
        } else {
            clearComputed("$liveMode");
        }

        streamLog.info(`capturing ${hwndStr}`);
    }

    /// Determines whether a window qualifies for capture based on the
    /// active preset's pattern list.  Returns null if the window should
    /// not be captured.  Otherwise returns the mode tag from the first
    /// matching include entry (which may itself be null if the pattern
    /// has no `@mode` prefix).  `@exclude` entries veto the match.
    private shouldCapture(executablePath: string, title: string): { mode: string | null } | null {
        const patterns = this.preset ? this.presets[this.preset] : null;
        if (!patterns) return null;

        // First matching include entry wins (determines the mode tag).
        let matchedMode: string | null = null;
        let included = false;

        for (const raw of patterns) {
            const parsed = parsePattern(raw);
            if (parsed.mode === "exclude") {
                // Exclude entries use case-insensitive exe-path matching.
                if (matchesParsed(parsed, executablePath, title, true)) return null;
            } else if (!included) {
                if (matchesParsed(parsed, executablePath, title, false)) {
                    included = true;
                    matchedMode = parsed.mode;
                }
            }
        }

        return included ? { mode: matchedMode } : null;
    }
}

// ── Pattern matching ─────────────────────────────────────────────────────────

/// A parsed config pattern with optional mode tag and title filter.
interface ParsedPattern {
    /// Optional mode tag from a leading `@mode ` prefix (e.g. `@code devenv.exe`).
    /// null when no mode prefix is present.
    mode: string | null;
    exePath: string;
    /// null when no `@` separator is present (plain exe-path-only pattern).
    title: string | null;
}

/// Parse a config string with optional `@mode ` prefix and `<exePath>@<windowTitle>` body.
///
/// A leading `@word<space>` is recognized as a mode tag only when the pattern
/// starts with `@`, followed by a non-empty word, followed by a space.  This is
/// unambiguous with the existing `@<windowTitle>` syntax (which has no space).
///
/// After stripping the mode prefix (if any), the remainder is split on the
/// first `@` into exe-path and window-title parts.
function parsePattern(pattern: string): ParsedPattern {
    let mode: string | null = null;
    let body = pattern;

    // Extract leading `@mode ` prefix (distinguished from `@title` by the space).
    if (body.startsWith("@")) {
        const spaceIdx = body.indexOf(" ");
        if (spaceIdx > 1) {
            mode = body.slice(1, spaceIdx);
            body = body.slice(spaceIdx + 1);
        }
    }

    const idx = body.indexOf("@");
    if (idx === -1) return { mode, exePath: body, title: null };
    return { mode, exePath: body.slice(0, idx), title: body.slice(idx + 1) };
}

/// Test whether a window (exe path + title) matches a pre-parsed pattern.
///
/// - `caseInsensitive` controls exe-path comparison; title comparison is
///   always case-insensitive regardless of this flag.
/// - Both the exe-path part and the title part (when present and non-empty)
///   must match (AND semantics).
function matchesParsed(
    parsed: ParsedPattern,
    executablePath: string,
    windowTitle: string,
    caseInsensitive: boolean): boolean {
    // Check exe-path part (skip if empty, e.g. "@SomeTitle").
    // Normalize path separators so `/` and `\` are interchangeable.
    if (parsed.exePath.length > 0) {
        let haystack = executablePath.replaceAll("\\", "/");
        let needle = parsed.exePath.replaceAll("\\", "/");
        if (caseInsensitive) { haystack = haystack.toLowerCase(); needle = needle.toLowerCase(); }
        if (!haystack.includes(needle)) return false;
    }

    // Check title part (skip if absent or empty, e.g. "foo.exe" or "foo.exe@").
    if (parsed.title !== null && parsed.title.length > 0) {
        if (!windowTitle.toLowerCase().includes(parsed.title.toLowerCase())) return false;
    }

    return true;
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
        log.error(`failed to get foreground window: ${e}`);
        return null;
    }
}

/// Format a numeric hwnd as a 0x hex string, matching the format used by
/// the process manager and API.
function formatHwnd(hwnd: number): string {
    return `0x${hwnd.toString(16).toUpperCase()}`;
}

// ── Singleton ────────────────────────────────────────────────────────────────

export const selector = new LiveWindowSelector();
await selector.loadPersistedConfig();
