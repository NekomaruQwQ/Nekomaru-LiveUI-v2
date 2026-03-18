// KPM (keystrokes-per-minute) meter component.
//
// Receives KPM values via WebSocket push from /api/v1/ws/kpm, computes
// peak hold + decay on the frontend for smooth animation, and renders
// a vertical VU-style meter in the ActionPanel.

import { useState, useEffect, useRef } from "react";
import { KeyboardIcon } from "lucide-react";

// ── Constants ────────────────────────────────────────────────────────────────

/// KPM value that maps to 100% bar height.
const MAX_KPM = 480;

/// Power curve exponent for the height mapping.
/// < 1.0 compresses the top end and expands the lower range, making
/// moderate typing (50-300 KPM) more visually interesting.
const CURVE_EXPONENT = 0.7;

/// Peak hold duration before decay begins (ms).
const PEAK_HOLD_MS = 1500;

/// Duration of the linear decay from peak to current (ms).
const PEAK_DECAY_MS = 500;

// ── Hook ─────────────────────────────────────────────────────────────────────

interface KpmState {
    kpm: number;
    peak: number;
}

/// Streams KPM values via WebSocket and computes peak hold + decay locally.
function useKpm(): KpmState | null {
    const [state, setState] = useState<KpmState | null>(null);

    // Peak tracking refs (not state — avoids re-renders on every tick).
    const peakRef = useRef(0);
    const peakTimeRef = useRef(0);

    useEffect(() => {
        const abort = new AbortController();

        void kpmWsLoop(abort.signal, (kpm) => {
            if (kpm == null) { setState(null); return; }

            const now = performance.now();

            // Update peak hold.
            if (kpm >= peakRef.current) {
                peakRef.current = kpm;
                peakTimeRef.current = now;
            } else {
                const elapsed = now - peakTimeRef.current;
                if (elapsed > PEAK_HOLD_MS) {
                    const decayProgress = Math.min(
                        (elapsed - PEAK_HOLD_MS) / PEAK_DECAY_MS, 1);
                    peakRef.current = peakRef.current + (kpm - peakRef.current) * decayProgress;
                }
            }

            setState({ kpm, peak: Math.round(peakRef.current) });
        });

        return () => abort.abort();
    }, []);

    return state;
}

// ── WebSocket reconnect loop ────────────────────────────────────────────────

const INITIAL_DELAY_MS = 100;
const MAX_DELAY_MS = 5000;

/// Connect to the KPM WebSocket with auto-reconnect and exponential backoff.
/// Calls `onValue` for each received KPM update (null = process not running).
async function kpmWsLoop(
    signal: AbortSignal,
    onValue: (kpm: number | null) => void,
): Promise<void> {
    let delay = INITIAL_DELAY_MS;

    while (!signal.aborted) {
        try {
            const connected = await runOneKpmConnection(signal, onValue);
            // Reset backoff on successful connection (the WS opened and
            // eventually closed normally — not a connection failure).
            if (connected) delay = INITIAL_DELAY_MS;
        } catch {
            // Connection failed — fall through to backoff.
        }

        if (signal.aborted) break;
        await sleep(delay);
        delay = Math.min(delay * 2, MAX_DELAY_MS);
    }
}

/// Open a single WS connection to /api/v1/ws/kpm and process messages.
/// Returns true if the connection was successfully established (for backoff
/// reset), false if it failed before opening.
function runOneKpmConnection(
    signal: AbortSignal,
    onValue: (kpm: number | null) => void,
): Promise<boolean> {
    return new Promise<boolean>((resolve, reject) => {
        if (signal.aborted) { resolve(false); return; }

        const proto = location.protocol === "https:" ? "wss:" : "ws:";
        const ws = new WebSocket(`${proto}//${location.host}/api/v1/ws/kpm`);

        const onAbort = () => ws.close();
        signal.addEventListener("abort", onAbort, { once: true });

        ws.onopen = () => {
            // Connection established — messages will flow until close.
        };

        ws.onmessage = (ev: MessageEvent) => {
            if (typeof ev.data === "string") {
                const data = JSON.parse(ev.data) as { kpm: number | null };
                onValue(data.kpm);
            }
        };

        ws.onclose = () => {
            signal.removeEventListener("abort", onAbort);
            resolve(true);
        };

        ws.onerror = () => {
            signal.removeEventListener("abort", onAbort);
            reject(new Error("WebSocket error"));
        };
    });
}

function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}

// ── Height mapping ───────────────────────────────────────────────────────────

/// Map a KPM value to a 0–100 percentage using a power curve.
function kpmToPercent(kpm: number): number {
    const clamped = Math.min(Math.max(kpm, 0), MAX_KPM);
    return (clamped / MAX_KPM) ** CURVE_EXPONENT * 100;
}

// ── Component ────────────────────────────────────────────────────────────────

/// Vertical VU-style KPM meter with peak hold marker.
///
/// Renders nothing if the KPM endpoint returns 404 (process not running).
/// At zero KPM, shows an empty meter ("quiet studio" aesthetic).
export function KpmMeter() {
    const state = useKpm();

    // Not available — render empty panel.
    if (!state) return null;

    const barPercent = kpmToPercent(state.kpm);
    const peakPercent = kpmToPercent(state.peak);

    return (
        <div className="flex! flex-col items-center w-full h-full gap-1">
            {/* Meter body */}
            <div className="kpm-meter flex-1 w-full relative">
                {/* LED segment overlay (decorative dark lines) */}
                <div className="kpm-segments absolute inset-0" />

                {/* Realtime bar — lower visual weight */}
                <div
                    className="kpm-bar absolute inset-x-0 bottom-0 rounded-sm"
                    style={{ height: `${barPercent}%` }}
                />

                {/* Peak hold marker — the hero element */}
                {state.peak > 0 && (
                    <div
                        className="kpm-peak absolute inset-x-0"
                        style={{ bottom: `${peakPercent}%` }}
                    >
                        {/* KPM number near the peak marker */}
                        <span className="kpm-peak-label">
                            {state.peak}
                        </span>
                    </div>
                )}
            </div>

            {/* Readout + label area */}
            <div className="flex! flex-col items-center gap-0.5 shrink-0">
                <span className="text-sm font-light opacity-75">{state.kpm}</span>
                <span className="text-[10px] tracking-wider font-light opacity-50">KPM</span>
                <KeyboardIcon size={24} className="opacity-50" />
            </div>
        </div>
    );
}
