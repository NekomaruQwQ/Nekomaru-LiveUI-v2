//! Top-level shared application state.
//!
//! Wrapped in `Arc<AppState>` and passed to all Axum handlers via the `State`
//! extractor.  Each subsystem owns its state behind a `tokio::sync::RwLock`.

use crate::audio::process::AudioState;
use crate::kpm::process::KpmState;
use crate::selector::manager::SelectorState;
use crate::strings::store::StringStore;
use crate::video::process::StreamRegistry;
use crate::youtube_music::manager::YtmState;

use std::sync::Arc;

use job_object::JobObject;
use tokio::sync::RwLock;

/// Shared state for the entire server.
pub struct AppState {
    strings: Arc<RwLock<StringStore>>,
    streams: Arc<RwLock<StreamRegistry>>,
    audio: Arc<RwLock<AudioState>>,
    kpm: Arc<RwLock<KpmState>>,
    selector: Arc<RwLock<SelectorState>>,
    ytm: Arc<RwLock<YtmState>>,
    #[expect(dead_code, reason = "kept alive for RAII — dropping kills all assigned children")]
    job: Arc<JobObject>,
}

impl AppState {
    pub fn new(video_exe_path: String, job: Arc<JobObject>) -> Self {
        Self {
            strings: Arc::new(RwLock::new(StringStore::new())),
            streams: Arc::new(RwLock::new(StreamRegistry::new(video_exe_path, Arc::clone(&job)))),
            audio: Arc::new(RwLock::new(AudioState::new())),
            kpm: Arc::new(RwLock::new(KpmState::new())),
            selector: Arc::new(RwLock::new(SelectorState::new())),
            ytm: Arc::new(RwLock::new(YtmState::new())),
            job,
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

    // ── Shutdown ──────────────────────────────────────────────────────────

    /// Gracefully stop all subsystems.  Called once on Ctrl+C.
    ///
    /// Order matters: stop managers first (they create/replace streams),
    /// then capture processes, then destroy remaining streams.
    pub async fn shutdown(&self) {
        log::info!("shutting down...");

        // 1. Stop polling managers so they don't spawn new streams.
        {
            let streams_arc = self.streams_arc();
            let strings_arc = self.strings_arc();
            self.selector.write().await.stop(&streams_arc, &strings_arc);
        }
        {
            let streams_arc = self.streams_arc();
            self.ytm.write().await.stop(&streams_arc);
        }

        // 2. Stop audio and KPM capture processes.
        self.audio.write().await.stop();
        self.kpm.write().await.stop();

        // 3. Destroy all video streams (kills child processes).
        self.streams.write().await.destroy_all();

        log::info!("all subsystems stopped");
    }
}
