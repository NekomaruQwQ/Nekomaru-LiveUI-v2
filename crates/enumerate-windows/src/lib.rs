//! Enumerates capturable desktop windows on Windows.
//!
//! Provides [`enumerate_windows`] which returns a list of visible, non-cloaked,
//! top-level windows with non-empty titles — the set of windows suitable for
//! screen capture.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;
use std::process;

use serde::Serialize;

use windows::core::*;
use windows::Win32::{
    Foundation::*,
    Graphics::Dwm::*,
    System::Threading::*,
    UI::WindowsAndMessaging::*,
};

// ── Output schema ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[derive(Serialize)]
pub struct WindowInfo {
    /// Window handle.
    pub hwnd: usize,
    /// Process ID that owns the window.
    pub pid: u32,
    /// Window title (lossy UTF-16 → UTF-8 conversion).
    pub title: String,
    /// Full executable path, or empty if inaccessible.
    pub executable_path: PathBuf,
}

// ── Window enumeration ───────────────────────────────────────────────────

/// Enumerates all visible, non-cloaked, top-level windows with non-empty titles.
/// Skips windows belonging to this process.
pub fn enumerate_windows() -> Vec<WindowInfo> {
    let mut out = Vec::<WindowInfo>::new();
    let out_ptr = &raw mut out;
    let own_pid = process::id();

    // `EnumWindows` invokes the callback for every top-level window.
    // We collect qualifying windows into `results` via the raw pointer.
    let _ = unsafe {
        EnumWindows(
            Some(enum_callback),
            LPARAM(out_ptr as _))
    };

    // Filter out our own process after collection (cleaner than embedding
    // the check in the callback where we'd need the PID anyway).
    out.retain(|window| window.pid != own_pid);
    out
}

/// Callback for `EnumWindows`. Returns `TRUE` to continue enumeration.
unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe {
        (lparam.0 as *mut Vec<WindowInfo>)
            .as_mut_unchecked()
    };
    enum_callback_internal(hwnd, out);
    TRUE
}

fn enum_callback_internal(hwnd: HWND, out: &mut Vec<WindowInfo>) {
    // Skip invisible windows.
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return;
    }

    // Skip owned windows (popups, toolbars, etc.) — only want top-level.
    if !(
        unsafe { GetWindow(hwnd, GW_OWNER) }
            .unwrap_or_default()
            .is_invalid()) {
        return;
    }

    // Skip cloaked windows (UWP placeholders, virtual-desktop-hidden).
    if is_cloaked(hwnd) {
        return;
    }

    let title = get_window_title(hwnd);
    if title.is_empty() {
        return;
    }

    let (pid, executable_path) = get_process_info(hwnd);

    out.push(WindowInfo {
        hwnd: hwnd.0 as usize,
        pid,
        title,
        executable_path,
    });
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Returns the window title via `GetWindowTextW`, or an empty string on failure.
fn get_window_title(hwnd: HWND) -> String {
    let buf_len = unsafe { GetWindowTextLengthW(hwnd) } as usize + 1;
    let mut buf = vec![0u16; buf_len];
    let _ = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if let Some(pos) = buf.iter().position(|&c| c == 0) {
        buf.truncate(pos);
    }
    String::from_utf16_lossy(&buf)
}

/// Returns `(pid, executable_path)` for the process owning `hwnd`.
/// On failure (e.g. elevated process), returns `(0, PathBuf::new())`.
fn get_process_info(hwnd: HWND) -> (u32, PathBuf) {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)); }
    if pid == 0 {
        return (0, PathBuf::new());
    }

    let executable_path =
        get_executable_path(pid).unwrap_or_default();
    (pid, executable_path)
}

/// Opens the process by PID and queries its full executable path.
fn get_executable_path(pid: u32) -> Option<PathBuf> {
    #[expect(clippy::multiple_unsafe_ops_per_block, reason = "Windows API calls")]
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        if handle.is_invalid() {
            return None;
        }

        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &raw mut len);
        let _ = CloseHandle(handle);
        ok.ok()?;

        Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }
}

/// Checks whether a window is "cloaked" (hidden by DWM).
/// Cloaked windows are technically visible but not shown to the user — common
/// with UWP app placeholders and windows on other virtual desktops.
fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let cloacked_ptr = &raw mut cloaked;
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            cloacked_ptr.cast(),
            size_of::<u32>() as u32)
    };
    hr.is_ok() && cloaked != 0
}
