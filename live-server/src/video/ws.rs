//! WebSocket endpoint for video frame streaming.
//!
//! `GET /api/v1/ws/video/:id` — upgrades to a WebSocket that pushes AVCC
//! frames in the same binary format as `GET /streams/:id/frames`.
//!
//! On connect the client may send a JSON text message `{"after": N}` to set
//! its initial cursor; otherwise the server starts from sequence 0 (which
//! triggers keyframe gating in the buffer).

use crate::state::AppState;
use crate::video::process::StreamRegistry;

use axum::Router;
use axum::extract::{Path, State, WebSocketUpgrade, ws};
use axum::response::IntoResponse;
use axum::routing::get;

use std::sync::Arc;

use tokio::sync::{RwLock, broadcast};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/ws/video/{id}", get(ws_video))
}

async fn ws_video(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let streams_arc = state.streams_arc();

    ws.on_upgrade(move |socket| handle_video_ws(socket, id, streams_arc))
}

/// Serialize buffered frames into the same binary layout as the HTTP endpoint.
///
/// Layout (all LE unless noted):
/// ```text
/// [u32: generation][u32: num_frames]
/// per frame: [u32: sequence][u64: timestamp_us][u8: is_keyframe]
///            [u32: avcc_payload_length][payload bytes]
/// ```
fn serialize_frames(
    registry: &StreamRegistry,
    id: &str,
    after: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let stream = registry.streams.get(id)?;
    let generation = stream.generation;
    let frames = stream.buffer.get_frames_after(after);

    if frames.is_empty() { return Some((Vec::new(), generation, after)); }

    // Per-frame envelope: sequence(4) + timestamp(8) + keyframe(1) + len(4) = 17.
    let mut total: usize = 8;
    for f in &frames { total += 17 + f.payload.len(); }

    let mut buf = vec![0u8; total];
    let mut pos = 0;

    buf[pos..pos + 4].copy_from_slice(&generation.to_le_bytes());
    pos += 4;
    buf[pos..pos + 4].copy_from_slice(&(frames.len() as u32).to_le_bytes());
    pos += 4;

    let mut last_seq = after;
    for f in &frames {
        buf[pos..pos + 4].copy_from_slice(&f.sequence.to_le_bytes());
        pos += 4;
        buf[pos..pos + 8].copy_from_slice(&f.timestamp_us.to_le_bytes());
        pos += 8;
        buf[pos] = u8::from(f.is_keyframe);
        pos += 1;
        buf[pos..pos + 4].copy_from_slice(&(f.payload.len() as u32).to_le_bytes());
        pos += 4;
        buf[pos..pos + f.payload.len()].copy_from_slice(&f.payload);
        pos += f.payload.len();
        last_seq = f.sequence;
    }

    Some((buf, generation, last_seq))
}

async fn handle_video_ws(
    mut socket: ws::WebSocket,
    id: String,
    streams: Arc<RwLock<StreamRegistry>>,
) {
    let mut last_seq: u32 = 0;

    // Subscribe to the broadcast channel for this stream.
    let mut rx: broadcast::Receiver<()> = {
        let registry = streams.read().await;
        let Some(stream) = registry.streams.get(&id) else {
            drop(socket);
            return;
        };
        let rx = stream.notify.subscribe();
        drop(registry);
        rx
    };

    // Check for an initial cursor message from the client (non-blocking).
    // The client may send `{"after": N}` to resume from a known position.
    if let Some(Ok(ws::Message::Text(text))) = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        socket.recv(),
    ).await.ok().flatten()
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(n) = v.get("after").and_then(serde_json::Value::as_u64) {
        last_seq = n as u32;
    }

    // Send an initial catch-up batch and seed `current_gen` from the stream's
    // actual generation (avoids a spurious gen-change on the first notification).
    let mut current_gen: u32;
    {
        let registry = streams.read().await;
        if let Some((buf, generation, new_seq)) = serialize_frames(&registry, &id, last_seq) {
            current_gen = generation;
            if !buf.is_empty() {
                last_seq = new_seq;
                drop(registry);
                if socket.send(ws::Message::Binary(buf.into())).await.is_err() { return; }
            }
        } else {
            // Stream gone.
            drop(socket);
            return;
        }
    }

    // Main push loop: wait for notification, read buffer, send frames.
    loop {
        // Wait for a frame-push notification.  Lagged is fine — we just
        // catch up via the sequence-based buffer read.
        match rx.recv().await {
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(_)) | Ok(()) => {}
        }

        let registry = streams.read().await;
        let Some((buf, generation, new_seq)) = serialize_frames(&registry, &id, last_seq) else {
            break; // Stream destroyed.
        };
        drop(registry);

        // Generation changed — the buffer was reset, so our cursor is stale.
        // Reset to 0 and re-read from the first keyframe.
        if generation != current_gen {
            current_gen = generation;
            if new_seq == last_seq {
                // The serialize used our stale cursor and returned nothing.
                // Re-read with cursor 0 to get the new generation's frames.
                last_seq = 0;
                let registry = streams.read().await;
                let Some((buf, _, new_seq)) = serialize_frames(&registry, &id, 0) else {
                    break;
                };
                drop(registry);
                if !buf.is_empty() {
                    last_seq = new_seq;
                    if socket.send(ws::Message::Binary(buf.into())).await.is_err() { break; }
                }
                continue;
            }
        }

        if buf.is_empty() { continue; }

        last_seq = new_seq;
        if socket.send(ws::Message::Binary(buf.into())).await.is_err() { break; }
    }

    drop(socket);
}
