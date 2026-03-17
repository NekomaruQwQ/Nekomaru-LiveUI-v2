//! HTTP routes for the string store.
//!
//! - `GET  /api/v1/strings`      — all key-value pairs (user + computed)
//! - `PUT  /api/v1/strings/:key` — set a value (403 for `$`-prefixed keys)
//! - `DELETE /api/v1/strings/:key` — delete a value (403 for `$`-prefixed keys)

use crate::state::AppState;
use crate::strings::store::StringStoreError;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;

use serde::Deserialize;

use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/strings", get(get_all))
        .route("/api/v1/strings/{key}", get(get_one).put(put_one).delete(delete_one))
}

/// `GET /api/v1/strings` — return all entries as a flat JSON object.
async fn get_all(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let store = state.strings().await;
    Json(store.get_all())
}

/// `GET /api/v1/strings/:key` — return a single entry.
async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let all = state.strings().await.get_all();
    match all.get(&key) {
        Some(value) => Json(serde_json::json!({ "value": value })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "not found" }))).into_response(),
    }
}

#[derive(Deserialize)]
struct PutBody {
    value: String,
}

/// `PUT /api/v1/strings/:key` — set a string value.
async fn put_one(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<PutBody>,
) -> impl IntoResponse {
    let mut store = state.strings_mut().await;
    match store.set(&key, &body.value) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(StringStoreError::ComputedReadonly) =>
            (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "computed strings are readonly" }))).into_response(),
        Err(StringStoreError::InvalidKey) =>
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid key" }))).into_response(),
    }
}

/// `DELETE /api/v1/strings/:key` — delete a string.
async fn delete_one(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let mut store = state.strings_mut().await;
    match store.delete(&key) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(StringStoreError::ComputedReadonly) =>
            (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "computed strings are readonly" }))).into_response(),
        Err(StringStoreError::InvalidKey) =>
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid key" }))).into_response(),
    }
}
