//! Audio capture process manager.
//!
//! Singleton that spawns `live-audio.exe`, reads stdout via
//! `live_audio::read_message()`, and pushes chunks into the `AudioBuffer`.

use crate::audio::buffer::AudioBuffer;

use live_audio::Message;

use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Buffer capacity: 100 chunks = ~1 second at 10ms/chunk.
const AUDIO_BUFFER_CAPACITY: usize = 100;

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
        state_arc: &Arc<RwLock<Self>>,
    ) {
        if self.active { return; }

        let args = [exe_path.to_owned(),
            "--device".into(),
            device_name.to_owned()];

        let mut child = Command::new(&args[0])
            .args(&args[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", args[0]));

        let stdout = child.stdout.take().expect("stdout must be piped");
        let stderr = child.stderr.take().expect("stderr must be piped");

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
                                    "[audio] params: {}Hz, {}ch, {}-bit",
                                    params.sample_rate, params.channels, params.bits_per_sample);
                                state.buffer.set_audio_params(params);
                            }
                            Message::AudioFrame(frame) => {
                                state.buffer.push_chunk(&frame);
                            }
                            Message::Error(e) => {
                                log::error!("[audio] capture error: {e}");
                            }
                        }
                        drop(state);
                    }
                    Ok(None) => {
                        log::info!("[audio] stdout EOF");
                        break;
                    }
                    Err(e) => {
                        log::error!("[audio] read error: {e}");
                        break;
                    }
                }
            }

            // Mark as inactive and reset buffer on exit.
            let mut state = state_clone.blocking_write();
            state.active = false;
            state.buffer.reset();
        });

        // Stderr reader task.
        tokio::task::spawn_blocking(move || {
            use std::io::BufRead as _;
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(line) if !line.is_empty() => log::info!("[audio] {line}"),
                    Err(e) => { log::error!("[audio] stderr: {e}"); break; }
                    _ => {}
                }
            }
        });

        self.child = Some(child);
        self.reader_handle = Some(reader_handle);
        self.active = true;

        log::info!("[audio] started");
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
        log::info!("[audio] stopped");
    }
}

impl Drop for AudioState {
    fn drop(&mut self) { self.stop(); }
}
