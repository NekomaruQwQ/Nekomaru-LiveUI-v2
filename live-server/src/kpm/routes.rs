//! HTTP route for KPM (keystrokes-per-minute).
//!
//! - `GET /api/v1/kpm` — `{ kpm: number }`

use crate::state::AppState;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;

use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/kpm", get(get_kpm))
}

/// `GET /api/v1/kpm` — current KPM value from the sliding window.
async fn get_kpm(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let kpm_state = state.kpm().await;
    if !kpm_state.active {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "kpm not available" }))).into_response();
    }

    let kpm = kpm_state.calculator.get_kpm();
    Json(serde_json::json!({ "kpm": kpm.round() as i64 })).into_response()
}
