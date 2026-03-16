//! `live-kpm.exe` — system-wide keystroke counter to stdout.
//!
//! Installs a low-level keyboard hook (`WH_KEYBOARD_LL`) to count keystrokes
//! and writes batched counts as JSON lines to stdout.  The server spawns this
//! process and reads stdout to compute keystrokes-per-minute.
//!
//! ## Privacy-by-Design
//!
//! The hook callback **never** inspects key identity — no `vkCode`, `scanCode`,
//! or `KBDLLHOOKSTRUCT` fields are read.  Only the *occurrence* of a key-down
//! event is counted.  This eliminates any risk of accidentally logging passwords,
//! stream keys, or private messages.
//!
//! ## Usage
//!
//! ```text
//! live-kpm.exe --batch-interval 50
//! ```

#![expect(clippy::multiple_unsafe_ops_per_block, reason = "Windows API calls")]

use live_kpm::*;

use clap::Parser;

use std::io::BufWriter;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ── Shared State ─────────────────────────────────────────────────────────────

/// Keystroke counter shared between the hook callback (main thread) and the
/// writer thread.  The hook increments; the writer reads + resets.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Signals the writer thread to exit (set on broken pipe or shutdown).
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

// ── CLI ──────────────────────────────────────────────────────────────────────

/// System-wide keystroke counter to stdout.
#[derive(Parser)]
struct Cli {
    /// Batch interval in milliseconds.  The process accumulates keystrokes
    /// for this duration, then writes a single JSON line with the count.
    #[arg(long)]
    batch_interval: u64,
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .target(pretty_env_logger::env_logger::Target::Stderr)
        .init();

    let cli = Cli::parse();

    log::info!("starting keyboard hook (batch interval: {}ms)", cli.batch_interval);

    // Spawn the writer thread before installing the hook so it's ready to
    // consume counts immediately.
    let batch_interval = Duration::from_millis(cli.batch_interval);
    std::thread::spawn(move || writer_loop(batch_interval));

    // Install the low-level keyboard hook on the main thread.
    // WH_KEYBOARD_LL hooks require the installing thread to pump messages.
    //
    // SAFETY: The hook callback (`keyboard_hook_proc`) follows the
    // `HOOKPROC` calling convention and always calls `CallNextHookEx`.
    let hook = unsafe {
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0)
    };

    let Ok(hook) = hook else {
        log::error!("failed to install keyboard hook");
        std::process::exit(1);
    };

    log::info!("keyboard hook installed, entering message pump");

    // Run the Win32 message pump.  This is required for `WH_KEYBOARD_LL` to
    // receive events — the OS dispatches hook callbacks on the thread that
    // installed the hook, but only if it's pumping messages.
    //
    // SAFETY: `msg` is zero-initialized and only used as an out-parameter
    // for `GetMessageW`.  The loop exits cleanly on `WM_QUIT` or error.
    unsafe {
        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&raw mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }

    log::info!("message pump exited, cleaning up");
    SHUTDOWN.store(true, Ordering::Relaxed);

    // SAFETY: `hook` is a valid handle from the successful `SetWindowsHookExW`.
    let _ = unsafe { UnhookWindowsHookEx(hook) };
}

// ── Hook Callback ────────────────────────────────────────────────────────────

/// Low-level keyboard hook callback.
///
/// # Privacy-by-design
///
/// This function **deliberately ignores all key identity fields** (`vkCode`,
/// `scanCode`, `flags`) in the `KBDLLHOOKSTRUCT` pointed to by `lparam`.
/// Only the event type (`wparam`) is inspected to distinguish key-down from
/// key-up events.  This makes it structurally impossible for this process to
/// act as a keylogger.
///
/// # Safety
///
/// Called by the OS with valid `ncode`, `wparam`, `lparam` per the
/// `WH_KEYBOARD_LL` contract.  Must return the result of `CallNextHookEx`
/// promptly (Windows removes hooks that don't return within ~200ms).
unsafe extern "system" fn keyboard_hook_proc(
    ncode: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if ncode >= 0 {
        // Count both regular and system key-downs (Alt+key combos).
        // WM_KEYDOWN fires on initial press AND auto-repeat — we count both
        // because holding a key IS typing activity for KPM purposes.
        let is_keydown = wparam.0 == WM_KEYDOWN as usize
            || wparam.0 == WM_SYSKEYDOWN as usize;

        if is_keydown {
            COUNTER.fetch_add(1, Ordering::Relaxed);
        }
    }

    // SAFETY: Always pass the event to the next hook in the chain.
    unsafe { CallNextHookEx(None, ncode, wparam, lparam) }
}

// ── Writer Thread ────────────────────────────────────────────────────────────

/// Timer loop that reads the atomic counter, resets it, and writes a JSON
/// batch line to stdout.  Exits on broken pipe or shutdown signal.
fn writer_loop(interval: Duration) {
    let mut writer = BufWriter::new(std::io::stdout().lock());

    loop {
        std::thread::sleep(interval);

        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        // Atomically read and reset the counter.
        let count = COUNTER.swap(0, Ordering::Relaxed);

        let timestamp_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;

        let batch = Batch { t: timestamp_us, c: count };

        if let Err(e) = write_batch(&mut writer, &batch) {
            // Broken pipe means the server killed us — exit cleanly.
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                log::info!("stdout closed, exiting");
            } else {
                log::error!("write failed: {e}");
            }
            SHUTDOWN.store(true, Ordering::Relaxed);
            // Post WM_QUIT to the main thread's message pump so it exits.
            // SAFETY: `PostQuitMessage` is safe to call from any thread.
            unsafe { PostQuitMessage(0); }
            break;
        }
    }
}
