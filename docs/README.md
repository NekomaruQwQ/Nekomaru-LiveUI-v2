# Nekomaru LiveUI

**Low-latency (<100ms) screen capture streaming from DirectX 11 to the browser**

**Status**: Encoding Pipeline Complete | `live-capture` Crate Done | LiveServer Implemented | Frontend Integrated | UI Redesigned (JetBrains Islands) | Auto Window Selector Integrated | End-to-End Testing Next
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
cd server && bun run index.ts

# Create a capture (replace HWND with your target window)
curl -X POST http://localhost:3000/streams \
    -H 'Content-Type: application/json' \
    -d '{"hwnd":"0x1A2B3C", "width":1920, "height":1200}'

# Open the frontend in any browser
# http://localhost:3000

# (Optional) Launch the webview host with locked aspect ratio
cargo run -p live-app
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
│    - /streams/auto        → start / stop / status auto-selector  │
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
│  Frontend (React + WebCodecs)                                    │
│    - Typed API client via Hono RPC (hc)                          │
│    - H264Decoder (avcC descriptor, Annex B → AVCC conversion)    │
│    - StreamRenderer (Canvas rendering, ~60fps polling)           │
│    - Auto-select mode by default (polls selector status)         │
│    - Manual fallback: window picker, create/stop captures        │
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

# List capturable windows as JSON
live-capture.exe --enumerate-windows

# Get the current foreground window as JSON (used by auto-selector)
live-capture.exe --foreground-window

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
    { "id": "abc123", "hwnd": "0x1A2B3C", "width": 1920, "height": 1200, "status": "running" }
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
        { "sequence": 123, "data": "<base64>" },
        { "sequence": 124, "data": "<base64>" }
    ]
}
```

The base64 `data` field contains a pre-serialized binary payload (timestamp + NAL units). Keyframe status is inferred from NAL unit types on the frontend.

**`GET /streams/windows`** — List capturable windows (one-shot spawn of `live-capture.exe --enumerate-windows`).

### Auto Window Selector

**`GET /streams/auto`** — Get auto-selector status.

```json
{ "active": true, "currentStreamId": "abc123", "currentHwnd": "0x1A2B3C" }
```

**`POST /streams/auto`** — Start the auto-selector (idempotent). Polls the foreground window every 2 seconds and automatically switches captures when the foreground matches the include list.

**`DELETE /streams/auto`** — Stop the auto-selector and destroy its managed stream.

---

## Implementation Status

### Completed (`live-capture` crate — `service/capture/`)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **IPC Protocol (lib)** | `service/capture/src/lib.rs` | Done | Wire protocol types (`NALUnit`, `CodecParams`, `FrameMessage`) + serialization/deserialization via `impl Write`/`impl Read`. Round-trip tested. |
| **CLI + Orchestration** | `service/capture/src/main.rs` | Done | `--hwnd`, `--width`, `--height` CLI. `--enumerate-windows` and `--foreground-window` one-shot modes. Bakery model: capture thread + encoding thread → binary stdout. |
| **D3D11 Helpers** | `service/capture/src/d3d11.rs` | Done | Device creation, texture/view factories (subset of monolith `app/helper.rs`) |
| **Format Converter** | `service/capture/src/converter.rs` | Done | GPU-accelerated BGRA→NV12 via `ID3D11VideoProcessor`. Resolution now parameterized. |
| **H.264 Encoder** | `service/capture/src/encoder.rs` | Done | Async MFT with low-latency settings, NAL parsing. Callbacks passed to `run()` (monomorphized, no `Box<dyn>`). |
| **Encoder Helpers** | `service/capture/src/encoder/helper.rs` | Done | Finds NVIDIA NVENC encoder |
| **Debug Logging** | `service/capture/src/encoder/debug.rs` | Done | Prints supported media types |
| **Resampler** | `service/capture/src/resample.rs` | Done | Scales captured frames with viewport set |
| **Capture** | `service/capture/src/capture.rs` | Done | Windows Graphics Capture wrapper + viewport calculation |
| **Window Enumeration** | `crates/enumerate-windows/src/lib.rs` | Done | `enumerate_windows()` lists capturable windows. `get_foreground_window()` returns current foreground window info. |

### Completed (Frontend Decoder)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **Frontend Decoder** | `frontend/src/streamDecoder.ts` | Done | H264Decoder with WebCodecs, avcC descriptor |
| **Frontend Renderer** | `frontend/src/streamRenderer.tsx` | Done | StreamRenderer with Canvas, ~60fps polling |

### Completed (Webview Host)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **live-app** | `app/src/main.rs` | Done | Non-resizable 1920x1200 wry webview via nkcore/winit event loop. Opens devtools in debug builds. Loads `http://localhost:3000`. |

### Completed (LiveServer — `server/`)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **Entry Point** | `server/index.ts` | Done | Hono app + Vite dev server (middleware mode) on single `node:http` port. Routes `/streams` → API, everything else → Vite. SIGINT/SIGTERM cleanup. |
| **Stream API** | `server/api.ts` | Done | Hono routes: `GET/POST/DELETE /streams`, `GET/POST/DELETE /streams/auto`, `GET /streams/:id/init`, `GET /streams/:id/frames?after=N`, `GET /streams/windows`. Zod validation on POST. |
| **Process Manager** | `server/process.ts` | Done | Spawns `live-capture.exe` via `Bun.spawn`. Wires stdout → ProtocolParser → StreamBuffer. Tracks lifecycle (starting → running → stopped). stderr forwarded with `[capture:id]` prefix. |
| **Protocol Parser** | `server/protocol.ts` | Done | Push-based incremental binary parser. Handles partial reads, greedy parse loop. Mirrors Rust wire format exactly. |
| **Frame Buffer** | `server/buffer.ts` | Done | Per-stream circular buffer (60 frames). Multi-viewer safe (no drain). Pre-serializes frames on push. Skips to first keyframe for new clients. |
| **Constants** | `server/common.ts` | Done | Port (`LIVE_PORT` env or 3000), exe path, buffer capacity. |
| **Auto Selector** | `server/selector.ts` | Done | `LiveWindowSelector` class. Polls foreground window every 2s via `live-capture.exe --foreground-window`. Include/exclude list matching. Auto-creates/destroys streams on window switch. |

### Completed (Frontend — React + Hono RPC)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **API Client** | `frontend/src/api.ts` | Done | Typed Hono RPC client via `hc<ApiType>("/streams")`. Imports server route type for end-to-end type safety. `fetchInit()` retries on 503 with exponential backoff. |
| **Decoder** | `frontend/src/streamDecoder.ts` | Done | Uses `fetchInit()` from API client (handles 503 retry). WebCodecs H264Decoder with avcC descriptor. |
| **Renderer** | `frontend/src/streamRenderer.tsx` | Done | Polls `/streams/:id/frames` via typed Hono RPC client at ~60fps. Canvas rendering with GPU memory management. Styled with Tailwind. |
| **App** | `frontend/src/app.tsx` | Done | Stream management UI. JetBrains Islands dark theme (Tailwind utility classes, no Emotion). Auto-select mode by default (polls `/streams/auto`). Manual fallback: window picker, create/stop captures. |
| **Entry Point** | `frontend/index.tsx` | Done | React 19 `createRoot()` (migrated from Preact). |
| **Vite Config** | `frontend/vite.config.ts` | Done | `@vitejs/plugin-react-swc`, `root: "."`, `@` and `@shadcn` aliases. |

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
├── Cargo.toml                       # Workspace root (members: ".", "app", "service/capture")
├── src/                             # LiveUI monolith binary (legacy, workspace member ".")
│   ├── main.rs
│   ├── app.rs
│   ├── app/
│   │   ├── capture_selector.rs
│   │   └── helper.rs
│   ├── stream.rs
│   ├── constant.rs
│   ├── converter.rs
│   ├── encoder.rs
│   ├── encoder/
│   │   ├── helper.rs
│   │   └── debug.rs
│   ├── resample.rs
│   └── resample.hlsl
│
├── app/                             # live-app.exe — webview host (Rust, wry + nkcore/winit)
│   ├── Cargo.toml
│   └── src/
│       └── main.rs                  # Non-resizable 1920x1200 window, loads localhost:3000
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
│   ├── biome.json                   # Biome formatter/linter config
│   ├── index.ts                     # Entry point: Hono + Vite on single node:http port
│   ├── common.ts                    # Constants (port, exe path, buffer capacity)
│   ├── api.ts                       # Hono routes for /streams/* (exports ApiType for frontend RPC)
│   ├── process.ts                   # Spawn/manage live-capture.exe child processes
│   ├── buffer.ts                    # Per-stream circular frame buffer + SPS/PPS cache
│   ├── protocol.ts                  # Incremental binary wire protocol parser
│   └── selector.ts                  # Auto window selector (polls foreground, switches captures)
│
└── frontend/                        # Frontend (React + Vite + Tailwind)
    ├── package.json
    ├── tsconfig.json
    ├── vite.config.ts               # Vite root = ., aliases: @→src
    ├── biome.json                   # Biome formatter/linter config
    ├── index.html
    ├── index.tsx                    # Entry point (React 19 createRoot)
    ├── global.css                   # CSS vars, dark gradient background, layout
    ├── global.tailwind.css          # Tailwind base config (shadcn theme vars)
    ├── debug.ts                     # Debug flags
    ├── src/                         # Application source (aliased as @/)
    │   ├── api.ts                   # Hono RPC client (imports ApiType from server)
    │   ├── app.tsx                  # Main app: JetBrains Islands UI, stream management
    │   ├── streamDecoder.ts         # H264Decoder (WebCodecs + avcC)
    │   └── streamRenderer.tsx       # StreamRenderer (Canvas + polling)
    └── public/
        └── img/
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

### Architecture Refactoring (Implementation Complete — Needs E2E Testing)

- [x] `live-capture.exe` runs standalone and writes binary frames to stdout
- [x] IPC wire protocol round-trip tested (Rust serialization + deserialization)
- [x] Stdout wire protocol parses correctly in TypeScript (`server/protocol.ts`)
- [x] LiveServer spawns and manages `live-capture.exe` instances (`server/process.ts`)
- [x] HTTP API serves codec params and frame data (`server/api.ts`)
- [x] Server API type-exported for Hono RPC (`ApiType`)
- [x] Multiple browsers can connect to the same stream (circular buffer, no drain)
- [x] `live-app.exe` opens webview to localhost with locked aspect ratio
- [x] Frontend uses typed Hono RPC client (`hc<ApiType>`)
- [x] Frontend points at real HTTP API (`/streams/:id/init`, `/streams/:id/frames`)
- [x] Frontend creates/selects/stops captures via API (no hardcoded stream ID)
- [x] Decoder retries on 503 (stream starting up) with exponential backoff
- [x] Migrated from Preact to React 19
- [x] UI redesigned: JetBrains Islands dark theme, Emotion CSS replaced with Tailwind utilities, shadcn removed
- [x] Auto window selector integrated (server-side `selector.ts`, one-shot `--foreground-window` CLI)
- [x] Frontend starts in auto-select mode by default, polls `/streams/auto` for stream ID changes
- [x] Manual fallback mode (window picker) available when auto-select is stopped
- [ ] Frontend works in both webview and regular browser

### End-to-End (Pending)

- [ ] Video displays in browser
- [ ] Video content matches captured window
- [ ] Latency < 100ms
- [ ] 60fps playback (smooth, no stuttering)
- [ ] No frame drops under normal load
- [ ] Handles long runs (10+ minutes) without memory leak
- [ ] CPU usage reasonable (NVENC should be low CPU)
- [ ] Auto-selector switches capture when foreground window changes
- [ ] Auto-selector skips windows not in include list
- [ ] Frontend tracks stream ID changes during auto-select switches

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
nkcore = { features = ["winit"] }  # Event loop integration (run_app_with)
winit = { version = "0.30", features = ["rwh_06"] }
wry = "0.54"
```

### LiveServer (TypeScript)

```json
{
    "dependencies": {
        "hono": "^4.x",
        "@hono/node-server": "^1.x",
        "@hono/zod-validator": "^0.7.x",
        "zod": "^4.x",
        "immer": "^11.x",
        "ts-pattern": "^5.x",
        "remeda": "^2.x"
    },
    "devDependencies": {
        "vite": "^7.x"
    }
}
```

### Frontend (TypeScript)

```json
{
    "dependencies": {
        "react": "^19.x",
        "react-dom": "^19.x",
        "@emotion/css": "^11.x (installed, no longer used in UI — migrated to Tailwind)",
        "tailwindcss": "^4.x",
        "hono": "^4.x (hono/client for RPC)",
        "zod": "^4.x",
        "immer": "^11.x",
        "lucide-react": "^0.563",
        "@fortawesome/react-fontawesome": "^3.x",
        "@radix-ui/react-*": "installed (shadcn removed, primitives kept for future use)"
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
