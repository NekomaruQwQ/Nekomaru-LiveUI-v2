//! `live-server` — Rust HTTP server for Nekomaru LiveUI.
//!
//! Replaces the TypeScript Hono server.  Manages video/audio capture processes,
//! protocol parsing, frame buffering, auto-selector, YouTube Music manager,
//! string store, and all HTTP API endpoints.
//!
//! ## Usage
//!
//! ```text
//! LIVE_CORE_PORT=3000 LIVE_VITE_PORT=5173 live-server
//! ```

mod constant;
mod state;
mod vite_proxy;
mod windows;

mod audio {
    pub mod buffer;
    pub mod process;
    pub mod routes;
    pub mod ws;
}
mod kpm {
    pub mod calculator;
    pub mod process;
    pub mod ws;
}

mod selector {
    pub mod config;
    pub mod manager;
    pub mod routes;
}

mod strings {
    pub mod routes;
    pub mod store;
}

mod video {
    pub mod buffer;
    pub mod process;
    pub mod routes;
    pub mod ws;
}

mod youtube_music {
    pub mod manager;
}

use state::AppState;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use clap::Parser;
use job_object::JobObject;

use std::process::Child;
use std::sync::Arc;

// ── CLI ─────────────────────────────────────────────────────────────────────

/// Nekomaru LiveUI server.
#[derive(Parser)]
#[command(name = "live-server")]
struct Cli {
    /// HTTP server port.  Required — reads from LIVE_CORE_PORT env if not
    /// passed as a flag.
    #[arg(long, env = "LIVE_CORE_PORT")]
    core_port: u16,

    /// Vite dev server port.  When set, spawns `bunx vite` as a child process
    /// and proxies non-API requests to it for dev assets / HMR.
    #[arg(long, env = "LIVE_VITE_PORT")]
    vite_port: Option<u16>,

    /// Enable audio capture.  Off by default to avoid feedback loops
    /// during localhost development.
    #[arg(long, env = "LIVE_AUDIO")]
    audio: bool,

    /// WASAPI capture device name for audio.
    #[arg(long, default_value = "Loopback L + R (Focusrite USB Audio)")]
    audio_device: String,
}

// ── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let _ = set_dpi_awareness::per_monitor_v2();

    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let cli = Cli::parse();

    // Resolve exe paths: if relative, look next to this binary.
    let video_exe = resolve_sibling_exe("live-video.exe");
    let audio_exe = resolve_sibling_exe("live-audio.exe");
    let kpm_exe = resolve_sibling_exe("live-kpm.exe");
    log::info!("video exe: {video_exe}");
    log::info!("audio exe: {audio_exe}");
    log::info!("kpm exe: {kpm_exe}");

    // Job object: all child processes assigned to it are killed when the
    // server exits — even on crash or Task Manager kill.
    let job = Arc::new(
        JobObject::new().expect("failed to create job object"));

    let state = Arc::new(AppState::new(video_exe, Arc::clone(&job)));

    // Start audio capture if enabled.
    if cli.audio {
        let audio_arc = state.audio_arc();
        state.audio_mut().await.start(&audio_exe, &cli.audio_device, &job, &audio_arc);
    }

    // Start KPM capture (always enabled).
    {
        let kpm_arc = state.kpm_arc();
        state.kpm_mut().await.start(&kpm_exe, &job, &kpm_arc);
    }

    // Start auto-selector and YouTube Music manager.
    {
        let selector_arc = state.selector_arc();
        let streams_arc = state.streams_arc();
        let strings_arc = state.strings_arc();
        state.selector_mut().await.start(&selector_arc, &streams_arc, &strings_arc);
    }
    {
        let streams_arc = state.streams_arc();
        state.ytm_mut().await.start(&streams_arc);
    }

    let mut app = Router::new()
        // HTTP routes.
        .merge(audio::routes::router())
        .merge(selector::routes::router())
        .merge(strings::routes::router())
        .merge(video::routes::router())
        .merge(windows::router())
        .route("/api/v1/refresh", post(refresh))
        // WebSocket routes.
        .merge(audio::ws::router())
        .merge(kpm::ws::router())
        .merge(video::ws::router())
        .with_state(Arc::clone(&state));

    // Fallback: dev → proxy to Vite, prod → serve dist/.
    if let Some(vp) = cli.vite_port {
        app = app.fallback(vite_proxy::fallback(vp));
    }

    let addr = format!("0.0.0.0:{}", cli.core_port);
    log::info!("listening on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind");

    // Spawn Vite dev server as a child process if LIVE_VITE_PORT is set.
    let mut vite_child = cli.vite_port
        .and_then(|vp| spawn_vite(vp, cli.core_port, &job));

    // Serve until Ctrl+C.
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            log::info!("Ctrl+C received");
        })
        .await
        .expect("server error");

    // Server has stopped accepting connections — clean up all subsystems.
    state.shutdown().await;

    if let Some(ref mut child) = vite_child {
        let _ = child.kill();
        let _ = child.wait();
        log::info!("vite stopped");
    }

    // `job` drops here, killing any straggler child processes.
}

// ── Refresh ─────────────────────────────────────────────────────────────────

/// `POST /api/v1/refresh` — reload string store and selector config from disk.
async fn refresh(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    state.strings_mut().await.reload();
    state.selector_mut().await.reload_config();
    Json(serde_json::json!({ "ok": true }))
}

// ── Exe Resolution ──────────────────────────────────────────────────────────

/// Resolve an executable name to a full path.  If the name is a bare filename
/// (no directory separators), look for it next to the current binary (the
/// `target/debug/` or `target/release/` directory from `cargo build`).
fn resolve_sibling_exe(name: &str) -> String {
    let path = std::path::Path::new(name);
    if path.parent().is_some_and(|p| p != std::path::Path::new("")) {
        // Already has a directory component — use as-is.
        return name.to_owned();
    }

    // Bare name: look next to this binary.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            let candidate = dir.join(name);
            // On Windows, try with .exe suffix.
            let with_ext = candidate.with_extension("exe");
            if with_ext.exists() {
                return with_ext.to_string_lossy().into_owned();
            }
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }

    // Fallback: hope it's on PATH.
    name.to_owned()
}

// ── Vite Dev Server ─────────────────────────────────────────────────────────

/// Spawn `bunx vite` as a child process.  The core server proxies non-API
/// requests to Vite for dev assets; Vite no longer proxies back to us.
///
/// Returns the child process handle for explicit cleanup on shutdown.
/// The child is also assigned to the job object so it dies on crash.
fn spawn_vite(vite_port: u16, _core_port: u16, job: &JobObject) -> Option<Child> {
    let frontend_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent()?.parent()?.parent().map(|d| d.join("frontend")))
        .unwrap_or_else(|| std::path::PathBuf::from("frontend"));

    log::info!("spawning vite dev server on port {vite_port} (frontend dir: {})", frontend_dir.display());

    let child = std::process::Command::new("bunx")
        .arg("--bun")
        .arg("vite")
        .current_dir(&frontend_dir)
        .env("LIVE_VITE_PORT", vite_port.to_string())
        .spawn();

    match child {
        Ok(child) => {
            if let Err(e) = job.assign(&child) {
                log::warn!("failed to assign vite to job object: {e}");
            }
            Some(child)
        }
        Err(e) => {
            log::error!("failed to spawn vite: {e}");
            None
        }
    }
}
