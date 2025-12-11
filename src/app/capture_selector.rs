use std::time::Instant;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[expect(clippy::struct_field_names)]
pub struct LiveCaptureWindowSelector {
    last_check: Instant,
    last_foreground: HWND,
    last_capture: HWND,
}

impl Default for LiveCaptureWindowSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveCaptureWindowSelector {
    pub fn new() -> Self {
        Self {
            last_capture: HWND::default(),
            last_check: Instant::now(),
            last_foreground: HWND::default(),
        }
    }

    pub fn update(&mut self, hwnd: &mut HWND) -> bool {
        // Throttle checks to once every 2 seconds.
        let now = Instant::now();
        if now.duration_since(self.last_check).as_secs_f32() < 2.0 {
            // Too early to check.
            return false;
        }
        self.last_check = now;

        // Get the current foreground window.
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_invalid() ||
            foreground == self.last_foreground {
            // No change in foreground window, and as a result, no need to
            // change the capture target.
            return false;
        }
        self.last_foreground = foreground;

        // Determine whether to capture this window by its executable path.
        let executable_path =
            Self::get_executable_path_by_hwnd(foreground)
                .unwrap_or_default();
        let window_text = {
            let mut buf = [0u8; 256];
            let len = unsafe { GetWindowTextA(foreground, &mut buf) };
            String::from_utf8_lossy(&buf[..len as usize]).into_owned()
        };
        log::info!("foreground: {window_text} ({executable_path})");
        if !Self::should_capture(&executable_path) {
            return false;
        }

        self.last_capture = foreground;
        *hwnd = foreground;
        true
    }

    fn get_executable_path_by_hwnd(hwnd: HWND) -> Option<String> {
        #[expect(clippy::multiple_unsafe_ops_per_block)]
        unsafe {
            let mut process_id = 0;
            let _ = GetWindowThreadProcessId(hwnd, Some(&raw mut process_id));

            let process_handle =
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION,
                    false,
                    process_id).ok()?;
            if process_handle.is_invalid() {
                return None;
            }

            let mut buffer = [0u8; 260];
            let mut length = 260u32;
            QueryFullProcessImageNameA(
                process_handle,
                PROCESS_NAME_WIN32,
                PSTR(buffer.as_mut_ptr()),
                &raw mut length).ok()?;
            CloseHandle(process_handle).ok()?;

            Some(String::from_utf8_lossy(&buffer[..length as usize]).into_owned())
        }
    }

    fn should_capture(path: &str) -> bool {
        const INCLUDE_LIST: &[&str] = &[
            "devenv.exe", // Visual Studio
            r"C:\Program Files\Microsoft Visual Studio Code\Code.exe",
            r"C:\Program Files\JetBrains\",
            r"D:\7-Games\",
            r"D:\7-Games.Steam\steamapps\common\",
            r"E:\Nekomaru.Games\",
            r"E:\SteamLibrary\steamapps\common\",
        ];

        const EXCLUDE_LIST: &[&str] = &[
            "gogh.exe",
        ];

        INCLUDE_LIST.iter().any(|&keyword| path.contains(keyword)) &&
            !EXCLUDE_LIST.iter().any(|&keyword| path.contains(keyword))
    }
}
