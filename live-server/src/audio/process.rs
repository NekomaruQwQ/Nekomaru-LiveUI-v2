//! Audio capture process manager.
//!
//! Singleton that spawns `live-audio.exe`, reads stdout via
//! `live_audio::read_message()`, and pushes chunks into the `AudioBuffer`.

use crate::audio::buffer::AudioBuffer;

use live_audio::Message;

use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use job_object::JobObject;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::constant::AUDIO_BUFFER_CAPACITY;

// ── AudioState ───────────────────────────────────────────────────────────────

pub struct AudioState {
    pub buffer: AudioBuffer,
    pub active: bool,
    pub child: Option<Child>,
    pub reader_handle: Option<JoinHandle<()>>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            buffer: AudioBuffer::new(AUDIO_BUFFER_CAPACITY),
            active: false,
            child: None,
            reader_handle: None,
        }
    }

    /// Start the audio capture process.
    pub fn start(
        &mut self,
        exe_path: &str,
        device_name: &str,
        job: &JobObject,
        state_arc: &Arc<RwLock<Self>>,
    ) {
        if self.active { return; }

        let args = [exe_path.to_owned(),
            "--device".into(),
            device_name.to_owned()];

        let mut child = Command::new(&args[0])
            .args(&args[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", args[0]));

        if let Err(e) = job.assign(&child) {
            log::warn!("failed to assign to job object: {e}");
        }

        let stdout = child.stdout.take().expect("stdout must be piped");

        let state_clone = Arc::clone(state_arc);

        // Stdout reader task.
        let reader_handle = tokio::task::spawn_blocking(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match live_audio::read_message(&mut reader) {
                    Ok(Some(msg)) => {
                        let mut state = state_clone.blocking_write();
                        match msg {
                            Message::AudioParams(params) => {
                                log::info!(
                                    "params: {}Hz, {}ch, {}-bit",
                                    params.sample_rate, params.channels, params.bits_per_sample);
                                state.buffer.set_audio_params(params);
                            }
                            Message::AudioChunk(frame) => {
                                state.buffer.push_chunk(&frame);
                            }
                            Message::Error(e) => {
                                log::error!("capture error: {e}");
                            }
                        }
                        drop(state);
                    }
                    Ok(None) => {
                        log::info!("stdout EOF");
                        break;
                    }
                    Err(e) => {
                        log::error!("read error: {e}");
                        break;
                    }
                }
            }

            // Mark as inactive and reset buffer on exit.
            let mut state = state_clone.blocking_write();
            state.active = false;
            state.buffer.reset();
        });

        self.child = Some(child);
        self.reader_handle = Some(reader_handle);
        self.active = true;

        log::info!("started");
    }

    /// Stop the audio capture process.
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
        self.buffer.reset();
        log::info!("stopped");
    }
}

impl Drop for AudioState {
    fn drop(&mut self) { self.stop(); }
}
