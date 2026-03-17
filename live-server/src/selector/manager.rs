//! Auto window selector manager.
//!
//! Polls the foreground window every 2 seconds via `enumerate_windows::
//! get_foreground_window()` (direct library call — no process spawn).
//! When the foreground matches the active preset and differs from the
//! current capture target, replaces the "main" stream in-place.

use crate::constant::{
    CSID_CAPTURE_MODE, CSID_CAPTURE_WINDOW_TITLE, CSID_LIVE_MODE,
    DEFAULT_CAPTURE_HEIGHT, DEFAULT_CAPTURE_WIDTH,
    SELECTOR_POLL_INTERVAL_MS, STREAM_ID_MAIN,
};
use crate::selector::config::{PresetConfig, should_capture};
use crate::strings::store::StringStore;
use crate::video::process::StreamRegistry;

use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

// ── Selector State ───────────────────────────────────────────────────────────

pub struct SelectorState {
    pub active: bool,
    pub last_capture_hwnd: Option<String>,
    pub last_capture_title: Option<String>,
    pub config: PresetConfig,
    pub poll_handle: Option<JoinHandle<()>>,
}

/// Serializable status for the API.
#[derive(serde::Serialize)]
pub struct SelectorStatus {
    pub active: bool,
    #[serde(rename = "currentStreamId")]
    pub current_stream_id: Option<String>,
    #[serde(rename = "currentHwnd")]
    pub current_hwnd: Option<String>,
    #[serde(rename = "currentTitle")]
    pub current_title: Option<String>,
}

impl SelectorState {
    pub fn new() -> Self {
        let config = PresetConfig::load();
        Self {
            active: false,
            last_capture_hwnd: None,
            last_capture_title: None,
            config,
            poll_handle: None,
        }
    }

    pub fn status(&self) -> SelectorStatus {
        SelectorStatus {
            active: self.active,
            current_stream_id: self.active.then(|| STREAM_ID_MAIN.into()),
            current_hwnd: self.last_capture_hwnd.clone(),
            current_title: self.last_capture_title.clone(),
        }
    }

    /// Start polling.  Requires shared references to the stream registry and
    /// string store (for computed string updates).
    pub fn start(
        &mut self,
        selector_arc: &Arc<RwLock<Self>>,
        streams_arc: &Arc<RwLock<StreamRegistry>>,
        strings_arc: &Arc<RwLock<StringStore>>,
    ) {
        if self.active { return; }
        self.active = true;

        // Set $captureMode computed string.
        {
            let strings = Arc::clone(strings_arc);
            tokio::spawn(async move {
                strings.write().await.set_computed(CSID_CAPTURE_MODE, "auto".into());
            });
        }

        let selector = Arc::clone(selector_arc);
        let streams = Arc::clone(streams_arc);
        let strings = Arc::clone(strings_arc);

        self.poll_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_millis(SELECTOR_POLL_INTERVAL_MS));

            loop {
                interval.tick().await;
                poll_once(&selector, &streams, &strings).await;
            }
        }));

        log::info!("started");
    }

    /// Stop polling and destroy the managed stream.
    pub fn stop(
        &mut self,
        streams_arc: &Arc<RwLock<StreamRegistry>>,
        strings_arc: &Arc<RwLock<StringStore>>,
    ) {
        if !self.active { return; }

        if let Some(handle) = self.poll_handle.take() {
            handle.abort();
        }

        self.active = false;
        self.last_capture_hwnd = None;
        self.last_capture_title = None;

        // Destroy the managed stream.
        let streams = Arc::clone(streams_arc);
        let strings = Arc::clone(strings_arc);
        tokio::spawn(async move {
            streams.write().await.destroy_stream(STREAM_ID_MAIN);
            let mut s = strings.write().await;
            s.clear_computed(CSID_CAPTURE_WINDOW_TITLE);
            s.clear_computed(CSID_CAPTURE_MODE);
            s.clear_computed(CSID_LIVE_MODE);
        });

        log::info!("stopped");
    }

    /// Reload config from disk.
    pub fn reload_config(&mut self) {
        self.config = PresetConfig::load();
    }
}

impl Drop for SelectorState {
    fn drop(&mut self) {
        if let Some(handle) = self.poll_handle.take() {
            handle.abort();
        }
    }
}

// ── Poll Logic ───────────────────────────────────────────────────────────────

/// One poll iteration: get foreground window, match patterns, replace stream.
async fn poll_once(
    selector_arc: &Arc<RwLock<SelectorState>>,
    streams_arc: &Arc<RwLock<StreamRegistry>>,
    strings_arc: &Arc<RwLock<StringStore>>,
) {
    // Get foreground window on a blocking thread (Win32 calls).
    let info = tokio::task::spawn_blocking(enumerate_windows::get_foreground_window)
        .await
        .ok()
        .flatten();

    let Some(info) = info else { return };

    let hwnd_str = format_hwnd(info.hwnd);

    let mut selector = selector_arc.write().await;

    // Check if foreground hasn't changed.
    if selector.last_capture_hwnd.as_deref() == Some(&hwnd_str) {
        // Title might have changed on the same window.
        if selector.last_capture_title.as_deref() != Some(&info.title) {
            selector.last_capture_title = Some(info.title.clone());
            strings_arc.write().await
                .set_computed(CSID_CAPTURE_WINDOW_TITLE, info.title);
        }
        return;
    }

    // Get the active patterns.
    let patterns = selector.config.presets
        .get(&selector.config.preset)
        .cloned();
    let Some(patterns) = patterns else { return };

    let exe_path = info.executable_path.to_string_lossy().to_string();
    let Some(capture_match) = should_capture(&patterns, &exe_path, &info.title) else { return };

    // Already capturing this window.
    if selector.last_capture_hwnd.as_deref() == Some(&hwnd_str) { return; }

    // Switch capture.
    {
        let mut streams = streams_arc.write().await;
        streams.replace_stream(STREAM_ID_MAIN, &hwnd_str, DEFAULT_CAPTURE_WIDTH, DEFAULT_CAPTURE_HEIGHT, streams_arc);
    }

    selector.last_capture_hwnd = Some(hwnd_str.clone());
    selector.last_capture_title = Some(info.title.clone());
    drop(selector);

    // Update computed strings.
    {
        let mut strings = strings_arc.write().await;
        strings.set_computed(CSID_CAPTURE_WINDOW_TITLE, info.title);
        match capture_match.mode {
            Some(m) => strings.set_computed(CSID_LIVE_MODE, m),
            None => strings.clear_computed(CSID_LIVE_MODE),
        }
    }

    log::info!("capturing {hwnd_str}");
}

fn format_hwnd(hwnd: usize) -> String {
    format!("0x{hwnd:X}")
}
