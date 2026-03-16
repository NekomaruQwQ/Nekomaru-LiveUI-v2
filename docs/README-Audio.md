# Audio Streaming Subsystem

**Low-latency raw PCM audio capture from WASAPI → HTTP polling → browser AudioWorklet playback, synchronized to the video stream.**

**Last Updated**: 2026-03-16

---

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Data Flow](#data-flow)
- [IPC Wire Protocol](#ipc-wire-protocol)
- [HTTP API](#http-api)
- [A/V Synchronization](#av-synchronization)
- [Frontend Audio Pipeline](#frontend-audio-pipeline)
- [Configuration](#configuration)
- [File Map](#file-map)
- [Bandwidth & Latency Budget](#bandwidth--latency-budget)
- [Testing](#testing)
- [Design Decisions](#design-decisions)

---

## Overview

The audio subsystem captures audio from a named WASAPI input device (e.g. a
virtual loopback microphone) and streams raw PCM s16le over HTTP to the browser.
It runs in parallel with the video pipeline but is architecturally independent —
audio is global (one device, not per-window) while video is per-stream.

The full path from microphone to speaker:

```
WASAPI device → live-audio.exe → stdout IPC → LiveServer → HTTP API → browser AudioWorklet
```

## Architecture

Three layers, mirroring the video pipeline:

```
┌──────────────────────────────────────────────────────────────────────┐
│  Rust: live-audio.exe                                                │
│  WASAPI shared-mode capture → s16le re-chunking → binary IPC stdout  │
└──────────────────────┬───────────────────────────────────────────────┘
                       │ stdout (binary IPC: AudioParams + AudioFrame)
                       │ stderr (env_logger text → server log)
┌──────────────────────▼───────────────────────────────────────────────┐
│  Server: AudioManager (Bun/TypeScript)                               │
│  AudioProtocolParser → AudioBuffer (circular, 100 chunks)            │
│  HTTP API: GET /init, GET /chunks?after=N                            │
└──────────────────────┬───────────────────────────────────────────────┘
                       │ HTTP polling (~16ms interval)
┌──────────────────────▼───────────────────────────────────────────────┐
│  Frontend: <AudioStream /> (React)                                   │
│  Fetch chunks → A/V sync gate → AudioWorklet (ring buffer → output)  │
└──────────────────────────────────────────────────────────────────────┘
```

### Key differences from video

| Aspect | Video | Audio |
|--------|-------|-------|
| Scope | Per-window (stream ID) | Global (one device) |
| Encoding | H.264 via NVENC | None (raw PCM s16le) |
| Keyframe gating | Yes (IDR required to start) | No (all chunks independent) |
| Buffer capacity | 60 frames (~1s at 60fps) | 100 chunks (~1s at 10ms/chunk) |
| Generation counter | Yes (stream replacement) | No (single process) |

## Data Flow

### 1. Rust capture (`live-audio.exe`)

1. Opens the named WASAPI capture device in shared mode
2. Queries the device's native mix format via `GetMixFormat()`
3. If the device provides f32le (common for WASAPI shared mode), converts to s16le in-process
4. Writes one `AudioParams` message to stdout (sample rate, channels, bit depth)
5. Enters the capture loop:
   - Polls `IAudioCaptureClient::GetBuffer()` every ~5ms
   - Accumulates samples into fixed-size 10ms chunks (480 samples at 48kHz)
   - Writes each chunk as an `AudioFrame` message with a wall-clock timestamp
6. Exits cleanly on stdout broken pipe (server killed the process)

### 2. Server (`AudioManager`)

1. `audioManager.start()` spawns `live-audio.exe --device <name>`
2. stdout is piped through `AudioProtocolParser` (push-based incremental binary parser)
3. Parsed messages are dispatched:
   - `AudioParams` → cached in `AudioBuffer` for the `/init` endpoint
   - `AudioFrame` → pre-serialized and pushed into the circular buffer
4. stderr is piped through the same grouped log renderer as video capture

### 3. Frontend (`<AudioStream />`)

1. Fetches `GET /api/v1/audio/init` (retries on 503 until params arrive)
2. Creates an `AudioContext` at the device's native sample rate
3. Loads the `PcmWorkletProcessor` module via `audioWorklet.addModule()`
4. Polls `GET /api/v1/audio/chunks?after=N` at ~16ms intervals
5. For each chunk, checks the A/V sync controller before posting to the worklet
6. The worklet converts s16le → f32, writes to a ring buffer, and outputs via `process()`

## IPC Wire Protocol

Same envelope as the video protocol (`live-capture`):

```
[u8:  message_type]
[u32 LE: payload_length]
[payload_length bytes: payload]
```

Audio message types use the `0x1x` range to avoid collision with video (`0x01`–`0x02`):

### AudioParams (`0x10`)

Sent once after device initialization.

```
[u32 LE: sample_rate]    e.g. 48000
[u8:     channels]       e.g. 2
[u8:     bits_per_sample] e.g. 16
```

Total payload: 6 bytes.

### AudioFrame (`0x11`)

One chunk of raw PCM audio.

```
[u64 LE: timestamp_us]   wall-clock microseconds since Unix epoch
[N bytes: pcm_data]       interleaved s16le samples
```

At 48kHz stereo with 10ms chunks: N = 480 samples × 2 channels × 2 bytes = 1920 bytes.
Total payload: 8 + 1920 = 1928 bytes per chunk.

### Error (`0xFF`)

Raw UTF-8 error description string. Shared discriminant with the video protocol.

## HTTP API

Mounted at `/api/v1/audio` on the Hono app.

### `GET /init`

Returns audio format parameters as JSON.

**Success (200)**:
```json
{
  "sampleRate": 48000,
  "channels": 2,
  "bitsPerSample": 16
}
```

**Not ready (503)**: The capture process hasn't sent `AudioParams` yet. Frontend retries.

### `GET /chunks?after=N`

Returns audio chunks with sequence number > N as a binary blob.

**Binary layout** (all little-endian):
```
[u32: num_chunks]
per chunk:
  [u32: sequence]
  [u32: payload_length]
  [payload_length bytes: payload]
```

Each chunk's payload is the pre-serialized `[u64 LE: timestamp_us][PCM bytes]`.

The frontend extracts `timestamp_us` from the first 8 bytes for A/V sync, then
posts the remaining PCM bytes to the AudioWorklet.

## A/V Synchronization

Audio arrives faster than video because it has no encoding step (raw PCM vs
H.264 NVENC). The sync strategy is **audio paces itself to video**:

### `AVSyncController` (singleton, `frontend/src/audio/sync.ts`)

- **Video side**: `StreamRenderer` calls `avSync.reportVideoTimestamp(ts)` each
  time a decoded `VideoFrame` is rendered. The timestamp is the wall-clock value
  from the capture process (microseconds since Unix epoch).

- **Audio side**: `AudioStream` calls `avSync.shouldRelease(audioTimestampUs)`
  for each chunk before posting it to the worklet. A chunk is released when:
  ```
  audioTimestampUs <= latestVideoTimestampUs + 20ms
  ```

- **Edge case**: When no video timestamp has been reported (value is `0n`), all
  audio is released unconditionally. This prevents audio from being permanently
  held if the video stream hasn't started yet.

### Why 20ms threshold?

Accounts for measurement jitter (HTTP polling intervals, event loop scheduling)
and the fact that video and audio timestamps come from independent
`SystemTime::now()` calls on the capture host. On WLAN with ~5ms jitter, 20ms
provides sufficient headroom without introducing perceptible audio delay.

### Timestamp source

Both `live-capture.exe` and `live-audio.exe` use the same wall clock:
```rust
SystemTime::now().duration_since(UNIX_EPOCH).as_micros() as u64
```
This makes their timestamps directly comparable for frontend sync.

## Frontend Audio Pipeline

### AudioWorklet (`frontend/src/audio/worklet.ts`)

Runs on the dedicated audio rendering thread (not the main thread).

- **Input**: `MessagePort` receives `{ type: "pcm", samples: Int16Array, channels }` from the main thread
- **Ring buffer**: ~50ms capacity (2400 frames at 48kHz). Stores f32 samples (converted from s16le on write).
- **Output**: `process()` pulls 128 frames per call from the ring buffer into the output channels
- **Underrun**: Outputs silence (zeros) — no glitch artifacts

Why 50ms ring buffer? Small enough to keep latency low, large enough to absorb
jitter from the HTTP polling interval (~16ms) and WLAN variability (~5ms).

### Browser autoplay policy

`AudioContext` starts in `"suspended"` state due to browser autoplay policy.
The component registers `click` and `keydown` listeners that call
`audioContext.resume()` on first user interaction.

## Configuration

All audio configuration lives in `server/common.ts`:

| Constant | Value | Purpose |
|----------|-------|---------|
| `audioExePath` | `../target/debug/live-audio.exe` | Path to the Rust binary |
| `audioDeviceName` | `"Loopback L + R (Focusrite USB Audio)"` | WASAPI device friendly name |

The device name is passed to `live-audio.exe` via `--device`. The Rust binary
requires this argument (hard error if missing). Use `--list-devices` to enumerate
available WASAPI capture devices:

```bash
live-audio.exe --list-devices
```

### Tuning constants

| Location | Constant | Default | Purpose |
|----------|----------|---------|---------|
| `main.rs` | `CHUNK_DURATION_MS` | 10 | PCM chunk size (ms) |
| `main.rs` | `POLL_SLEEP_MS` | 5 | WASAPI buffer poll interval (ms) |
| `audio.ts` | `AUDIO_BUFFER_CAPACITY` | 100 | Server circular buffer size (chunks) |
| `audio-api.ts` | — | — | No tuning constants (stateless) |
| `index.tsx` | `POLL_INTERVAL_MS` | 16 | Frontend HTTP poll interval (ms) |
| `index.tsx` | `INIT_RETRY_MS` | 500 | Retry delay for `/init` 503s (ms) |
| `worklet.ts` | `RING_CAPACITY_FRAMES` | 2400 | AudioWorklet ring buffer (frames, ~50ms) |
| `sync.ts` | `SYNC_THRESHOLD_US` | 20000 | A/V sync tolerance (μs) |

## File Map

### Rust (`core/live-audio/`)

| File | Purpose |
|------|---------|
| `Cargo.toml` | Crate manifest (deps: windows, clap, env_logger, log, widestring) |
| `src/lib.rs` | IPC wire protocol: message types, serialization, deserialization, tests |
| `src/main.rs` | WASAPI capture loop: device enumeration, format detection, f32→s16 conversion, chunking |

### Server (`server/`)

| File | Purpose |
|------|---------|
| `audio-protocol.ts` | Push-based incremental binary parser for `0x10`/`0x11` audio IPC messages |
| `audio-buffer.ts` | Circular buffer (100 chunks), pre-serializes on push, no keyframe gating |
| `audio.ts` | Singleton `AudioManager`: spawns process, wires stdout/stderr, lifecycle |
| `audio-api.ts` | Hono sub-router: `GET /init` (JSON) + `GET /chunks?after=N` (binary) |
| `common.ts` | `audioExePath` and `audioDeviceName` constants (edited, not new) |
| `index.ts` | Mounts audio API, starts/stops manager in lifecycle hooks (edited, not new) |

### Frontend (`frontend/src/audio/`)

| File | Purpose |
|------|---------|
| `worklet.ts` | AudioWorklet processor: ring buffer, s16le→f32, silence on underrun |
| `sync.ts` | A/V sync controller: timestamp comparison, 20ms threshold |
| `index.tsx` | `<AudioStream />` component: init fetch, chunk polling, worklet wiring |

### Frontend (edited)

| File | Change |
|------|--------|
| `app.tsx` | Mounts `<AudioStream />` at app root |
| `stream/index.tsx` | Reports video timestamps to `avSync` for A/V sync |

## Bandwidth & Latency Budget

### Bandwidth

```
48000 Hz × 2 channels × 16 bits × 10ms chunks
= 48000 × 2 × 2 bytes = 192,000 bytes/sec
= 1.536 Mbit/s
```

For comparison, video is ~8 Mbit/s (CBR H.264). Audio adds ~19% to the total
bandwidth — acceptable for WLAN streaming.

### Latency breakdown (approximate)

| Stage | Latency |
|-------|---------|
| WASAPI buffer fill | 10ms (one chunk) |
| WASAPI poll interval | 0–5ms |
| IPC (stdout pipe) | <1ms |
| Server buffer | 0ms (immediate push) |
| HTTP poll interval | 0–16ms |
| WLAN round trip | ~5ms |
| AudioWorklet ring buffer | 0–50ms (jitter absorption) |
| **Total** | **~30–90ms** |

The bottleneck is the frontend polling interval + worklet ring buffer. The
actual perceived latency depends on how full the ring buffer is at any given
moment — in steady state it should be near the low end.

## Testing

### 1. Rust binary standalone

```bash
# List available capture devices
live-audio.exe --list-devices

# Capture to stdout (produces binary output — will look like gibberish)
live-audio.exe --device "Loopback L + R (Focusrite USB Audio)"
# Kill with Ctrl+C after a few seconds
```

### 2. Server API

```bash
# Start the server
cd server && bun run index.ts

# Check audio params (should return JSON after ~1 second)
curl http://localhost:3000/api/v1/audio/init

# Check chunk flow (should return >4 bytes)
curl -s http://localhost:3000/api/v1/audio/chunks?after=0 | wc -c
```

### 3. Frontend playback (on remote PC)

Open the browser on the remote machine (not localhost — avoids audio feedback loop).
Check browser console for:
- `AudioStream: params 48000Hz 2ch 16-bit`
- Click the page once to satisfy browser autoplay policy
- Audio should play from the captured device

### 4. Resilience

- **Tab close/reopen**: Audio should restart cleanly (new `AudioContext`, new poll loop)
- **Process crash**: Server logs the exit, frontend gets empty chunk responses (no crash)
- **Device unavailable**: `live-audio.exe` exits with error, server logs `process exited with code 1`

## Design Decisions

### Why raw PCM instead of a compressed codec?

At 1.536 Mbit/s, raw PCM is only ~19% of the video bandwidth (8 Mbit/s). On a
local WLAN link this is negligible. Compression (Opus, AAC) would add:
- Encoding latency (~5–20ms depending on frame size)
- Decode complexity in the browser (MediaSource Extensions or WebCodecs)
- Additional failure modes (codec initialization, bitrate negotiation)

Raw PCM keeps the pipeline simple and latency minimal.

### Why a separate binary instead of integrating into `live-capture.exe`?

Audio capture is global (one device) while video capture is per-window. The
audio device doesn't change when the window selector switches targets. A separate
process keeps the concerns cleanly separated and avoids complicating the
per-window lifecycle in `process.ts`.

### Why HTTP polling instead of WebSocket?

Matches the existing video transport pattern. The video pipeline already achieves
<100ms latency with HTTP polling at ~60fps. Audio polling at ~16ms intervals
(~62 polls/sec) provides comparable responsiveness. WebSocket would save HTTP
overhead per request but adds connection management complexity for minimal gain.

### Why AudioWorklet instead of ScriptProcessorNode?

`ScriptProcessorNode` is deprecated and runs on the main thread — it would
compete with React rendering and HTTP polling for CPU time.
`AudioWorkletProcessor.process()` runs on a dedicated audio rendering thread,
ensuring glitch-free playback independent of main thread load.

### Why 10ms chunks?

- **Smaller (1ms)**: Too many IPC messages, HTTP requests, and buffer management
  overhead for negligible latency improvement.
- **Larger (50–100ms)**: Adds perceptible latency to the audio path, making A/V
  sync harder and the audio feel sluggish.
- **10ms**: Standard for VoIP and real-time audio. At 48kHz stereo s16le, each
  chunk is 1920 bytes — small enough for low overhead, large enough to amortize
  IPC and HTTP costs.

### Why audio syncs to video (not the other way around)?

Video has higher latency (H.264 encoding + decoding) and is the primary content.
Audio has no encoding step and arrives faster. Holding audio until the video
clock catches up is simpler and more robust than trying to speed up or slow down
video to match audio. The 20ms sync threshold accounts for clock jitter without
introducing perceptible delay.
