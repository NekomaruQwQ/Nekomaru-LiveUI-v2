//! `live-server` — Rust HTTP server for Nekomaru LiveUI.
//!
//! Replaces the TypeScript Hono server.  Manages video/audio capture processes,
//! protocol parsing, frame buffering, auto-selector, YouTube Music manager,
//! string store, and all HTTP API endpoints.
//!
//! ## Usage
//!
//! ```text
//! LIVE_CORE_PORT=3000 LIVE_PORT=5173 live-server
//! ```

mod audio;
mod kpm;
mod state;
mod strings;
mod video;
mod windows;

use state::AppState;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use clap::Parser;

use std::sync::Arc;

// ── CLI ─────────────────────────────────────────────────────────────────────

/// Nekomaru LiveUI server.
#[derive(Parser)]
#[command(name = "live-server")]
struct Cli {
    /// HTTP server port.  Required — reads from LIVE_CORE_PORT env if not
    /// passed as a flag.
    #[arg(long, env = "LIVE_CORE_PORT")]
    port: u16,

    /// Vite dev server port.  When set, spawns `bunx vite` as a child process
    /// with this port and a proxy back to the core server.
    #[arg(long, env = "LIVE_PORT")]
    vite_port: Option<u16>,

    /// Path to the `live-video` executable.  Defaults to `live-video` in the
    /// same directory as this binary (from `cargo build`).
    #[arg(long, default_value = "live-video")]
    video_exe: String,

    /// Path to the `live-audio` executable.
    #[arg(long, default_value = "live-audio")]
    audio_exe: String,

    /// WASAPI capture device name for audio.
    #[arg(long, default_value = "Loopback L + R (Focusrite USB Audio)")]
    audio_device: String,

    /// Enable audio capture.  Off by default to avoid feedback loops
    /// during localhost development.
    #[arg(long, env = "LIVE_AUDIO")]
    audio: bool,

    /// Path to the `live-kpm` executable.
    #[arg(long, default_value = "live-kpm")]
    kpm_exe: String,
}

// ── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let cli = Cli::parse();

    // Resolve exe paths: if relative, look next to this binary.
    let video_exe = resolve_sibling_exe(&cli.video_exe);
    let audio_exe = resolve_sibling_exe(&cli.audio_exe);
    let kpm_exe = resolve_sibling_exe(&cli.kpm_exe);
    log::info!("video exe: {video_exe}");
    log::info!("audio exe: {audio_exe}");
    log::info!("kpm exe: {kpm_exe}");

    let state = Arc::new(AppState::new(video_exe));

    // Start audio capture if enabled.
    if cli.audio {
        let audio_arc = state.audio_arc();
        state.audio_mut().await.start(&audio_exe, &cli.audio_device, &audio_arc);
    }

    // Start KPM capture (always enabled).
    {
        let kpm_arc = state.kpm_arc();
        state.kpm_mut().await.start(&kpm_exe, &kpm_arc);
    }

    let app = Router::new()
        .merge(audio::routes::router())
        .merge(kpm::routes::router())
        .merge(strings::routes::router())
        .merge(video::routes::router())
        .merge(windows::router())
        .route("/api/v1/refresh", post(refresh))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", cli.port);
    log::info!("listening on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind");

    // Spawn Vite dev server as a child process if LIVE_PORT is set.
    if let Some(vite_port) = cli.vite_port {
        spawn_vite(vite_port, cli.port);
    }

    axum::serve(listener, app)
        .await
        .expect("server error");
}

// ── Refresh ─────────────────────────────────────────────────────────────────

/// `POST /api/v1/refresh` — reload string store (and later, selector config)
/// from disk.
async fn refresh(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    state.strings.write().await.reload();
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
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
    }

    // Fallback: hope it's on PATH.
    name.to_owned()
}

// ── Vite Dev Server ─────────────────────────────────────────────────────────

/// Spawn `bunx vite` as a child process.  Vite's proxy config (in
/// `frontend/vite.config.ts`) forwards `/api/*` to our Axum server.
fn spawn_vite(vite_port: u16, core_port: u16) {
    let frontend_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent()?.parent()?.parent().map(|d| d.join("frontend")))
        .unwrap_or_else(|| std::path::PathBuf::from("frontend"));

    log::info!("spawning vite dev server on port {vite_port} (frontend dir: {})", frontend_dir.display());

    std::thread::spawn(move || {
        let status = std::process::Command::new("bunx")
            .arg("vite")
            .current_dir(&frontend_dir)
            .env("LIVE_PORT", vite_port.to_string())
            .env("LIVE_CORE_PORT", core_port.to_string())
            .status();

        match status {
            Ok(s) => log::info!("vite exited with {s}"),
            Err(e) => log::error!("failed to spawn vite: {e}"),
        }
    });
}
