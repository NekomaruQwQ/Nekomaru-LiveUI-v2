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
    dpi::PhysicalPosition,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    event_loop::EventLoop,
    window::Window,
    window::WindowButtons,
};

use wry::WebView;
use wry::WebViewBuilder;

use clap::Parser;

const WINDOW_SIZE: LogicalSize<u32> = LogicalSize::new(1280, 800);

/// Reads `LIVE_PORT` from the environment, panics if not set or invalid,
/// and constructs the server URL.
fn get_server_url() -> String {
    let port = std::env::var("LIVE_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .expect("LIVE_PORT not set or is not a valid port number");
    format!("http://localhost:{port}")
}

/// CLI arguments for the webview host.
#[derive(Parser)]
#[command(name = "live-app")]
struct LiveAppArgs {
    /// URL to load in the webview.
    pub url: Option<String>,

    /// Window title. Defaults to the URL if not provided.
    #[arg(long, short = 'm')]
    pub window_title: Option<String>,
}

/// Parse CLI arguments and launch the webview. Convenience entry point for
/// binaries that don't need programmatic control over the URL or title.
fn main() {
    let args = LiveAppArgs::parse();
    let title =
        args.window_title
            .as_ref()
            .map_or("Nekomaru LiveUI v2", String::as_str);
    let url =
        args.url
            .unwrap_or_else(get_server_url);
    run_webview(title, &url);
}

pub fn run_webview(title: &str, url: &str) {
    // SAFETY: Single-threaded access to environment variable, set before
    // any threads are spawned.
    unsafe {
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", [
            // Force WebView2 to use a device scale factor of 1 to get consistent behavior
            // across different DPI settings.
            "--force-device-scale-factor=2",
            // Disable WebView2's background throttling features to prevent the webview
            // from freezing when the window is not in the foreground. This is necessary
            // for streaming.
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
        ].join(" "));
    }

    pretty_env_logger::init();

    EventLoop::<()>::new()
        .expect("failed to create event loop")
        .run_app_with(move |event_loop| {
            let app = LiveApp::new(event_loop);
            app.window.set_title(title);
            app.webview.load_url(url).expect("failed to load the given url");

            move |event_loop, event| {
                // We do not need to do anything in the event loop, but we must
                // keep the app alive for the lifetime of the loop. Dropping it
                // will close the window and webview.
                let _ = app;

                if let AppEvent::WindowEvent(window_id, event) = event &&
                    window_id == app.window.id() {
                    match event {
                        WindowEvent::CloseRequested =>
                            event_loop.exit(),
                        WindowEvent::Resized(new_size) => {
                            let _ = app.webview.set_bounds(wry::Rect {
                                position: PhysicalPosition::new(0, 0).into(),
                                size: new_size.into(),
                            });
                        }
                        _ => {}
                    }
                }
            }
        })
        .expect("failed to run event loop");
}

/// Holds the window and webview, kept alive for the lifetime of the app.
struct LiveApp {
    window: Window,
    webview: WebView,
}

impl LiveApp {
    fn new(event_loop: &ActiveEventLoop) -> Self {
        let window =
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Loading - Nekomaru LiveUI v2")
                    .with_inner_size(WINDOW_SIZE)
                    .with_resizable(false)
                    .with_enabled_buttons(WindowButtons::CLOSE))
                .expect("failed to create window");

        let webview =
            WebViewBuilder::new()
                .build(&window)
                .expect("failed to create webview");

        Self { window, webview }
    }
}
