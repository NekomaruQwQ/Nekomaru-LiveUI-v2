fn main() {
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

    live_app::run_webview(
        "YouTube Music - Nekomaru LiveUI v2",
        "https://music.youtube.com/");
}
