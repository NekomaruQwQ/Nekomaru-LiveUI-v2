// KPM (keystrokes-per-minute) meter component.
//
// Polls GET /api/v1/kpm at ~150ms for the current KPM value, computes
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

/// How often to poll the server (ms).
const POLL_INTERVAL_MS = 150;

/// Peak hold duration before decay begins (ms).
const PEAK_HOLD_MS = 1500;

/// Duration of the linear decay from peak to current (ms).
const PEAK_DECAY_MS = 500;

// ── Hook ─────────────────────────────────────────────────────────────────────

interface KpmState {
    kpm: number;
    peak: number;
}

/// Polls the KPM endpoint and computes peak hold + decay locally.
function useKpm(): KpmState | null {
    const [state, setState] = useState<KpmState | null>(null);

    // Peak tracking refs (not state — avoids re-renders on every tick).
    const peakRef = useRef(0);
    const peakTimeRef = useRef(0);

    useEffect(() => {
        let timer: ReturnType<typeof setTimeout>;
        let cancelled = false;

        async function poll() {
            try {
                const res = await fetch("/api/v1/kpm");
                if (res.status === 404) { setState(null); return; }
                if (!res.ok) return;

                const data = await res.json() as { kpm: number };
                const now = performance.now();
                const kpm = data.kpm;

                // Update peak hold.
                if (kpm >= peakRef.current) {
                    // New peak — reset hold timer.
                    peakRef.current = kpm;
                    peakTimeRef.current = now;
                } else {
                    // Decay: hold for PEAK_HOLD_MS, then linear decay over PEAK_DECAY_MS.
                    const elapsed = now - peakTimeRef.current;
                    if (elapsed > PEAK_HOLD_MS) {
                        const decayProgress = Math.min(
                            (elapsed - PEAK_HOLD_MS) / PEAK_DECAY_MS, 1);
                        peakRef.current = peakRef.current + (kpm - peakRef.current) * decayProgress;
                    }
                }

                setState({ kpm, peak: Math.round(peakRef.current) });
            } catch {
                // Network error — keep last state.
            }

            if (!cancelled) timer = setTimeout(poll, POLL_INTERVAL_MS);
        }

        poll();
        return () => { cancelled = true; clearTimeout(timer); };
    }, []);

    return state;
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
