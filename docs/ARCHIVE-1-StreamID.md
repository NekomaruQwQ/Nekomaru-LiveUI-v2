# Plan: Server-Managed Streams with Well-Known IDs

## Context

The frontend currently has ~200 lines of stream management logic split across `capture.ts` (auto-selector polling, window enumeration, manual stream create/destroy) and `youtube-music.ts` (window discovery, crop stream lifecycle). This makes the control panel unable to manage what the UI shows, and creates unnecessary complexity in the browser.

We move all stream selection and lifecycle management to the server, giving the frontend two hardcoded stream IDs: `"main"` and `"youtube-music"`. The frontend becomes a pure viewer.

## Key Design Decision: Generation Counter

**Problem**: When the auto-selector switches windows, the `"main"` stream's underlying capture process is replaced. Since the stream ID stays `"main"`, the frontend's `useEffect([streamId])` won't retrigger. The decoder must reinitialize because the new window may have different resolution/codec params.

**Solution**: Add a `generation` field to `CaptureStream` (starts at 1, increments on each replacement). The frames endpoint includes `generation` in every response. The frontend's stream loop detects generation changes and reinitializes the decoder.

## Server Changes

### 1. `server/process.ts` — Add generation + replaceStream

Add `generation: number` to `CaptureStream` interface.

Extract the process-spawning and stdout/stderr wiring from `spawnCapture()` into a helper `spawnAndWire(stream, args, label)` that operates on an existing `CaptureStream` object. This avoids duplicating the wiring logic between create and replace paths.

Then:

- **Existing `createStream` / `createCropStream`**: Build a new `CaptureStream` with `generation: 1`, insert into the Map, call `spawnAndWire`. Unchanged external behavior (random IDs for manual use via live-control).

- **New `replaceStream(id, hwnd, width, height)`**: Look up stream by `id`. Kill old process, reset buffer (clear frames, codec params, reset sequence to 1), bump `generation`, set `status: "starting"`, call `spawnAndWire` with new args. The Map entry is never removed — no gap where `getStream(id)` returns `undefined`. If no stream with that `id` exists, create one with that ID (makes it idempotent).

- **New `replaceCropStream(id, hwnd, cropWidth, cropHeight, cropAlign)`**: Same pattern for crop mode.

Buffer reset: add a `reset()` method to `StreamBuffer` that clears `frames`, `codecParams`, and resets `writeIndex`/`count`/`nextSequence`.

### 2. `server/api.ts` — Add generation to responses

- `GET /` (list streams): include `generation` in each stream object.
- `GET /:id/frames`: include `generation` at the top level of the response.
- All other endpoints unchanged.

### 3. `server/selector.ts` — Use fixed "main" ID

Replace the current destroy-then-create pattern:
```
// Before: generates new random ID each time
proc.destroyStream(this.currentStreamId);
const stream = proc.createStream(hwndStr, ...);
this.currentStreamId = stream.id;  // changes every switch

// After: ID is always "main"
proc.replaceStream("main", hwndStr, DEFAULT_WIDTH, DEFAULT_HEIGHT);
```

- Remove `currentStreamId` field (it's always `"main"`).
- `stop()`: still destroys the `"main"` stream (user explicitly stopped the selector).
- `SelectorStatus`: replace `currentStreamId` with literal `"main"` when active.

### 4. New `server/youtube-music.ts` — YTM Manager

New `YouTubeMusicManager` class, structurally similar to `LiveWindowSelector`:

- Polls `enumerateWindows()` (already exported from `process.ts`) every 5s.
- Finds window with title starting `"YouTube Music"`.
- When found and no `"youtube-music"` stream exists: `replaceCropStream("youtube-music", hwnd, "full", 128, "bottom")`.
- When window hwnd changes (YTM restarted): same `replaceCropStream` call (idempotent).
- When window disappears: `destroyStream("youtube-music")`.
- Tracks `lastKnownHwnd` to detect changes.
- Exports singleton `ytmManager` with `start()` / `stop()` / `status()`.

### 5. `server/index.ts` — Auto-start both managers

- Import `ytmManager`.
- Call `selector.start()` and `ytmManager.start()` after the HTTP server is listening.
- SIGINT/SIGTERM cleanup: call both `.stop()` methods.

## Frontend Changes

### 6. `frontend/src/stream/index.tsx` — Generation-aware stream loop

The `startStreamLoop` function gains generation tracking:

```ts
let currentGeneration: number | null = null;

// Inside the fetch loop:
const data = await res.json();

if (currentGeneration !== null && data.generation !== currentGeneration) {
    // Stream replaced — reinitialize decoder
    decoder.close();
    decoder = new H264Decoder(streamId, renderCallback);
    await decoder.init();  // fetchInit retries on 503
    lastSequence = 0;
}
currentGeneration = data.generation;
```

This requires refactoring the component slightly: the stream loop needs to own decoder creation (currently the component creates the decoder and passes it in). Move decoder lifecycle fully into `startStreamLoop`, which accepts `streamId`, `canvas`, `ctx`, and an `AbortSignal` for cleanup.

Also: extend the error handling to treat 404 as retriable (stream not yet created), not fatal. Currently 404 increments `consecutiveErrors` and eventually breaks the loop. Instead, on 404, sleep and retry (the server may create the stream soon).

### 7. `frontend/src/stream/decoder.ts` — fetchInit retries on 404

Currently `fetchInit` retries on 503 but throws on 404. Change to also retry on 404 (stream doesn't exist yet — the server may create it momentarily). Same exponential backoff, longer max retries to allow for startup.

### 8. Delete `frontend/src/capture.ts`

All 150 lines removed. Auto-selector activation, status polling, window enumeration, manual stream create/destroy — all gone.

### 9. Delete `frontend/src/youtube-music.ts`

All 87 lines removed. Window discovery and crop stream lifecycle moved to server.

### 10. New `frontend/src/streams.ts` — Simple availability hook

```ts
export function useStreamStatus() {
    // Polls GET /streams every 2-3s
    // Returns { hasMain: boolean, hasYouTubeMusic: boolean }
}
```

Used by `app.tsx` to decide whether to show the YouTube Music island (vs placeholder). The main stream always renders `<StreamRenderer streamId="main" />` regardless — it handles unavailability internally via retry.

### 11. Simplify `frontend/src/app.tsx`

Before: 145 lines with `useCaptureControl`, `useYouTubeMusicStream`, `MainCaptureContent`, `WindowPicker`, auto/manual mode switching.

After: ~50 lines. Hardcoded `streamId="main"` and `streamId="youtube-music"`. YouTube Music island shown/hidden via `useStreamStatus()`. Remove `WindowPicker`, `MainCaptureContent`, all control buttons (Stop Auto, Stop Capture, Pick Window, Auto). The sidebar keeps the greeting text.

## Unchanged

- `server/protocol.ts`, `server/common.ts` — no changes
- `core/live-capture/` — no changes
- `core/live-control/` — treats stream IDs as opaque strings, works unchanged
- `core/live-app/` — no changes
- `frontend/src/api.ts` — Hono RPC client unchanged (fewer calls made)

## Implementation Order

1. **Server data model** (process.ts: generation, buffer reset, replaceStream) — testable via curl
2. **Server API** (api.ts: generation in responses) — testable via curl
3. **Server selector** (selector.ts: fixed "main" ID) — test by switching windows, verify ID stays "main"
4. **Server YTM manager** (youtube-music.ts: new module) — test by opening/closing YouTube Music
5. **Server auto-start** (index.ts) — verify both managers start on boot
6. **Frontend stream loop** (stream/index.tsx: generation tracking + 404 handling)
7. **Frontend decoder** (stream/decoder.ts: 404 retry in fetchInit)
8. **Frontend cleanup** (delete capture.ts, youtube-music.ts, create streams.ts, simplify app.tsx)

Steps 1-5 can be tested server-side without touching the frontend (old frontend still works). Steps 6-8 are the frontend cutover.

## Verification

1. `curl GET /streams` — confirm "main" and "youtube-music" appear with generation field
2. Switch foreground windows — confirm "main" stream's generation increments, ID stays "main"
3. Open/close YouTube Music — confirm "youtube-music" stream appears/disappears
4. Open frontend — confirm main capture renders with no controls
5. Switch foreground windows — confirm video seamlessly switches (decoder reinits on generation change, no page reload)
6. YouTube Music island appears/disappears based on window presence
