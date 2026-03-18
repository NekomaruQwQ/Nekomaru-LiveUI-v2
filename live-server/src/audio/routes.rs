//! HTTP route for audio initialization.
//!
//! - `GET /api/v1/audio/init` — audio format params

use crate::state::AppState;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;

use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/audio/init", get(get_init))
}

/// `GET /api/v1/audio/init` — audio format parameters.
async fn get_init(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let audio = state.audio().await;
    if !audio.active {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "audio disabled" }))).into_response();
    }

    let Some(params) = audio.buffer.get_audio_params() else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "audio params not yet available" }))).into_response();
    };

    let sample_rate = params.sample_rate;
    let channels = params.channels;
    let bits_per_sample = params.bits_per_sample;
    drop(audio);

    Json(serde_json::json!({
        "sampleRate": sample_rate,
        "channels": channels,
        "bitsPerSample": bits_per_sample,
    })).into_response()
}
