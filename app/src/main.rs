//! Minimal wry webview host for Nekomaru LiveUI.
//!
//! Opens a non-resizable window at the stream resolution and loads the
//! LiveServer frontend. This is a thin shell — all capture, encoding, and
//! stream management lives in `live-capture.exe` and LiveServer.

use nkcore::prelude::*;
use nkcore::os::windows::winit::{
    AppEvent,
    EventLoopExt as _,
};

use winit::{
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    event_loop::EventLoop,
    window::Window,
    window::WindowButtons,
};

use wry::WebView;
use wry::WebViewBuilder;

const WINDOW_SIZE: PhysicalSize<u32> = PhysicalSize::new(1920, 1200);

/// Reads `LIVE_PORT` from the environment, panics if not set or invalid,
/// and constructs the server URL.
fn get_server_url() -> String {
    let port = std::env::var("LIVE_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .expect("LIVE_PORT not set or is not a valid port number");
    format!("http://localhost:{port}")
}

fn main() {
    pretty_env_logger::init();

    EventLoop::<()>::new()
        .expect("failed to create event loop")
        .run_app_with(|event_loop| {
            let app = LiveApp::new(event_loop);
            move |event_loop, event| {
                // We do not need to do anything in the event loop, but we must
                // keep the app alive for the lifetime of the loop. Dropping it
                // will close the window and webview.
                let _ = app;

                if let AppEvent::WindowEvent(
                    window_id,
                    WindowEvent::CloseRequested) = event &&
                    window_id == app.window.id() {
                    event_loop.exit();
                }
            }
        })
        .expect("failed to run event loop");
}

/// Holds the window and webview, kept alive for the lifetime of the app.
#[expect(dead_code, reason = "fields must be kept alive")]
struct LiveApp {
    window: Window,
    webview: WebView,
}

impl LiveApp {
    fn new(event_loop: &ActiveEventLoop) -> Self {
        let window =
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Nekomaru LiveUI")
                    .with_inner_size(WINDOW_SIZE)
                    .with_resizable(false)
                    .with_enabled_buttons(WindowButtons::CLOSE))
                .expect("failed to create window");

        let url = get_server_url();
        let webview =
            WebViewBuilder::new()
                .with_url(&url)
                .build(&window)
                .expect("failed to create webview");

        #[cfg(debug_assertions)]
            webview.open_devtools();

        log::info!("loading frontend at: {url}");

        Self { window, webview }
    }
}
