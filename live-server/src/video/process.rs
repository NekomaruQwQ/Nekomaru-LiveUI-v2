//! Process manager for `live-video.exe` instances.
//!
//! Spawns child processes, reads their stdout via `live_video::read_message()`,
//! and pushes parsed frames into the stream's `VideoBuffer`.

use crate::video::buffer::StreamBuffer;

use live_video::Message;

use std::collections::HashMap;
use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use job_object::JobObject;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::constant::FRAME_BUFFER_CAPACITY;

// ── Types ────────────────────────────────────────────────────────────────────

/// Status of a capture stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamStatus {
    Starting,
    Running,
    Stopped,
}

/// A single capture stream backed by a `live-video.exe` child process.
pub struct VideoStream {
    pub id: String,
    pub hwnd: String,
    pub status: StreamStatus,
    pub buffer: StreamBuffer,
    /// Bumped each time the underlying capture process is replaced.
    pub generation: u32,
    /// Handle to the child process.
    pub child: Option<Child>,
    /// Abort handle for the stdout reader task.
    pub reader_handle: Option<JoinHandle<()>>,
}

impl VideoStream {
    /// Kill the child process and abort the reader task.
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
        self.status = StreamStatus::Stopped;
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) { self.kill(); }
}

// ── Stream Registry ──────────────────────────────────────────────────────────

/// Registry of all active capture streams.
///
/// The `RwLock` is held briefly for reads (frame polling at 60fps) and writes
/// (stream create/destroy, frame push).  Individual stream buffers are accessed
/// within the lock — the granularity is the entire registry, not per-stream,
/// keeping the implementation simple for the expected 1-3 concurrent streams.
pub struct StreamRegistry {
    pub streams: HashMap<String, VideoStream>,
    /// Path to the `live-video.exe` binary.
    pub exe_path: String,
    /// Job object — assigned children are killed when the server exits.
    pub job: Arc<JobObject>,
}

impl StreamRegistry {
    pub fn new(exe_path: String, job: Arc<JobObject>) -> Self {
        Self {
            streams: HashMap::new(),
            exe_path,
            job,
        }
    }

    /// List all streams as serializable info structs.
    pub fn list(&self) -> Vec<StreamInfo> {
        self.streams.values().map(|s| StreamInfo {
            id: s.id.clone(),
            hwnd: s.hwnd.clone(),
            status: s.status,
            generation: s.generation,
        }).collect()
    }

    /// Create a resample-mode stream with a random ID.
    pub fn create_stream(
        &mut self,
        hwnd: &str,
        width: u32,
        height: u32,
        registry: &std::sync::Arc<RwLock<Self>>,
    ) -> String {
        let id = short_id();
        let args = vec![
            self.exe_path.clone(),
            "--hwnd".into(), hwnd.into(),
            "--width".into(), width.to_string(),
            "--height".into(), height.to_string(),
        ];
        self.spawn_capture(&id, hwnd, &args, registry);
        id
    }

    /// Create a crop-mode stream with a random ID.
    pub fn create_crop_stream(
        &mut self,
        hwnd: &str,
        crop: &CropParams,
        fps: Option<u32>,
        registry: &std::sync::Arc<RwLock<Self>>,
    ) -> String {
        let id = short_id();
        let mut args = vec![
            self.exe_path.clone(),
            "--hwnd".into(), hwnd.into(),
            "--crop-min-x".into(), crop.min_x.to_string(),
            "--crop-min-y".into(), crop.min_y.to_string(),
            "--crop-max-x".into(), crop.max_x.to_string(),
            "--crop-max-y".into(), crop.max_y.to_string(),
        ];
        if let Some(fps) = fps {
            args.push("--fps".into());
            args.push(fps.to_string());
        }
        self.spawn_capture(&id, hwnd, &args, registry);
        id
    }

    /// Replace a well-known stream in-place (kill old process, bump generation).
    /// Creates a new stream if none exists with this ID.
    pub fn replace_stream(
        &mut self,
        id: &str,
        hwnd: &str,
        width: u32,
        height: u32,
        registry: &std::sync::Arc<RwLock<Self>>,
    ) {
        let args = vec![
            self.exe_path.clone(),
            "--hwnd".into(), hwnd.into(),
            "--width".into(), width.to_string(),
            "--height".into(), height.to_string(),
        ];
        self.replace_or_create(id, hwnd, &args, registry);
    }

    /// Replace a well-known crop stream in-place.
    pub fn replace_crop_stream(
        &mut self,
        id: &str,
        hwnd: &str,
        crop: &CropParams,
        fps: Option<u32>,
        registry: &std::sync::Arc<RwLock<Self>>,
    ) {
        let mut args = vec![
            self.exe_path.clone(),
            "--hwnd".into(), hwnd.into(),
            "--crop-min-x".into(), crop.min_x.to_string(),
            "--crop-min-y".into(), crop.min_y.to_string(),
            "--crop-max-x".into(), crop.max_x.to_string(),
            "--crop-max-y".into(), crop.max_y.to_string(),
        ];
        if let Some(fps) = fps {
            args.push("--fps".into());
            args.push(fps.to_string());
        }
        self.replace_or_create(id, hwnd, &args, registry);
    }

    /// Destroy a stream by ID.
    pub fn destroy_stream(&mut self, id: &str) {
        if let Some(mut stream) = self.streams.remove(id) {
            stream.kill();
            log::info!("[{id}] destroyed");
        }
    }

    /// Kill all child processes.  Called on server shutdown.
    pub fn destroy_all(&mut self) {
        let ids: Vec<_> = self.streams.keys().cloned().collect();
        for id in ids {
            self.destroy_stream(&id);
        }
    }

    // ── Internal ─────────────────────────────────────────────────────────

    fn replace_or_create(
        &mut self,
        id: &str,
        hwnd: &str,
        args: &[String],
        registry: &std::sync::Arc<RwLock<Self>>,
    ) {
        if let Some(stream) = self.streams.get_mut(id) {
            stream.kill();
            hwnd.clone_into(&mut stream.hwnd);
            stream.status = StreamStatus::Starting;
            stream.buffer.reset();
            stream.generation += 1;
            let generation = stream.generation;

            let (child, reader_handle) = spawn_and_wire(id, args, &self.job, registry);
            stream.child = Some(child);
            stream.reader_handle = Some(reader_handle);

            log::info!("[{id}] replaced (gen {generation})");
        } else {
            self.spawn_capture(id, hwnd, args, registry);
        }
    }

    fn spawn_capture(
        &mut self,
        id: &str,
        hwnd: &str,
        args: &[String],
        registry: &std::sync::Arc<RwLock<Self>>,
    ) {
        let (child, reader_handle) = spawn_and_wire(id, args, &self.job, registry);

        let stream = VideoStream {
            id: id.to_owned(),
            hwnd: hwnd.to_owned(),
            status: StreamStatus::Starting,
            buffer: StreamBuffer::new(FRAME_BUFFER_CAPACITY),
            generation: 1,
            child: Some(child),
            reader_handle: Some(reader_handle),
        };

        self.streams.insert(id.to_owned(), stream);
        log::info!("[{id}] spawned");
    }
}

/// Crop region parameters for crop-mode capture.
pub struct CropParams {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

/// Serializable stream info for the list endpoint.
#[derive(serde::Serialize)]
pub struct StreamInfo {
    pub id: String,
    pub hwnd: String,
    pub status: StreamStatus,
    pub generation: u32,
}

// ── Child Process + Reader Task ──────────────────────────────────────────────

/// Spawn a `live-video.exe` child process and start a tokio task that reads
/// its stdout via `live_video::read_message()`.
///
/// Returns the child process handle and the reader task's join handle.
fn spawn_and_wire(
    id: &str,
    args: &[String],
    job: &JobObject,
    registry: &std::sync::Arc<RwLock<StreamRegistry>>,
) -> (Child, JoinHandle<()>) {
    let mut child = Command::new(&args[0])
        .args(&args[1..])
        .args(["--stream-id", id])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", args[0]));

    if let Err(e) = job.assign(&child) {
        log::warn!("[{id}] failed to assign to job object: {e}");
    }

    let stdout = child.stdout.take().expect("stdout must be piped");

    let id_owned = id.to_owned();
    let registry_clone = Arc::clone(registry);

    // Stdout reader: blocking read_message() on a dedicated thread.
    // Uses blocking_write() to push frames — hold time is microseconds per
    // frame, so contention with HTTP readers is negligible.
    let reader_handle = tokio::task::spawn_blocking(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match live_video::read_message(&mut reader) {
                Ok(Some(msg)) => {
                    let mut registry = registry_clone.blocking_write();
                    if let Some(stream) = registry.streams.get_mut(&id_owned) {
                        match msg {
                            Message::CodecParams(params) => {
                                stream.buffer.set_codec_params(params);
                                if stream.status == StreamStatus::Starting {
                                    stream.status = StreamStatus::Running;
                                    log::info!("@{id_owned} running (codec params received)");
                                }
                            }
                            Message::Frame(frame) => {
                                stream.buffer.push_frame(&frame);
                            }
                            Message::Error(e) => {
                                log::error!("@{id_owned} capture error: {e}");
                            }
                        }
                    }
                    drop(registry);
                }
                Ok(None) => {
                    log::info!("@{id_owned} stdout EOF");
                    break;
                }
                Err(e) => {
                    log::error!("@{id_owned} read error: {e}");
                    break;
                }
            }
        }

        // Mark stream as stopped.
        let mut registry = registry_clone.blocking_write();
        if let Some(stream) = registry.streams.get_mut(&id_owned) {
            stream.status = StreamStatus::Stopped;
        }
    });

    (child, reader_handle)
}

/// Generate a short random ID (8 hex chars).
fn short_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{:08x}", t.as_nanos() as u32)
}
