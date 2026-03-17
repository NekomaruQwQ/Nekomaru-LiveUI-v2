//! YouTube Music stream manager.
//!
//! Polls `enumerate_windows::enumerate_windows()` every 5 seconds looking for
//! a window titled "YouTube Music - Nekomaru LiveUI v2".  When found, creates
//! (or replaces) a crop-mode stream capturing the bottom playback bar.  When
//! the window disappears, the stream is destroyed.

use crate::constant::{STREAM_ID_YTM, YTM_POLL_INTERVAL_MS, YTM_TITLE};
use crate::video::process::StreamRegistry;

use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

// ── YTM State ────────────────────────────────────────────────────────────────

pub struct YtmState {
    pub active: bool,
    pub last_known_hwnd: Option<String>,
    pub poll_handle: Option<JoinHandle<()>>,
}

impl YtmState {
    pub const fn new() -> Self {
        Self { active: false, last_known_hwnd: None, poll_handle: None }
    }

    pub fn start(&mut self, streams_arc: &Arc<RwLock<StreamRegistry>>) {
        if self.active { return; }
        self.active = true;

        let streams = Arc::clone(streams_arc);

        // Shared hwnd state between poll iterations.
        let last_hwnd: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let last_hwnd_clone = last_hwnd;

        self.poll_handle = Some(tokio::spawn(async move {
            // Immediate first poll.
            poll_once(&streams, &last_hwnd_clone).await;

            let mut interval = tokio::time::interval(
                std::time::Duration::from_millis(YTM_POLL_INTERVAL_MS));

            loop {
                interval.tick().await;
                poll_once(&streams, &last_hwnd_clone).await;
            }
        }));

        log::info!("[ytm] started");
    }

    /// Stop polling and destroy the managed YouTube Music stream.
    pub fn stop(&mut self, streams_arc: &Arc<RwLock<StreamRegistry>>) {
        if !self.active { return; }

        if let Some(handle) = self.poll_handle.take() {
            handle.abort();
        }

        self.active = false;
        self.last_known_hwnd = None;

        let streams = Arc::clone(streams_arc);
        tokio::spawn(async move {
            streams.write().await.destroy_stream(STREAM_ID_YTM);
        });

        log::info!("[ytm] stopped");
    }
}

impl Drop for YtmState {
    fn drop(&mut self) {
        if let Some(handle) = self.poll_handle.take() {
            handle.abort();
        }
    }
}

// ── Poll Logic ───────────────────────────────────────────────────────────────

async fn poll_once(
    streams_arc: &Arc<RwLock<StreamRegistry>>,
    last_hwnd: &Arc<RwLock<Option<String>>>,
) {
    let windows = tokio::task::spawn_blocking(enumerate_windows::enumerate_windows)
        .await
        .unwrap_or_default();

    let ytm = windows.iter().find(|w| w.title == YTM_TITLE);

    if let Some(ytm) = ytm {
        let hwnd_str = format!("0x{:X}", ytm.hwnd);

        let current = last_hwnd.read().await;
        if current.as_deref() == Some(&hwnd_str) { return; }
        drop(current);

        log::info!("[ytm] window detected: {hwnd_str} ({}x{})", ytm.width, ytm.height);

        let Some(crop) = crate::constant::ytm_crop_geometry(ytm.width, ytm.height)
            else { return; };

        {
            let mut streams = streams_arc.write().await;
            streams.replace_crop_stream(
                STREAM_ID_YTM, &hwnd_str, &crop, Some(2), streams_arc);
        }

        *last_hwnd.write().await = Some(hwnd_str);
    } else {
        let current = last_hwnd.read().await;
        if current.is_some() {
            drop(current);
            streams_arc.write().await.destroy_stream(STREAM_ID_YTM);
            *last_hwnd.write().await = None;
            log::info!("[ytm] window disappeared, stream destroyed");
        }
    }
}
