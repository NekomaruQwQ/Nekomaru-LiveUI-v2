//! KPM capture process manager.
//!
//! Spawns `live-kpm.exe`, reads 12-byte binary batches from stdout via
//! `live_kpm::read_batch()`, and pushes them into the `KpmCalculator`.

use crate::kpm::calculator::KpmCalculator;

use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use job_object::JobObject;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Batch interval passed to `live-kpm.exe` (milliseconds).
const BATCH_INTERVAL_MS: u64 = 50;

/// Sliding window duration for KPM calculation (milliseconds).
const WINDOW_DURATION_MS: u64 = 5000;

// ── KpmState ─────────────────────────────────────────────────────────────────

pub struct KpmState {
    pub calculator: KpmCalculator,
    pub active: bool,
    pub child: Option<Child>,
    pub reader_handle: Option<JoinHandle<()>>,
}

impl KpmState {
    pub const fn new() -> Self {
        Self {
            calculator: KpmCalculator::new(WINDOW_DURATION_MS, BATCH_INTERVAL_MS),
            active: false,
            child: None,
            reader_handle: None,
        }
    }

    /// Start the KPM capture process.
    pub fn start(&mut self, exe_path: &str, job: &JobObject, state_arc: &Arc<RwLock<Self>>) {
        if self.active { return; }

        let mut child = Command::new(exe_path)
            .arg("--batch-interval")
            .arg(BATCH_INTERVAL_MS.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {exe_path}: {e}"));

        if let Err(e) = job.assign(&child) {
            log::warn!("[kpm] failed to assign to job object: {e}");
        }

        let stdout = child.stdout.take().expect("stdout must be piped");

        let state_clone = Arc::clone(state_arc);

        // Stdout reader: reads fixed 12-byte binary batches.
        let reader_handle = tokio::task::spawn_blocking(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match live_kpm::read_batch(&mut reader) {
                    Ok(Some(batch)) => {
                        let mut state = state_clone.blocking_write();
                        state.calculator.push_batch(batch.t, batch.c);
                        drop(state);
                    }
                    Ok(None) => {
                        log::info!("[kpm] stdout EOF");
                        break;
                    }
                    Err(e) => {
                        log::error!("[kpm] read error: {e}");
                        break;
                    }
                }
            }

            let mut state = state_clone.blocking_write();
            state.active = false;
            state.calculator.reset();
        });

        self.child = Some(child);
        self.reader_handle = Some(reader_handle);
        self.active = true;

        log::info!("[kpm] started (batch: {BATCH_INTERVAL_MS}ms, window: {WINDOW_DURATION_MS}ms)");
    }

    pub fn stop(&mut self) {
        if !self.active { return; }

        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }

        self.active = false;
        self.calculator.reset();
        log::info!("[kpm] stopped");
    }
}

impl Drop for KpmState {
    fn drop(&mut self) { self.stop(); }
}
