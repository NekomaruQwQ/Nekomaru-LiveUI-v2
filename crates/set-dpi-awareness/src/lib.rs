//! Thin wrapper around the Win32 DPI awareness API.
//!
//! Call [`per_monitor_v2`] once at process startup — before any window or
//! geometry API usage — so that Win32 APIs return physical pixels instead of
//! DPI-virtualized logical pixels.

use windows::Win32::UI::HiDpi::*;

/// Declare per-monitor DPI awareness v2 for the current process.
///
/// Best-effort: silently ignores failure (e.g. already set by a manifest).
///
/// # Safety contract
///
/// Must be called before any Win32 window, geometry, or display API.
pub fn per_monitor_v2() {
    // SAFETY: Called once at startup, before any window/geometry API usage.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}
