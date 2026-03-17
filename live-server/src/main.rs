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

mod state;
mod strings;
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
}

// ── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let cli = Cli::parse();

    let state = Arc::new(AppState::new());

    let app = Router::new()
        .merge(strings::routes::router())
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
