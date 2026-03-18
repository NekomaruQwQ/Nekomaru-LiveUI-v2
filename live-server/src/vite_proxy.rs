//! Reverse proxy to the Vite dev server.
//!
//! Active only in dev mode (`LIVE_VITE_PORT` set).  Forwards any request that
//! doesn't match an API or WebSocket route to `http://localhost:LIVE_VITE_PORT`,
//! so the browser loads frontend assets (HTML, JS, CSS, source maps,
//! `/@vite/client`, etc.) from the core server's port.
//!
//! HMR WebSocket traffic is NOT proxied — Vite's `server.hmr.clientPort`
//! tells the HMR client to connect directly to Vite.

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};

/// Create a reqwest client and return a fallback handler bound to the
/// given Vite dev server port.
pub fn fallback(
    vite_port: u16,
) -> impl Clone + Send + Fn(Request) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>> {
    let client = reqwest::Client::new();
    move |req: Request| {
        let client = client.clone();
        Box::pin(proxy(client, vite_port, req))
    }
}

/// Forward a single request to the Vite dev server and stream the response
/// back to the browser.
async fn proxy(client: reqwest::Client, vite_port: u16, req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    let url = format!("http://localhost:{vite_port}{uri}");

    // Build the outgoing request, preserving method and headers.
    let mut builder = client.request(method, &url);
    for (name, value) in &headers {
        builder = builder.header(name, value);
    }

    // For most dev requests the body is empty (GET for assets), but forward
    // it anyway for completeness.
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            log::warn!("vite proxy: failed to read request body: {e}");
            return (StatusCode::BAD_REQUEST, "bad request body").into_response();
        }
    };
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }

    let upstream = match builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            log::warn!("vite proxy: {e}");
            return (StatusCode::BAD_GATEWAY, "vite dev server unreachable")
                .into_response();
        }
    };

    // Build the response, preserving status and headers from Vite.
    let status = upstream.status();
    let resp_headers = upstream.headers().clone();
    let body = Body::from_stream(upstream.bytes_stream());

    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = resp_headers;
    response
}
