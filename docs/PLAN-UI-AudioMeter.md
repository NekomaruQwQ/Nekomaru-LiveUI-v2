# Audio Level Meter in ActionPanel

## Context

The right-most column of the UI (`ActionPanel`, 40px wide) is currently empty. Audio is already streaming through a PCM AudioWorklet. We can tap into the audio graph to drive a vertical level meter — a nice visual indicator that audio is alive.

## Approach: AnalyserNode + CSS scaleY

Insert a Web Audio `AnalyserNode` between the worklet and destination. The AnalyserNode provides time-domain sample data for free — no worklet changes, no server changes. A React component polls it at `requestAnimationFrame` rate and drives a CSS-animated vertical bar.

```
workletNode → AnalyserNode → ctx.destination
                  ↓ (getFloatTimeDomainData)
            LevelMeter component (rAF loop)
```

## Files

### 1. `frontend/src/audio/index.tsx` — expose AnalyserNode

- Add `onAnalyser?: (node: AnalyserNode | null) => void` prop to `AudioStream`.
- Pass it through to `startAudioLoop`.
- Create `AnalyserNode({ fftSize: 2048 })`, wire `workletNode → analyser → destination`.
- Call `onAnalyser(analyser)` after wiring, `onAnalyser(null)` on cleanup.
- When audio is disabled (404), the callback is never called — meter stays off.

### 2. `frontend/src/components/level-meter.tsx` — new file

Vertical bar meter driven by `requestAnimationFrame`:

- **Input**: `analyser: AnalyserNode | null` prop.
- **Computation**: Peak (max |sample|) from `getFloatTimeDomainData()`. Fast attack, exponential decay (`0.93`). Separate peak-hold line with 1.5s hold + slow decay.
- **Rendering**: Direct DOM manipulation via refs (no React state at 60fps).
  - Background track: `w-2 rounded-full bg-white/5`, full height.
  - Filled bar: same width, `transform: scaleY(level)` with `transform-origin: bottom`. Gradient from bottom to top: `--color-green` → `--color-yellow` (70%) → `--color-red` (95%).
  - Peak hold line: 2px horizontal line at peak level, `--foreground` at 60% opacity.
- **Off state** (`analyser === null`): bar at `scaleY(0)`, peak line hidden. Empty track stays visible.

### 3. `frontend/src/app.tsx` — wire it up

- `const [analyser, setAnalyser] = useState<AnalyserNode | null>(null)`
- `<AudioStream onAnalyser={setAnalyser} />`
- Pass `analyser` to `ActionPanel` → `<LevelMeter analyser={analyser} />`

## Design Decisions

| Decision | Why |
|---|---|
| AnalyserNode (not manual RMS from polling data) | Standard Web Audio pattern, zero worklet changes, runs at audio render rate |
| Callback prop (not Context/global store) | One producer, one consumer — simplest wiring |
| CSS scaleY (not Canvas) | 24px usable width, single bar — GPU-composited transform is the cheapest animation path |
| Peak (not RMS) | Standard for visual meters — instant response to transients, smoother decay via exponential smoothing |
| Direct DOM refs (not React state) | 60fps animation should not trigger React renders |

## Edge Cases

- **Audio disabled**: `onAnalyser` never called → meter shows empty track
- **Autoplay suspended**: AnalyserNode returns silence → meter shows nothing until user interaction
- **Tab hidden**: rAF paused by browser → meter pauses (fine, nobody watching)
- **Peak > 1.0**: Clamped to 1.0

## Verification

1. Start server with `LIVE_AUDIO=1`, open browser
2. Meter should animate in response to system audio
3. Stop audio → bar should decay to zero smoothly
4. Start server without `LIVE_AUDIO` → meter should show empty track (no errors in console)
