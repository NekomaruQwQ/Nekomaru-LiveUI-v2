//! Centralized constants and well-known values for `live-server`.

use crate::video::process::CropParams;

// ── Data Paths ──────────────────────────────────────────────────────────────

/// Root directory for persisted runtime data (gitignored).
pub const DATA_DIR: &str = "data";

/// Selector preset config filename (inside `DATA_DIR`).
pub const SELECTOR_CONFIG_FILENAME: &str = "selector-config.json";

// ── Well-Known Stream IDs ───────────────────────────────────────────────────

/// Stream ID managed by the auto-selector (foreground window).
pub const STREAM_ID_MAIN: &str = "main";

/// Stream ID managed by the YouTube Music manager.
pub const STREAM_ID_YTM: &str = "youtube-music";

// ── Computed String IDs ─────────────────────────────────────────────────────

/// Human-readable label for the window being captured on the "main" stream.
/// Prefers the executable's FileDescription (from PE version info); falls back
/// to the window title when version info is unavailable.
pub const CSID_CAPTURE_INFO: &str = "$captureInfo";

/// Current capture mode — `"auto"` when the selector is active.
pub const CSID_CAPTURE_MODE: &str = "$captureMode";

/// Live mode derived from the matched pattern's `@mode` tag (e.g. `"code"`).
pub const CSID_LIVE_MODE: &str = "$liveMode";

/// Revision timestamp of the `@-` jj revision, read at server startup.
pub const CSID_TIMESTAMP: &str = "$timestamp";

// ── Capture Defaults ────────────────────────────────────────────────────────

/// Default capture resolution (width).
pub const DEFAULT_CAPTURE_WIDTH: u32 = 1920;

/// Default capture resolution (height).
pub const DEFAULT_CAPTURE_HEIGHT: u32 = 1200;

// ── Buffer Capacities ───────────────────────────────────────────────────────

/// Video frame buffer capacity (~1 second at 60fps).
pub const FRAME_BUFFER_CAPACITY: usize = 60;

/// Audio chunk buffer capacity (~1 second at 10ms/chunk).
pub const AUDIO_BUFFER_CAPACITY: usize = 100;

// ── Poll / Timer Intervals ──────────────────────────────────────────────────

/// Auto-selector foreground window poll interval (milliseconds).
pub const SELECTOR_POLL_INTERVAL_MS: u64 = 2000;

/// YouTube Music window poll interval (milliseconds).
pub const YTM_POLL_INTERVAL_MS: u64 = 5000;

/// Batch interval passed to `live-kpm.exe` (milliseconds).
pub const KPM_BATCH_INTERVAL_MS: u64 = 50;

/// Sliding window duration for KPM calculation (milliseconds).
pub const KPM_WINDOW_DURATION_MS: u64 = 5000;

// ── YouTube Music ───────────────────────────────────────────────────────────

/// Expected YouTube Music window title.
pub const YOUTUBE_MUSIC_WINDOW_TITLE: &str = "YouTube Music - Nekomaru LiveUI v2";

/// Compute the crop box for the YouTube Music playback bar.
///
/// Layout (from bottom of client area, measured from full window including
/// title bar):
/// - Title bar: 48px
/// - Playback bar: 112px tall
/// - Bottom margin: 12px below bar
/// - Right margin: 96px trimmed from right edge
///
/// Returns `None` when the window is too small for a meaningful crop box.
pub const fn get_youtube_music_crop_geometry(window_width: u32, window_height: u32) -> Option<CropParams> {
    let title_bar = 48u32;
    let bar_height = 112u32;
    let bottom_margin = 12u32;
    let right_margin = 96u32;

    let full_height = window_height + title_bar;
    let min_y = full_height.saturating_sub(bar_height + bottom_margin);
    let max_y = full_height.saturating_sub(bottom_margin);
    let max_x = window_width.saturating_sub(right_margin);

    if max_x == 0 || max_y <= min_y { return None; }
    Some(CropParams { min_x: 0, min_y, max_x, max_y })
}

// ── Default Selector Config ─────────────────────────────────────────────────

/// Default selector config, used when `data/selector-config.json` is missing.
pub fn default_selector_config() -> serde_json::Value {
    serde_json::json!({
        "preset": "main",
        "presets": {
            "main": [
                "@code devenv.exe",
                "@code C:/Program Files/Microsoft Visual Studio Code/Code.exe",
                "@code C:/Program Files/JetBrains/",
                "@game D:/7-Games/",
                "@game D:/7-Games.Steam/steamapps/common/",
                "@game E:/Nekomaru-Games/",
                "@game E:/SteamLibrary/steamapps/common/",
                "@exclude gogh.exe",
                "@exclude vtube studio.exe"
            ]
        }
    })
}
