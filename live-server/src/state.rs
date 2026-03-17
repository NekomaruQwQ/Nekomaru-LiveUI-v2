//! Top-level shared application state.
//!
//! Wrapped in `Arc<AppState>` and passed to all Axum handlers via the `State`
//! extractor.  Each subsystem (video, audio, KPM, selector, strings) owns its
//! state behind a `tokio::sync::RwLock` for concurrent read-heavy access.

use crate::audio::process::AudioState;
use crate::kpm::process::KpmState;
use crate::strings::store::StringStore;
use crate::video::process::StreamRegistry;

use std::sync::Arc;

use tokio::sync::RwLock;

/// Shared state for the entire server.
pub struct AppState {
    pub strings: RwLock<StringStore>,
    streams_inner: Arc<RwLock<StreamRegistry>>,
    audio_inner: Arc<RwLock<AudioState>>,
    kpm_inner: Arc<RwLock<KpmState>>,
}

impl AppState {
    pub fn new(video_exe_path: String) -> Self {
        Self {
            strings: RwLock::new(StringStore::new()),
            streams_inner: Arc::new(RwLock::new(StreamRegistry::new(video_exe_path))),
            audio_inner: Arc::new(RwLock::new(AudioState::new())),
            kpm_inner: Arc::new(RwLock::new(KpmState::new())),
        }
    }

    pub async fn streams(&self) -> tokio::sync::RwLockReadGuard<'_, StreamRegistry> {
        self.streams_inner.read().await
    }

    pub async fn streams_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, StreamRegistry> {
        self.streams_inner.write().await
    }

    pub fn streams_arc(&self) -> Arc<RwLock<StreamRegistry>> {
        self.streams_inner.clone()
    }

    pub async fn audio(&self) -> tokio::sync::RwLockReadGuard<'_, AudioState> {
        self.audio_inner.read().await
    }

    pub async fn audio_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, AudioState> {
        self.audio_inner.write().await
    }

    pub fn audio_arc(&self) -> Arc<RwLock<AudioState>> {
        self.audio_inner.clone()
    }

    pub async fn kpm(&self) -> tokio::sync::RwLockReadGuard<'_, KpmState> {
        self.kpm_inner.read().await
    }

    pub async fn kpm_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, KpmState> {
        self.kpm_inner.write().await
    }

    pub fn kpm_arc(&self) -> Arc<RwLock<KpmState>> {
        self.kpm_inner.clone()
    }
}
