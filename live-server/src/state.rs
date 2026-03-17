//! Top-level shared application state.
//!
//! Wrapped in `Arc<AppState>` and passed to all Axum handlers via the `State`
//! extractor.  Each subsystem owns its state behind a `tokio::sync::RwLock`.

use crate::audio::process::AudioState;
use crate::kpm::process::KpmState;
use crate::selector::manager::SelectorState;
use crate::strings::store::StringStore;
use crate::video::process::StreamRegistry;
use crate::ytm::manager::YtmState;

use std::sync::Arc;

use tokio::sync::RwLock;

/// Shared state for the entire server.
pub struct AppState {
    strings: Arc<RwLock<StringStore>>,
    streams: Arc<RwLock<StreamRegistry>>,
    audio: Arc<RwLock<AudioState>>,
    kpm: Arc<RwLock<KpmState>>,
    selector: Arc<RwLock<SelectorState>>,
    ytm: Arc<RwLock<YtmState>>,
}

impl AppState {
    pub fn new(video_exe_path: String) -> Self {
        Self {
            strings: Arc::new(RwLock::new(StringStore::new())),
            streams: Arc::new(RwLock::new(StreamRegistry::new(video_exe_path))),
            audio: Arc::new(RwLock::new(AudioState::new())),
            kpm: Arc::new(RwLock::new(KpmState::new())),
            selector: Arc::new(RwLock::new(SelectorState::new())),
            ytm: Arc::new(RwLock::new(YtmState::new())),
        }
    }

    // ── Strings ──────────────────────────────────────────────────────────

    pub async fn strings(&self) -> tokio::sync::RwLockReadGuard<'_, StringStore> {
        self.strings.read().await
    }

    pub async fn strings_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, StringStore> {
        self.strings.write().await
    }

    pub fn strings_arc(&self) -> Arc<RwLock<StringStore>> {
        Arc::clone(&self.strings)
    }

    // ── Streams ──────────────────────────────────────────────────────────

    pub async fn streams(&self) -> tokio::sync::RwLockReadGuard<'_, StreamRegistry> {
        self.streams.read().await
    }

    pub async fn streams_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, StreamRegistry> {
        self.streams.write().await
    }

    pub fn streams_arc(&self) -> Arc<RwLock<StreamRegistry>> {
        Arc::clone(&self.streams)
    }

    // ── Audio ────────────────────────────────────────────────────────────

    pub async fn audio(&self) -> tokio::sync::RwLockReadGuard<'_, AudioState> {
        self.audio.read().await
    }

    pub async fn audio_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, AudioState> {
        self.audio.write().await
    }

    pub fn audio_arc(&self) -> Arc<RwLock<AudioState>> {
        Arc::clone(&self.audio)
    }

    // ── KPM ──────────────────────────────────────────────────────────────

    pub async fn kpm(&self) -> tokio::sync::RwLockReadGuard<'_, KpmState> {
        self.kpm.read().await
    }

    pub async fn kpm_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, KpmState> {
        self.kpm.write().await
    }

    pub fn kpm_arc(&self) -> Arc<RwLock<KpmState>> {
        Arc::clone(&self.kpm)
    }

    // ── Selector ─────────────────────────────────────────────────────────

    pub async fn selector(&self) -> tokio::sync::RwLockReadGuard<'_, SelectorState> {
        self.selector.read().await
    }

    pub async fn selector_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, SelectorState> {
        self.selector.write().await
    }

    pub fn selector_arc(&self) -> Arc<RwLock<SelectorState>> {
        Arc::clone(&self.selector)
    }

    // ── YTM ──────────────────────────────────────────────────────────────

    pub async fn ytm_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, YtmState> {
        self.ytm.write().await
    }

    #[expect(dead_code, reason = "API completeness — no YTM routes yet")]
    pub fn ytm_arc(&self) -> Arc<RwLock<YtmState>> {
        Arc::clone(&self.ytm)
    }
}
