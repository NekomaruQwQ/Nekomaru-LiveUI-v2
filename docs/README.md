# Nekomaru LiveUI

**Low-latency (<100ms) screen capture streaming from DirectX 11 to the browser**

**Status**: Encoding Pipeline Complete | `live-capture` Crate Done | Architecture Refactoring In Progress
**Last Updated**: 2026-02-25
**Hardware**: RTX 5090 | Windows 11

---

## Table of Contents

- [Quick Start](#quick-start)
- [Architecture Overview](#architecture-overview)
- [IPC Wire Protocol](#ipc-wire-protocol)
- [HTTP API](#http-api)
- [Implementation Status](#implementation-status)
- [Performance Metrics](#performance-metrics)
- [Encoding Pipeline Reference](#encoding-pipeline-reference)
- [Bugs Fixed & Learnings](#bugs-fixed--learnings)
- [File Structure](#file-structure)
- [Testing Checklist](#testing-checklist)

---

## Quick Start

```bash
# Build the Rust executables
cargo build --release

# Start the server (serves frontend + manages captures)
cd server && bun run dev

# (Optional) Launch the webview host with locked aspect ratio
cargo run -p live-app

# Or just open http://localhost:3000 in any browser
```

---

## Architecture Overview

### Multi-Executable Design

The project is split into three independently running components. The hard work (GPU capture + hardware encoding) stays in Rust. Everything the user touches (HTTP API, stream buffering, frontend serving) is TypeScript for fast iteration.

```
┌─────────────────────────────────────────────────────────────────┐
│ live-capture.exe  (Rust)                                        │
│                                                                  │
│  One instance per captured window.                               │
│  Spawned by LiveServer as a child process.                       │
│                                                                  │
│  Windows Graphics Capture                                        │
│    ↓                                                             │
│  Resample (scale to target resolution, set viewport)             │
│    ↓                                                             │
│  BGRA → NV12 (ID3D11VideoProcessor, GPU)                        │
│    ↓                                                             │
│  H.264 Encode (NVENC async MFT, hardware)                       │
│    ↓                                                             │
│  Binary frame messages → stdout                                  │
└────────────────────┬────────────────────────────────────────────┘
                     │
                     │ stdout (binary wire protocol)
                     ↓
┌─────────────────────────────────────────────────────────────────┐
│ LiveServer  (TypeScript, Hono on Bun)                            │
│                                                                  │
│  Process Manager                                                 │
│    - Spawns / kills live-capture.exe instances                   │
│    - Reads their stdout, parses binary frames                    │
│                                                                  │
│  Stream Buffer (per capture)                                     │
│    - Circular buffer (~60 frames, ~1 second)                     │
│    - Caches SPS/PPS from IDR frames for new clients              │
│    - Sequence numbering for polling                              │
│                                                                  │
│  HTTP API                                                        │
│    - /streams             → list / create / delete captures      │
│    - /streams/:id/init    → codec params (SPS, PPS, resolution)  │
│    - /streams/:id/frames  → encoded frames (polling)             │
│                                                                  │
│  Frontend Server                                                 │
│    - Proxies to Vite dev server (development)                    │
│    - Serves static build (production)                            │
└────────────────────┬────────────────────────────────────────────┘
                     │
                     │ HTTP (localhost, preconfigured port)
                     ↓
┌─────────────────────────────────────────────────────────────────┐
│ Browser / live-app.exe                                           │
│                                                                  │
│  Any browser works. live-app.exe is an optional thin wry         │
│  webview host that locks the window aspect ratio for streaming.  │
│                                                                  │
│  Frontend (Preact + WebCodecs)                                   │
│    - H264Decoder (avcC descriptor, Annex B → AVCC conversion)    │
│    - StreamRenderer (Canvas rendering, ~60fps polling)           │
│    - Multiple viewers can connect to the same stream             │
└─────────────────────────────────────────────────────────────────┘
```

### Why This Split?

| Concern | Decision | Rationale |
|---------|----------|-----------|
| GPU capture + encoding | Rust (`live-capture`) | Requires `unsafe` Windows APIs, hardware access, zero-copy GPU pipelines. No alternative. |
| HTTP server + stream management | TypeScript (Hono on Bun) | Pure I/O multiplexing — shuttles bytes from child processes to HTTP clients. Dev velocity (hot reload) matters far more than soundness here. |
| Webview host | Rust (`live-app`, optional) | Tiny wry wrapper for aspect-ratio-locked window. Could also just use a browser. |
| IPC | Child process stdout | Zero config, natural lifetime (process death = stream death), trivially testable (`live-capture > dump.bin`). |

### Why Not a Monolith?

The previous monolith (`src/app.rs`) mixed window events, GPU capture, encoding, HTTP protocol handling, and webview hosting in one process. It worked, but:

- **Can't view in a normal browser** (wry custom protocol only)
- **Can't run multiple captures** (single encoding thread)
- **Can't iterate on the server/API** without recompiling Rust
- **Can't develop frontend** without the full Rust app running

---

## IPC Wire Protocol

`live-capture.exe` writes length-prefixed binary messages to stdout. LiveServer reads and parses them.

### Message Format

```
[u8:  message_type]
[u32 LE: payload_length]
[payload_length bytes: payload]
```

### Message Types

#### `0x01` — CodecParams

Sent once after encoder initialization, and again on any IDR frame if parameters change.

```
[u16 LE: width]
[u16 LE: height]
[u16 LE: sps_length]
[sps_length bytes: SPS NAL data]
[u16 LE: pps_length]
[pps_length bytes: PPS NAL data]
```

#### `0x02` — Frame

Sent for every encoded frame.

```
[u64 LE: timestamp_us]
[u8: is_keyframe (0 or 1)]
[u32 LE: num_nal_units]
For each NAL unit:
    [u8: nal_type]
    [u32 LE: data_length]
    [data_length bytes: NAL data with Annex B start code]
```

#### `0xFF` — Error

Non-fatal error. Fatal errors are signaled by process exit.

```
[payload_length bytes: UTF-8 error message]
```

### CLI Interface

```bash
# Spawn a capture for a specific window
live-capture.exe --hwnd 0x1A2B3C --width 1920 --height 1200

# Capture the primary monitor's active window
live-capture.exe --width 1920 --height 1200

# Dump to file for debugging
live-capture.exe --hwnd 0x1A2B3C --width 1920 --height 1200 > capture_dump.bin
```

Logging goes to stderr.

---

## HTTP API

Served by LiveServer (Hono on Bun). Port is preconfigured via environment variable or hardcoded default.

### Stream Management

**`GET /streams`** — List active capture streams.

```json
[
    { "id": "abc123", "windowTitle": "Visual Studio Code", "width": 1920, "height": 1200, "status": "running" }
]
```

**`POST /streams`** — Create a new capture (spawns a `live-capture.exe` instance).

```json
// Request
{ "hwnd": "0x1A2B3C", "width": 1920, "height": 1200 }

// Response
{ "id": "abc123" }
```

**`DELETE /streams/:id`** — Stop and remove a capture (kills the child process).

### Stream Data

**`GET /streams/:id/init`** — Codec parameters for decoder initialization.

```json
{
    "sps": "<base64>",
    "pps": "<base64>",
    "width": 1920,
    "height": 1200
}
```

**`GET /streams/:id/frames?after=N`** — Encoded frames after sequence number N.

```json
{
    "frames": [
        { "sequence": 123, "data": "<base64>", "keyframe": false },
        { "sequence": 124, "data": "<base64>", "keyframe": false }
    ]
}
```

The base64 `data` field contains the same binary frame format as the IPC protocol's Frame message.

---

## Implementation Status

### Completed (`live-capture` crate — `service/capture/`)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **IPC Protocol (lib)** | `service/capture/src/lib.rs` | Done | Wire protocol types (`NALUnit`, `CodecParams`, `FrameMessage`) + serialization/deserialization via `impl Write`/`impl Read`. Round-trip tested. |
| **CLI + Orchestration** | `service/capture/src/main.rs` | Done | `--hwnd`, `--width`, `--height` CLI. Bakery model: capture thread + encoding thread → binary stdout. |
| **D3D11 Helpers** | `service/capture/src/d3d11.rs` | Done | Device creation, texture/view factories (subset of monolith `app/helper.rs`) |
| **Format Converter** | `service/capture/src/converter.rs` | Done | GPU-accelerated BGRA→NV12 via `ID3D11VideoProcessor`. Resolution now parameterized. |
| **H.264 Encoder** | `service/capture/src/encoder.rs` | Done | Async MFT with low-latency settings, NAL parsing. Callbacks passed to `run()` (monomorphized, no `Box<dyn>`). |
| **Encoder Helpers** | `service/capture/src/encoder/helper.rs` | Done | Finds NVIDIA NVENC encoder |
| **Debug Logging** | `service/capture/src/encoder/debug.rs` | Done | Prints supported media types |
| **Resampler** | `service/capture/src/resample.rs` | Done | Scales captured frames with viewport set |
| **Capture** | `service/capture/src/capture.rs` | Done | Windows Graphics Capture wrapper + viewport calculation |

### Completed (Frontend Decoder)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **Frontend Decoder** | `frontend/streamDecoder.ts` | Done | H264Decoder with WebCodecs, avcC descriptor |
| **Frontend Renderer** | `frontend/streamRenderer.tsx` | Done | StreamRenderer with Canvas, ~60fps polling |

### Planned (Architecture Refactoring)

| Component | Location | Status | Notes |
|-----------|----------|--------|-------|
| **LiveServer** | `server/` | Planned | Hono on Bun, process manager + HTTP API + stream buffering |
| **live-app** | `app/` | Planned | Minimal wry webview host, aspect ratio lock |
| **Frontend updates** | `frontend/` | Planned | Point at real HTTP API instead of `stream.localhost` |

---

## Performance Metrics

### Latency Breakdown (Estimated)

| Component | Time | Method |
|-----------|------|--------|
| Capture | 0-16ms | Windows Graphics Capture (1 frame buffer) |
| Resample | 0.5-1ms | GPU shader (fullscreen triangle) |
| GPU Flush + Wait | 5ms | `Flush()` + `sleep(5ms)` |
| BGRA→NV12 | 0.5-1ms | `ID3D11VideoProcessor` |
| GPU Flush | 1-2ms | `Flush()` |
| H.264 Encode | 5-15ms | NVENC hardware encoder |
| NAL Parse | <0.1ms | CPU Annex B parsing |
| IPC (stdout) | <0.1ms | Pipe buffer, same machine |
| HTTP response | <1ms | Localhost |
| **Total** | **13-36ms** | Well under 100ms target |

### Frame Sizes (1920x1200 @ 8 Mbps CBR)

| Frame Type | Size Range | Scenario |
|------------|------------|----------|
| **IDR (keyframe)** | ~67 KB | SPS(27B) + PPS(8B) + full I-frame |
| **P-frame (static)** | 1.5-10 KB | Mostly unchanged screen content |
| **P-frame (typing/scrolling)** | 10-30 KB | Text editing, web browsing |
| **P-frame (high motion)** | 30-50 KB | Video playback, animations |

**Red Flags:**
- 12-byte P-frames → Empty/black frames (viewport bug)
- 9KB IDR → Possible empty first frame

### Encoding Settings

| Setting | Value | Reason |
|---------|-------|--------|
| Profile | H.264 Baseline | No B-frames, WebCodecs compatibility |
| Bitrate | 8 Mbps CBR | Constant for predictable latency |
| Frame Rate | 60 fps | Encoder runs at constant 60fps |
| GOP Size | 120 frames (2 sec) | Fast recovery from packet loss |
| B-frames | 0 | Baseline profile prohibits (low latency) |
| Low Latency Mode | Enabled | `CODECAPI_AVLowLatencyMode = true` |

---

## Encoding Pipeline Reference

### Format Converter (`service/capture/src/converter.rs`)

GPU-accelerated BGRA→NV12 conversion via `ID3D11VideoProcessor`. Hardware H.264 encoders require NV12 input.

Performance: ~0.5-1ms for 1920x1200.

### H.264 Encoder (`service/capture/src/encoder.rs`)

Async Media Foundation Transform (MFT). Runs a blocking event loop:

- `METransformNeedInput` → read from staging texture, convert, feed to encoder
- `METransformHaveOutput` → parse NAL units, write to stdout

NAL unit types: SPS(7) ~27B, PPS(8) ~8B, IDR(5) ~67KB, NonIDR(1) ~1.5-30KB.

### "Bakery Model" (Capture Thread ↔ Encoding Thread)

Within `live-capture.exe`, the capture thread (main) and encoding thread share a staging texture ("the shelf"). The capture thread continuously restocks it with the latest captured frame; the encoding thread reads at a constant 60fps. No channels, no CPU copies — just a shared GPU texture with `Flush()` synchronization.

**Trade-off**: Encoder may encode the same frame twice if capture is slow. Acceptable for live streaming.

---

## Bugs Fixed & Learnings

### Bug #1: Codec API Settings Order

**Problem**: `ICodecAPI::SetValue()` before media types → "parameter is incorrect"

**Fix**: Set media types first, then codec API values. Correct order:
1. Output media type (H.264, resolution, frame rate, bitrate, profile)
2. Input media type (NV12, resolution, frame rate)
3. D3D manager (attach GPU device)
4. Codec API values (B-frames, GOP, latency mode, rate control)
5. Start streaming

### Bug #2: VARIANT Type Mismatch

**Problem**: B-frame count setting failed with `VT_UI4`.

**Fix**: Use `i32` (signed) for all codec API values: `VARIANT::from(0i32)`.

### Bug #3: Missing Viewport → Empty Frames

**Problem**: All P-frames were 12 bytes (black frames). Resampler didn't set viewport → GPU clipped fullscreen triangle → empty output.

**Fix**: Always set `RSSetViewports()` before draw calls.

**Lesson**: D3D11 draw calls require explicit viewport, scissor, and render target setup.

### Bug #4: GPU Synchronization

**Problem**: Encoder reading stale/empty frames from staging texture.

**Fix**: Call `Flush()` after GPU operations + small sleep:
- UI thread after resample: `Flush()` + `sleep(5ms)`
- Encoding thread after NV12 conversion: `Flush()`

**Alternative** (not yet implemented): D3D11 queries/fences for proper synchronization.

---

## File Structure

```
Nekomaru-LiveUI-v2/
├── Cargo.toml                       # Workspace root
│
├── app/                             # live-app.exe — webview host (Rust, wry)
│   ├── Cargo.toml
│   └── src/
│       └── main.rs                  # Opens webview to localhost, locks aspect ratio
│
├── service/
│   └── capture/                     # live-capture.exe + live_capture lib (Rust)
│       ├── Cargo.toml               # Emits both [[bin]] and [lib]
│       └── src/
│           ├── lib.rs               # IPC protocol types + serialization (public API)
│           ├── main.rs              # CLI args, orchestrates capture → encode → stdout
│           ├── d3d11.rs             # D3D11 device + texture/view creation helpers
│           ├── capture.rs           # Windows Graphics Capture wrapper + viewport calc
│           ├── converter.rs         # NV12Converter (BGRA→NV12, GPU, parameterized)
│           ├── encoder.rs           # H264Encoder (async MFT, NAL parsing)
│           ├── encoder/
│           │   ├── helper.rs        # Encoder enumeration (NVIDIA preference)
│           │   └── debug.rs         # Media type logging utilities
│           ├── resample.rs          # BGRA scaling shader
│           └── resample.hlsl        # Fullscreen triangle vertex/pixel shaders
│
├── server/                          # LiveServer — HTTP server (TypeScript, Hono on Bun)
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
│       ├── index.ts                 # Entry point, Hono app setup
│       ├── streams.ts               # Stream management routes
│       ├── process.ts               # Spawn/manage live-capture.exe child processes
│       ├── buffer.ts                # Per-stream circular frame buffer + SPS/PPS cache
│       └── protocol.ts             # Parse binary wire format from stdout
│
└── frontend/                        # Frontend (Preact + Vite)
    ├── package.json
    ├── vite.config.ts
    ├── index.tsx                     # Entry point
    ├── app.tsx                       # Main app with StreamRenderer
    ├── streamDecoder.ts              # H264Decoder (WebCodecs + avcC)
    ├── streamRenderer.tsx            # StreamRenderer (Canvas + polling)
    └── debug.ts                      # Debug flags
```

---

## Testing Checklist

### Encoding Pipeline (Complete)

- [x] Encoder initializes successfully
- [x] Low-latency settings applied (Baseline, CBR, GOP=120)
- [x] SPS/PPS generated on first frame (~27B + 8B)
- [x] IDR frame reasonable size (~67 KB for 1920x1200)
- [x] P-frames vary with screen content (1.5-30 KB)
- [x] Viewport set before resample (no more 12-byte P-frames)
- [x] GPU synchronization working (Flush + sleep)

### Frontend Decoder (Complete)

- [x] WebCodecs `VideoDecoder` initializes with avcC descriptor
- [x] Codec string dynamically built from SPS
- [x] Annex B → AVCC conversion
- [x] Decodes IDR and P-frames successfully
- [x] No memory leaks (`frame.close()` called after render)

### Architecture Refactoring (In Progress)

- [x] `live-capture.exe` runs standalone and writes binary frames to stdout
- [x] IPC wire protocol round-trip tested (Rust serialization + deserialization)
- [ ] Stdout wire protocol parses correctly in TypeScript
- [ ] LiveServer spawns and manages `live-capture.exe` instances
- [ ] HTTP API serves codec params and frame data
- [ ] Multiple browsers can connect to the same stream
- [ ] `live-app.exe` opens webview to localhost with locked aspect ratio
- [ ] Frontend works in both webview and regular browser

### End-to-End (Pending)

- [ ] Video displays in browser
- [ ] Video content matches captured window
- [ ] Latency < 100ms
- [ ] 60fps playback (smooth, no stuttering)
- [ ] No frame drops under normal load
- [ ] Handles long runs (10+ minutes) without memory leak
- [ ] CPU usage reasonable (NVENC should be low CPU)

---

## Known Issues

### 1. Hardcoded NVIDIA Encoder

Only selects encoders with "nvidia" in name. Fails on Intel/AMD.
**Priority**: Low (personal use, RTX 5090).

### 2. ~~Hardcoded Resolution~~ (Fixed)

Resolution is now configurable via `--width` and `--height` CLI args in `live-capture.exe`. The monolith (`src/`) still hardcodes 1920x1200.

### 3. No Error Recovery

Encoding errors cause panic (`.unwrap()` / `.expect()`).
**Priority**: Medium. Should skip frames and log to stderr instead.

---

## Dependencies

### live-capture (Rust)

```toml
[dependencies]
nkcore = { features = ["debug"] }   # Common utilities, macros, euclid re-export
ngd3dcompile = { ... }              # Compile-time HLSL shader compilation
winrt-capture = { ... }             # Windows Graphics Capture wrapper
log = "0.4"
pretty_env_logger = "0.5"
pretty-name = "0.4"
widestring = "1.2"
windows = { version = "0.62", features = [
    "Graphics_Capture",
    "Graphics_DirectX_Direct3D11",
    "Win32_Graphics_Direct3D11",
    "Win32_Media_MediaFoundation",
    "Win32_System_Com",
    # ...
]}
```

### live-app (Rust)

```toml
[dependencies]
wry = "0.53"
winit = { version = "0.30", features = ["rwh_06"] }
```

### LiveServer (TypeScript)

```json
{
    "dependencies": {
        "hono": "^4.x"
    }
}
```

---

## References

### Windows API
- [Media Foundation Transforms](https://learn.microsoft.com/en-us/windows/win32/medfound/media-foundation-transforms)
- [H.264 Video Encoder](https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder)
- [ID3D11VideoProcessor](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nn-d3d11-id3d11videoprocessor)
- [Async MFTs](https://learn.microsoft.com/en-us/windows/win32/medfound/asynchronous-mfts)

### Web Standards
- [WebCodecs API](https://w3c.github.io/webcodecs/)
- [H.264 Specification](https://www.itu.int/rec/T-REC-H.264)
- [ISO 14496-15 (AVC File Format)](https://www.iso.org/standard/55980.html)

---

**Author**: Nekomaru
**Co-Pilot**: Claude
**Hardware**: NVIDIA GeForce RTX 5090
**License**: Personal Use Only
