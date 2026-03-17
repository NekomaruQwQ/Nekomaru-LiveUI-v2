//! HTTP routes for audio streaming.
//!
//! - `GET /api/v1/audio/init`          — audio format params
//! - `GET /api/v1/audio/chunks?after=N` — binary audio chunks (delta + gzip)

use crate::state::AppState;

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Deserialize;

use std::io::Write as _;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/audio/init", get(get_init))
        .route("/api/v1/audio/chunks", get(get_chunks))
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

// ── Chunks ───────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ChunksQuery {
    after: Option<String>,
}

/// `GET /api/v1/audio/chunks?after=N` — binary audio chunks.
///
/// Response layout (all little-endian, then gzip-compressed):
/// ```text
/// [u32: num_chunks]
/// per chunk: [u32: sequence][u32: payload_length][payload bytes]
/// ```
///
/// Payload per chunk: `[u64 LE: timestamp_us][delta-encoded s16le PCM]`.
async fn get_chunks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ChunksQuery>,
) -> impl IntoResponse {
    let after: u32 = query.after
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Collect chunk data and drop the lock before serialization.
    let audio = state.audio().await;
    let chunk_data: Vec<(u32, Vec<u8>)> = audio.buffer.get_chunks_after(after)
        .iter()
        .map(|ch| (ch.sequence, ch.payload.clone()))
        .collect();
    drop(audio);

    // Pre-compute total size: 4-byte header + (8 + payload) per chunk.
    let mut total_size: usize = 4;
    for &(_, ref payload) in &chunk_data {
        total_size += 8 + payload.len();
    }

    let mut buf = vec![0u8; total_size];
    let mut pos = 0;

    // Header: chunk count.
    buf[pos..pos + 4].copy_from_slice(&(chunk_data.len() as u32).to_le_bytes());
    pos += 4;

    // Each chunk: sequence + payload length + delta-encoded payload.
    for &(sequence, ref payload) in &chunk_data {
        buf[pos..pos + 4].copy_from_slice(&sequence.to_le_bytes());
        pos += 4;
        buf[pos..pos + 4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        pos += 4;

        // Copy payload into the response buffer.
        buf[pos..pos + payload.len()].copy_from_slice(payload);

        // Delta-encode the PCM region in-place on the copy.
        // Payload layout: [u64 timestamp (8 bytes)][s16le PCM samples...].
        delta_encode_pcm(&mut buf, pos + 8, payload.len() - 8);

        pos += payload.len();
    }

    // Gzip compress the entire response.
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&buf).expect("gzip write");
    let compressed = encoder.finish().expect("gzip finish");

    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_ENCODING, "gzip")
        .body(Body::from(compressed))
        .unwrap()
        .into_response()
}

// ── Delta encoding ──────────────────────────────────────────────────────────

/// Delta-encode s16le PCM samples in-place within `buf`.
///
/// First sample is kept as-is; each subsequent sample becomes `(current - prev)`.
/// Iterates backwards so earlier values aren't clobbered before they're read.
fn delta_encode_pcm(buf: &mut [u8], byte_offset: usize, byte_length: usize) {
    let sample_count = byte_length / 2;
    // Walk backwards: delta[i] = sample[i] - sample[i-1].
    for i in (1..sample_count).rev() {
        let off = byte_offset + i * 2;
        let cur = i16::from_le_bytes([buf[off], buf[off + 1]]);
        let prev = i16::from_le_bytes([buf[off - 2], buf[off - 1]]);
        let delta = cur.wrapping_sub(prev);
        buf[off..off + 2].copy_from_slice(&delta.to_le_bytes());
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_encode_roundtrip() {
        // 4 s16le samples: 100, 200, 150, 300
        let mut buf = vec![
            100i16.to_le_bytes()[0], 100i16.to_le_bytes()[1],
            200i16.to_le_bytes()[0], 200i16.to_le_bytes()[1],
            150i16.to_le_bytes()[0], 150i16.to_le_bytes()[1],
            300i16.to_le_bytes()[0], 300i16.to_le_bytes()[1],
        ];

        delta_encode_pcm(&mut buf, 0, 8);

        // First sample unchanged, rest are deltas.
        let s0 = i16::from_le_bytes([buf[0], buf[1]]);
        let d1 = i16::from_le_bytes([buf[2], buf[3]]);
        let d2 = i16::from_le_bytes([buf[4], buf[5]]);
        let d3 = i16::from_le_bytes([buf[6], buf[7]]);

        assert_eq!(s0, 100);   // unchanged
        assert_eq!(d1, 100);   // 200 - 100
        assert_eq!(d2, -50);   // 150 - 200
        assert_eq!(d3, 150);   // 300 - 150
    }
}
