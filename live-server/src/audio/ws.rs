//! WebSocket endpoint for audio chunk streaming.
//!
//! `GET /api/v1/ws/audio` — upgrades to a WebSocket that pushes PCM chunks
//! in the same binary format as `GET /audio/chunks`, but without gzip
//! compression (WebSocket has its own per-message compression via
//! permessage-deflate, and individual chunks are small enough that gzip
//! overhead hurts more than it helps).
//!
//! On connect the client may send `{"after": N}` to set its cursor.

use crate::audio::process::AudioState;
use crate::state::AppState;

use axum::Router;
use axum::extract::{State, WebSocketUpgrade, ws};
use axum::response::IntoResponse;
use axum::routing::get;

use std::sync::Arc;

use tokio::sync::{RwLock, broadcast};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/ws/audio", get(ws_audio))
}

async fn ws_audio(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let audio_arc = state.audio_arc();
    ws.on_upgrade(move |socket| handle_audio_ws(socket, audio_arc))
}

/// Serialize buffered audio chunks into binary (same layout as HTTP, no gzip).
///
/// Layout: `[u32: num_chunks] per chunk: [u32: sequence][u32: len][payload]`.
fn serialize_chunks(audio: &AudioState, after: u32) -> (Vec<u8>, u32) {
    let chunks = audio.buffer.get_chunks_after(after);
    if chunks.is_empty() { return (Vec::new(), after); }

    let mut total: usize = 4;
    for ch in &chunks { total += 8 + ch.payload.len(); }

    let mut buf = vec![0u8; total];
    let mut pos = 0;

    buf[pos..pos + 4].copy_from_slice(&(chunks.len() as u32).to_le_bytes());
    pos += 4;

    let mut last_seq = after;
    for ch in &chunks {
        buf[pos..pos + 4].copy_from_slice(&ch.sequence.to_le_bytes());
        pos += 4;
        buf[pos..pos + 4].copy_from_slice(&(ch.payload.len() as u32).to_le_bytes());
        pos += 4;
        buf[pos..pos + ch.payload.len()].copy_from_slice(&ch.payload);
        pos += ch.payload.len();
        last_seq = ch.sequence;
    }

    (buf, last_seq)
}

async fn handle_audio_ws(
    mut socket: ws::WebSocket,
    audio_arc: Arc<RwLock<AudioState>>,
) {
    let mut last_seq: u32 = 0;

    // Subscribe to the broadcast channel.
    let mut rx: broadcast::Receiver<()> = {
        let audio = audio_arc.read().await;
        audio.notify.subscribe()
    };

    // Check for an initial cursor message.
    if let Some(Ok(ws::Message::Text(text))) = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        socket.recv(),
    ).await.ok().flatten()
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(n) = v.get("after").and_then(serde_json::Value::as_u64) {
        last_seq = n as u32;
    }

    // Initial catch-up.
    {
        let audio = audio_arc.read().await;
        let (buf, new_seq) = serialize_chunks(&audio, last_seq);
        if !buf.is_empty() {
            last_seq = new_seq;
            drop(audio);
            if socket.send(ws::Message::Binary(buf.into())).await.is_err() { return; }
        }
    }

    // Push loop.
    loop {
        match rx.recv().await {
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(_)) | Ok(()) => {}
        }

        let audio = audio_arc.read().await;
        let (buf, new_seq) = serialize_chunks(&audio, last_seq);
        drop(audio);

        if buf.is_empty() { continue; }

        last_seq = new_seq;
        if socket.send(ws::Message::Binary(buf.into())).await.is_err() { break; }
    }

    drop(socket);
}
