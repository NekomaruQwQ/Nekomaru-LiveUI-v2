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
    strings_inner: Arc<RwLock<StringStore>>,
    streams_inner: Arc<RwLock<StreamRegistry>>,
    audio_inner: Arc<RwLock<AudioState>>,
    kpm_inner: Arc<RwLock<KpmState>>,
    selector_inner: Arc<RwLock<SelectorState>>,
    ytm_inner: Arc<RwLock<YtmState>>,
}

impl AppState {
    pub fn new(video_exe_path: String) -> Self {
        Self {
            strings_inner: Arc::new(RwLock::new(StringStore::new())),
            streams_inner: Arc::new(RwLock::new(StreamRegistry::new(video_exe_path))),
            audio_inner: Arc::new(RwLock::new(AudioState::new())),
            kpm_inner: Arc::new(RwLock::new(KpmState::new())),
            selector_inner: Arc::new(RwLock::new(SelectorState::new())),
            ytm_inner: Arc::new(RwLock::new(YtmState::new())),
        }
    }

    // ── Strings ──────────────────────────────────────────────────────────

    pub async fn strings(&self) -> tokio::sync::RwLockReadGuard<'_, StringStore> {
        self.strings_inner.read().await
    }

    pub async fn strings_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, StringStore> {
        self.strings_inner.write().await
    }

    pub fn strings_arc(&self) -> Arc<RwLock<StringStore>> {
        self.strings_inner.clone()
    }

    // ── Streams ──────────────────────────────────────────────────────────

    pub async fn streams(&self) -> tokio::sync::RwLockReadGuard<'_, StreamRegistry> {
        self.streams_inner.read().await
    }

    pub async fn streams_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, StreamRegistry> {
        self.streams_inner.write().await
    }

    pub fn streams_arc(&self) -> Arc<RwLock<StreamRegistry>> {
        self.streams_inner.clone()
    }

    // ── Audio ────────────────────────────────────────────────────────────

    pub async fn audio(&self) -> tokio::sync::RwLockReadGuard<'_, AudioState> {
        self.audio_inner.read().await
    }

    pub async fn audio_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, AudioState> {
        self.audio_inner.write().await
    }

    pub fn audio_arc(&self) -> Arc<RwLock<AudioState>> {
        self.audio_inner.clone()
    }

    // ── KPM ──────────────────────────────────────────────────────────────

    pub async fn kpm(&self) -> tokio::sync::RwLockReadGuard<'_, KpmState> {
        self.kpm_inner.read().await
    }

    pub async fn kpm_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, KpmState> {
        self.kpm_inner.write().await
    }

    pub fn kpm_arc(&self) -> Arc<RwLock<KpmState>> {
        self.kpm_inner.clone()
    }

    // ── Selector ─────────────────────────────────────────────────────────

    pub async fn selector(&self) -> tokio::sync::RwLockReadGuard<'_, SelectorState> {
        self.selector_inner.read().await
    }

    pub async fn selector_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, SelectorState> {
        self.selector_inner.write().await
    }

    pub fn selector_arc(&self) -> Arc<RwLock<SelectorState>> {
        self.selector_inner.clone()
    }

    // ── YTM ──────────────────────────────────────────────────────────────

    pub async fn ytm_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, YtmState> {
        self.ytm_inner.write().await
    }

    pub fn ytm_arc(&self) -> Arc<RwLock<YtmState>> {
        self.ytm_inner.clone()
    }
}
