# Nekomaru LiveUI: H.264 Video Streaming

**Low-latency (<100ms) screen capture streaming from DirectX11 to WebView**

**Status**: ✅ Encoding Pipeline Complete | ⏳ Streaming Protocol Pending
**Last Updated**: 2025-12-11
**Hardware**: RTX 5090 | Windows 11

---

## Table of Contents

- [Quick Start](#quick-start)
- [Architecture Overview](#architecture-overview)
- [Implementation Status](#implementation-status)
- [Performance Metrics](#performance-metrics)
- [Key Components](#key-components)
- [Bugs Fixed & Learnings](#bugs-fixed--learnings)
- [Next Steps](#next-steps)
- [File Structure](#file-structure)
- [Testing Checklist](#testing-checklist)

---

## Quick Start

### What Works Now ✅

```bash
cargo build --release
cargo run
```

**You'll see:**
- Window captures your IDE/desktop
- Encoding thread processes frames at 60fps
- Console logs NAL units: `SPS(27B) + PPS(8B) + IDR(67KB)` then `P-frames(1.5-30KB)`
- StreamManager buffers encoded frames (60 frame circular buffer)

**What's NOT working yet:**
- No video display in webview (custom protocol not wired up)
- No frontend decoder (WebCodecs implementation pending)

---

## Architecture Overview

### "Bakery Model" Design

We use a producer-consumer pattern where the UI thread continuously "restocks" a staging texture that the encoding thread reads from at a constant rate.

```
┌─────────────────────────────────────────────────────────────────┐
│ UI THREAD (Producer / "The Cook")                               │
│                                                                  │
│  RedrawRequested Event Loop (60+ fps)                           │
│    ↓                                                             │
│  Windows Graphics Capture Update                                │
│    ↓                                                             │
│  Resample Pass (scale to 1920x1200, set viewport!)              │
│    ↓                                                             │
│  Write to "Shelf" → staging_bgra8 texture                       │
│    ↓                                                             │
│  GPU Flush() + sleep(5ms)  ← ensure GPU completes               │
│    ↓                                                             │
│  request_redraw() → loop                                        │
└────────────────────┬────────────────────────────────────────────┘
                     │
                     ↓ (shared texture)
┌─────────────────────────────────────────────────────────────────┐
│ "THE SHELF" (staging_bgra8)                                     │
│  - Always contains latest captured frame                        │
│  - D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE       │
│  - Read by encoding thread, written by UI thread                │
└────────────────────┬────────────────────────────────────────────┘
                     │
                     ↓ (read at 60fps)
┌─────────────────────────────────────────────────────────────────┐
│ ENCODING THREAD (Consumer / "The Customer")                     │
│                                                                  │
│  Async MFT Event Loop (blocking GetEvent)                       │
│    ↓                                                             │
│  METransformNeedInput → read from shelf                         │
│    ↓                                                             │
│  BGRA → NV12 Conversion (ID3D11VideoProcessor)                  │
│    ↓                                                             │
│  GPU Flush() ← ensure conversion completes                      │
│    ↓                                                             │
│  Feed NV12 to H.264 Encoder (WMF async MFT)                     │
│    ↓                                                             │
│  METransformHaveOutput → parse NAL units                        │
│    ↓                                                             │
│  Push to StreamManager (lock-free queue)                        │
└─────────────────────────────────────────────────────────────────┘
                     │
                     ↓
┌─────────────────────────────────────────────────────────────────┐
│ STREAM MANAGER                                                   │
│  - 60-frame circular buffer (crossbeam::ArrayQueue)             │
│  - Caches SPS/PPS from IDR frames                               │
│  - Sequence numbering for long-polling                          │
└────────────────────┬────────────────────────────────────────────┘
                     │
                     ↓ (TODO)
┌─────────────────────────────────────────────────────────────────┐
│ CUSTOM PROTOCOL HANDLER (stream://)                             │
│  - stream://init → JSON {sps, pps, width, height}               │
│  - stream://stream?after=N → Binary frame data                  │
└────────────────────┬────────────────────────────────────────────┘
                     │
                     ↓ (TODO)
┌─────────────────────────────────────────────────────────────────┐
│ FRONTEND (WebCodecs + Canvas)                                   │
│  - H.264 decoder (VideoDecoder API)                             │
│  - Canvas rendering with frame.close()                          │
└─────────────────────────────────────────────────────────────────┘
```

**Why This Model?**
- **Simpler**: No `mpsc` channels, no complex event passing
- **Natural**: UI updates shelf whenever ready, encoder reads at constant rate
- **Efficient**: GPU operations are async anyway, no CPU→CPU copies
- **Trade-off**: Encoder may encode same frame twice if UI is slow (acceptable for live streaming)

---

## Implementation Status

### ✅ Completed (Backend Encoding Pipeline)

| Component | File | Status | Notes |
|-----------|------|--------|-------|
| **Format Converter** | `src/converter.rs` | ✅ Done | GPU-accelerated BGRA→NV12 via `ID3D11VideoProcessor` |
| **H.264 Encoder** | `src/encoder.rs` | ✅ Done | Async MFT with low-latency settings, NAL parsing |
| **Encoder Helpers** | `src/encoder/helper.rs` | ✅ Done | Finds NVIDIA NVENC encoder (hardcoded) |
| **Debug Logging** | `src/encoder/debug.rs` | ✅ Done | Prints supported media types |
| **Stream Manager** | `src/stream.rs` | ✅ Done | Lock-free queue, SPS/PPS caching |
| **Integration** | `src/app.rs` | ✅ Done | Spawns encoding thread, wires callbacks |
| **Resampler** | `src/resample.rs` | ✅ Done | Scales captured frames with viewport set |

### ⏳ Pending (Streaming & Frontend)

| Component | File | Status | Next Action |
|-----------|------|--------|-------------|
| **Protocol Handler** | `src/app.rs` | ⏳ Commented out | Uncomment `handle_stream_request()`, wire to WebView |
| **Frontend Decoder** | `frontend/decoder.ts` | ⏳ Not started | Implement `H264Decoder` with WebCodecs |
| **Frontend Renderer** | `frontend/renderer.tsx` | ⏳ Not started | Implement `VideoRenderer` with Canvas |
| **Integration** | `frontend/app.tsx` | ⏳ Partial | Replace empty Card with VideoRenderer |

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
| **Total** | **12-35ms** | ✅ Well under 100ms target |

### Frame Sizes (1920x1200 @ 8 Mbps CBR)

| Frame Type | Size Range | Scenario |
|------------|------------|----------|
| **IDR (first frame)** | ~67 KB | SPS(27B) + PPS(8B) + full I-frame |
| **P-frame (static)** | 1.5-10 KB | Mostly unchanged screen content |
| **P-frame (typing/scrolling)** | 10-30 KB | Text editing, web browsing |
| **P-frame (high motion)** | 30-50 KB | Video playback, animations |

**Red Flags:**
- 🚨 12-byte P-frames → Empty/black frames (viewport bug)
- 🚨 9KB IDR → Possible empty first frame

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

## Key Components

### 1. Format Converter (`src/converter.rs`)

**Purpose**: GPU-accelerated BGRA→NV12 conversion
**Why**: Hardware H.264 encoders require NV12 input

```rust
pub struct NV12Converter {
    device: ID3D11VideoDevice,
    device_context: ID3D11VideoContext,
    processor: ID3D11VideoProcessor,
    enumerator: ID3D11VideoProcessorEnumerator,
}

impl NV12Converter {
    pub fn convert(
        &mut self,
        bgra_texture: &ID3D11Texture2D,
        nv12_texture: &ID3D11Texture2D
    ) -> anyhow::Result<()> {
        // Creates input/output views
        // Calls VideoProcessorBlt()
        // Returns immediately (GPU async)
    }
}
```

**Performance**: ~0.5-1ms for 1920x1200

### 2. H.264 Encoder (`src/encoder.rs`)

**Purpose**: Encode NV12 frames to H.264 NAL units
**Type**: Async Media Foundation Transform (MFT)

```rust
pub struct H264Encoder {
    mf_transform: IMFTransform,
    mf_event_generator: IMFMediaEventGenerator,
    config: H264EncoderConfig,
    // ...
}

pub struct H264EncoderConfig {
    pub frame_size: Size2D<u32>,
    pub frame_rate: u32,
    pub bitrate: u32,
    pub frame_source_callback: Box<dyn FnMut() -> ID3D11Texture2D>,
    pub frame_target_callback: Box<dyn FnMut(Vec<NALUnit>)>,
}
```

**Event Loop**:
```rust
pub fn run(mut self) {
    loop {
        let event = self.mf_event_generator.GetEvent(default())?;
        match event.GetType()? {
            METransformNeedInput => self.process_input()?,   // Read from shelf → encode
            METransformHaveOutput => self.process_output()?, // Parse NAL → callback
            _ => {}
        }
    }
}
```

**NAL Unit Types**:
- `SPS(7)` - Sequence Parameter Set (~27 bytes)
- `PPS(8)` - Picture Parameter Set (~8 bytes)
- `IDR(5)` - Keyframe (~67 KB)
- `NonIDR(1)` - P-frame (~1.5-30 KB)

### 3. Stream Manager (`src/stream.rs`)

**Purpose**: Thread-safe circular buffer for encoded frames

```rust
pub struct StreamManager {
    frame_queue: Arc<crossbeam::queue::ArrayQueue<StreamFrame>>,
    codec_params: Arc<Mutex<Option<CodecParams>>>,
    sequence_counter: AtomicU64,
}

pub struct StreamFrame {
    pub sequence: u64,
    pub nal_units: Vec<NALUnit>,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
}
```

**Features**:
- **Capacity**: 60 frames (~1 second buffer at 60fps)
- **Overflow**: Drops oldest frame (live streaming behavior)
- **SPS/PPS caching**: Extracts from IDR frames for new clients
- **Long-polling**: `wait_for_frame(after_sequence, timeout)`

### 4. Integration (`src/app.rs`)

**UI Thread** (`on_window_event`):
```rust
WindowEvent::RedrawRequested => {
    self.main_capture.update();
    let source_texture = self.main_capture.frame_buffer();

    // Set viewport (CRITICAL!)
    let viewport = D3D11_VIEWPORT { /* ... */ };
    self.device_context.RSSetViewports(Some(&[viewport]));

    // Resample to staging texture ("restock shelf")
    self.resampler.resample(&self.device_context, &source_view, &staging_rtv);

    // Flush GPU to ensure completion
    unsafe { self.device_context.Flush(); }
    thread::sleep(Duration::from_millis(5));

    // Loop
    self.main_window.request_redraw();
}
```

**Encoding Thread** (spawned in `LiveApp::new()`):
```rust
thread::Builder::new()
    .name("Encoding Thread".to_owned())
    .spawn(move || {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;

        H264Encoder::new(&device, H264EncoderConfig {
            frame_size: (1920, 1200).into(),
            frame_rate: 60,
            bitrate: 8_000_000,

            frame_source_callback: Box::new(move || {
                // Read from "shelf" and convert
                nv12_converter.convert(&staging_bgra8, &staging_nv12)?;
                staging_nv12.clone()
            }),

            frame_target_callback: Box::new(move |nal_units| {
                // Push to stream manager
                stream_manager.push_frame(nal_units)?;
            }),
        })?.run();
    })?;
```

---

## Bugs Fixed & Learnings

### 🐛 Bug #1: Codec API Settings Order

**Problem**: Setting `ICodecAPI::SetValue()` before media types → "parameter is incorrect" error

**Root Cause**: Encoder can't validate codec settings without knowing resolution, frame rate, format

**Fix**: Correct order:
```rust
// ✅ CORRECT ORDER:
1. Set output media type (H.264, resolution, frame rate, bitrate, profile)
2. Set input media type (NV12, resolution, frame rate)
3. Set D3D manager (attach GPU device)
4. Set codec API values (B-frames, GOP, latency mode, rate control)
5. Start streaming (BEGIN_STREAMING, START_OF_STREAM)
```

**Location**: `src/encoder.rs:127-184`

---

### 🐛 Bug #2: VARIANT Type Mismatch

**Problem**: B-frame count setting failed despite docs saying `VT_UI4`

**Root Cause**: Unclear, but most codec APIs prefer signed integers

**Fix**: Use `i32` for all codec API values:
```rust
// ❌ FAILED:
VARIANT::from(0u32)

// ✅ WORKS:
VARIANT::from(0i32)
```

**Note**: B-frame setting may still fail on NVIDIA (Baseline profile makes it redundant anyway)

**Location**: `src/encoder.rs:169-179`

---

### 🐛 Bug #3: Missing Viewport → Empty Frames

**Problem**: All P-frames were exactly 12 bytes (black frames compressing to nothing)

**Root Cause**: Resampler didn't set viewport → GPU clipped fullscreen triangle → empty output texture

**Symptoms**:
- First frame: IDR ~9KB (should be 67KB)
- P-frames: 12 bytes (should be 1.5-30KB)
- No variation in frame sizes

**Fix**: Always set viewport before draw calls:
```rust
let viewport = D3D11_VIEWPORT {
    TopLeftX: 0.0,
    TopLeftY: 0.0,
    Width: 1920.0,
    Height: 1200.0,
    MinDepth: 0.0,
    MaxDepth: 1.0,
};
unsafe { device_context.RSSetViewports(Some(&[viewport])); }
```

**Location**: `src/app.rs:216-227`

**Lesson**: D3D11 draw calls require explicit viewport, scissor, and render target setup!

---

### 🐛 Bug #4: GPU Synchronization

**Problem**: Encoder reading stale/empty frames from staging texture

**Root Cause**: GPU operations are async; `sleep()` on CPU doesn't guarantee GPU completion

**Fix**: Call `Flush()` after GPU operations + small sleep for execution time:
```rust
// UI thread after resample:
unsafe { self.device_context.Flush(); }
thread::sleep(Duration::from_millis(5));

// Encoding thread after NV12 conversion:
unsafe { self.device_context.Flush(); }
```

**Why Both?**:
- UI thread: Ensure resample completes before encoder reads
- Encoding thread: Ensure conversion completes before encoder processes

**Alternative** (not implemented): Use D3D11 queries/fences for proper synchronization

**Location**: `src/app.rs:231`, `src/converter.rs:125`

---

### 💡 Design Choice: "Bakery Model"

**Alternative Considered**: Event-driven with `mpsc` channels

```rust
// REJECTED APPROACH:
UI Thread:
  capture.update()
  resample()
  channel.send(texture_copy)  // ← CPU copy overhead

Encoding Thread:
  let texture = channel.recv()?
  convert(texture)
  encode(texture)
```

**Why We Chose Bakery Model:**
1. **No CPU copies**: Encoder reads GPU texture directly
2. **Simpler code**: No channel management, no complex synchronization
3. **GPU-native**: Leverages async GPU operations naturally
4. **Acceptable trade-off**: Re-encoding same frame occasionally is fine for live streaming

**When Event-Driven Makes Sense:**
- Recording to file (every frame must be unique)
- Variable frame rates (capture at 30fps, encode at 60fps)
- Multiple consumers (need to distribute same frame to multiple encoders)

---

## Next Steps

### Immediate (Streaming Protocol)

**Goal**: Wire up `stream://` custom protocol handler

**Tasks**:
1. ⏳ Uncomment `handle_stream_request()` in `src/app.rs:249`
2. ⏳ Add `stream_manager` reference to `LiveApp` struct (currently `_stream_manager`)
3. ⏳ Wire up WebView custom protocol:
   ```rust
   let stream_manager_clone = Arc::clone(&stream_manager);
   let main_webview = WebViewBuilder::new()
       .with_url("http://localhost:9688/")
       .with_custom_protocol("stream", move |request| {
           handle_stream_request(&stream_manager_clone, request)
       })
       .build(&main_window)?;
   ```
4. ⏳ Test endpoints:
   - `stream://init` → Should return JSON `{sps, pps, width, height}`
   - `stream://stream?after=0` → Should return binary frame data

**Test with curl** (once wired):
```bash
# Get codec params
curl stream://init

# Get first frame (won't work from curl, need browser)
# (Custom protocols only work in webview context)
```

---

### Short-Term (Frontend Decoder)

**Goal**: Implement WebCodecs H.264 decoder

**File**: `frontend/decoder.ts`

**Key Components**:
```typescript
export class H264Decoder {
    private decoder: VideoDecoder;

    async init() {
        // Fetch SPS/PPS from stream://init
        // Build avcC descriptor (ISO 14496-15)
        // Configure VideoDecoder
    }

    async decodeFrame(frameData: StreamFrameData) {
        // Parse binary frame format
        // Create EncodedVideoChunk
        // decoder.decode(chunk)
    }
}

export function parseStreamFrame(buffer: Uint8Array): StreamFrameData {
    // Parse binary format:
    // [u64: timestamp] [u32: num_nals] [nal_type, length, data]...
}

function buildAvcCDescriptor(sps: Uint8Array, pps: Uint8Array): Uint8Array {
    // ISO 14496-15 format for WebCodecs
}
```

**Codec String**: `avc1.42001f` (Baseline profile, level 3.1)

---

### Medium-Term (Frontend Renderer)

**Goal**: Render decoded frames to Canvas

**File**: `frontend/renderer.tsx`

**Component**:
```typescript
export function VideoRenderer() {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const decoderRef = useRef<H264Decoder | null>(null);

    useEffect(() => {
        const decoder = new H264Decoder((frame: VideoFrame) => {
            const canvas = canvasRef.current;
            const ctx = canvas.getContext('2d');

            // Resize canvas if needed
            canvas.width = frame.displayWidth;
            canvas.height = frame.displayHeight;

            // Draw frame
            ctx.drawImage(frame, 0, 0);

            // CRITICAL: Release GPU memory
            frame.close();
        });

        decoder.init().then(() => startStreamLoop(decoder));

        return () => decoder.close();
    }, []);

    return <canvas ref={canvasRef} />;
}

async function startStreamLoop(decoder: H264Decoder) {
    let lastSequence = 0;
    while (true) {
        const response = await fetch(`stream://stream?after=${lastSequence}`);
        const frameData = parseStreamFrame(await response.arrayBuffer());
        await decoder.decodeFrame(frameData);
        lastSequence = parseInt(response.headers.get('X-Sequence'));
    }
}
```

**Integration** (`frontend/app.tsx`):
```typescript
<Card className={/* ... */}>
    <VideoRenderer />  {/* Replace empty Card */}
</Card>
```

---

### Future Enhancements

**Lower Priority**:
- 🔮 Dynamic encoder selection (fallback Intel QSV / AMD VCE if NVIDIA not available)
- 🔮 Adaptive bitrate based on scene complexity (QP feedback loop)
- 🔮 Multiple capture sources (picture-in-picture, multi-monitor)
- 🔮 Recording to MP4 file (add ffmpeg or manual MP4 muxer)
- 🔮 Text/image overlays before encoding
- 🔮 Dynamic resolution (respond to window resize)

---

## File Structure

```
LiveUI/
├── Cargo.toml               # Dependencies: crossbeam, base64, serde_json, wry, windows
├── docs/
│   └── README.md            # ← YOU ARE HERE
│
├── src/
│   ├── main.rs              # Entry point, logger init
│   ├── app.rs               # ✅ Main app, window events, encoding thread
│   ├── app/
│   │   └── helper.rs        # ✅ Device creation, ApplicationHandler
│   ├── capture.rs           # ✅ Windows Graphics Capture wrapper
│   ├── converter.rs         # ✅ NV12Converter (BGRA→NV12 GPU)
│   ├── encoder.rs           # ✅ H264Encoder (async MFT, NAL parsing)
│   ├── encoder/
│   │   ├── helper.rs        # ✅ Encoder enumeration, media type config
│   │   └── debug.rs         # ✅ Media type logging utilities
│   ├── resample.rs          # ✅ BGRA scaling shader (with viewport!)
│   └── stream.rs            # ✅ StreamManager (lock-free queue)
│
└── frontend/
    ├── app.tsx              # ⏳ Main React app (needs VideoRenderer)
    ├── decoder.ts           # ⏳ TODO: H264Decoder, parseStreamFrame
    └── renderer.tsx         # ⏳ TODO: VideoRenderer component
```

**Note**: No `encoding_thread.rs` - logic is inlined in `app.rs::LiveApp::new()`

---

## Testing Checklist

### ✅ Encoding Pipeline (Complete)

- [x] Encoder initializes successfully
- [x] Low-latency settings applied (Baseline profile, CBR, GOP=120, low latency mode)
- [x] B-frame setting attempts but may fail on NVIDIA (Baseline prohibits anyway)
- [x] SPS/PPS generated on first frame (~27B + 8B) ✅
- [x] IDR frame reasonable size (~67 KB for 1920x1200) ✅
- [x] P-frames vary with screen content (1.5-30 KB) ✅
- [x] NAL units logged correctly (types and sizes) ✅
- [x] StreamManager receives and buffers frames ✅
- [x] Viewport set before resample (no more 12-byte P-frames!) ✅
- [x] GPU synchronization working (Flush + sleep) ✅

### ⏳ Streaming Protocol (Pending)

- [ ] `stream://init` returns valid JSON `{sps, pps, width, height}`
- [ ] `stream://stream?after=N` returns binary frame data
- [ ] Headers set correctly: `X-Sequence`, `X-Timestamp`, `X-Keyframe`
- [ ] Long-polling timeout works (100ms)
- [ ] Sequence numbers increment monotonically
- [ ] Keyframe flag matches NAL unit type (IDR=true, NonIDR=false)
- [ ] Base64 encoding of SPS/PPS works
- [ ] Binary frame format parses correctly on frontend

### ⏳ Frontend Decoder (Pending)

- [ ] WebCodecs `VideoDecoder` initializes with avcC descriptor
- [ ] Codec string correct: `avc1.42001f` (Baseline level 3.1)
- [ ] Decodes IDR frames successfully
- [ ] Decodes P-frames successfully
- [ ] No memory leaks (`frame.close()` called after render)
- [ ] Browser DevTools shows no errors
- [ ] Console logs frame timestamps and types

### ⏳ End-to-End (Pending)

- [ ] Video displays in webview
- [ ] Video content matches captured window
- [ ] Latency < 100ms (measure with screen flash test)
- [ ] 60fps playback (smooth motion, no stuttering)
- [ ] No frame drops under normal load
- [ ] Handles window resize gracefully
- [ ] Handles long runs (10+ minutes) without memory leak
- [ ] CPU usage reasonable (NVENC should be low CPU)
- [ ] GPU usage visible in Task Manager

---

## Known Issues

### 1. Hardcoded NVIDIA Encoder ⚠️
**Location**: `src/encoder/helper.rs:107`
**Issue**: Only selects encoders with "nvidia" in name
**Impact**: Fails on Intel/AMD systems
**Priority**: Low (personal use, RTX 5090 exclusive)
**Workaround**: Manually edit to match your GPU vendor

### 2. Hardcoded Resolution ⚠️
**Location**: `src/app.rs:63` (`STREAM_FRAME_SIZE = 1920x1200`)
**Issue**: Fixed resolution, can't adapt to different monitors
**Impact**: Wrong aspect ratio on other displays
**Priority**: Low (matches current monitor)
**Workaround**: Edit constant to match your resolution

### 3. No Error Recovery ⚠️
**Issue**: Encoding errors cause panic (`.unwrap()` / `.expect()`)
**Impact**: App crashes on encoding failure instead of graceful degradation
**Priority**: Medium (should skip frames and log instead)
**Example**: `src/encoder.rs:263` - `.unwrap()` on process_input

---

## Dependencies

```toml
[dependencies]
base64 = "0.22"              # Base64 encoding for SPS/PPS in JSON
crossbeam = "0.8"            # Lock-free ArrayQueue for StreamManager
log = "0.4"
pretty-name = "0.4"          # Method name formatting for errors
pretty_env_logger = "0.5"
serde_json = "1"             # JSON serialization for stream://init
widestring = "1.2"           # UTF-16 string handling for Windows APIs
winit = { version = "0.30", features = ["rwh_06"] }
wry = { version = "0.53", features = ["protocol"] }  # Custom protocol support
windows = { version = "0.61", features = [
    "Graphics_Capture",
    "Graphics_DirectX_Direct3D11",
    "Win32_Graphics_Direct3D11",
    "Win32_Media_MediaFoundation",
    "Win32_System_Com",
    # ... (full list in Cargo.toml)
]}
```

**No additional dependencies needed** for current implementation ✅

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

### Community Resources
- [webm-wasm](https://github.com/GoogleChromeLabs/webm-wasm) - Similar approach with VP8/VP9
- [Broadway.js](https://github.com/mbebenita/Broadway) - Pure JS H.264 decoder (slower, for compatibility)

---

**Last Updated**: 2025-12-11
**Author**: Nekomaru
**Co-Pilot**: Claude Sonnet 4.5
**Hardware**: NVIDIA GeForce RTX 5090 (don't @ me, Intel/AMD users 😎)
**License**: Personal Use Only
