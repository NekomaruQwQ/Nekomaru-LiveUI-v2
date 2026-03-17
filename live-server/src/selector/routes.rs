//! HTTP routes for the auto window selector.
//!
//! - `GET    /api/v1/streams/auto`               — selector status
//! - `POST   /api/v1/streams/auto`               — start selector
//! - `DELETE /api/v1/streams/auto`               — stop selector
//! - `GET    /api/v1/streams/auto/config`        — full preset config
//! - `PUT    /api/v1/streams/auto/config`        — replace full config
//! - `PUT    /api/v1/streams/auto/config/preset` — switch active preset

use crate::selector::config::PresetConfig;
use crate::state::AppState;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;

use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/streams/auto",
            get(get_status).post(start_selector).delete(stop_selector))
        .route("/api/v1/streams/auto/config",
            get(get_config).put(set_config))
        .route("/api/v1/streams/auto/config/preset",
            axum::routing::put(set_preset))
}

async fn get_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let sel = state.selector().await;
    Json(sel.status())
}

async fn start_selector(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut sel = state.selector_mut().await;
    let selector_arc = state.selector_arc();
    let streams_arc = state.streams_arc();
    let strings_arc = state.strings_arc();
    sel.start(&selector_arc, &streams_arc, &strings_arc);
    (StatusCode::CREATED, Json(sel.status()))
}

async fn stop_selector(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut sel = state.selector_mut().await;
    let streams_arc = state.streams_arc();
    let strings_arc = state.strings_arc();
    sel.stop(&streams_arc, &strings_arc);
    drop(sel);
    Json(serde_json::json!({ "ok": true }))
}

async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let sel = state.selector().await;
    Json(serde_json::json!({
        "preset": sel.config.preset,
        "presets": sel.config.presets,
    }))
}

async fn set_config(
    State(state): State<Arc<AppState>>,
    Json(config): Json<PresetConfig>,
) -> impl IntoResponse {
    let mut sel = state.selector_mut().await;
    sel.config = config;
    sel.config.save();
    log::info!("[selector] config updated: preset=\"{}\", {} preset(s)",
        sel.config.preset, sel.config.presets.len());
    drop(sel);
    Json(serde_json::json!({ "ok": true }))
}

async fn set_preset(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> impl IntoResponse {
    let name = String::from_utf8_lossy(&body).trim().to_owned();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "preset name required" }))).into_response();
    }

    let mut sel = state.selector_mut().await;

    // Reload from disk first (picks up hand-edits).
    sel.reload_config();

    if !sel.config.presets.contains_key(&name) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("preset \"{name}\" not found") }))).into_response();
    }

    name.clone_into(&mut sel.config.preset);
    sel.config.save();
    log::info!("[selector] switched to preset \"{name}\"");
    drop(sel);

    Json(serde_json::json!({ "ok": true })).into_response()
}
