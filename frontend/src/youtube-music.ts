// Hook that auto-discovers a YouTube Music window and streams its playback
// bar (bottom 128px) via crop-mode capture.
//
// Completely independent from useCaptureControl() — the two hooks manage
// separate streams with separate lifecycles.

import { useEffect, useState } from "react";

import { api } from "./api";
import type { WindowInfo } from "./capture";

/// Title prefix used to identify the YouTube Music window.
const YTM_TITLE_PREFIX = "YouTube Music";

/// How often to re-check for the YouTube Music window (ms).
/// Slow poll — this isn't latency-sensitive.
const POLL_INTERVAL_MS = 5000;

// ── Hook ─────────────────────────────────────────────────────────────────

/// Manages a crop-mode stream of the YouTube Music playback bar.
///
/// On mount, enumerates windows and looks for one whose title starts with
/// "YouTube Music".  If found, creates a crop stream (full width × 128px,
/// bottom-aligned).  Polls every 5s to detect the window appearing or
/// disappearing, creating/destroying the stream accordingly.
///
/// Returns `streamId` (non-null when streaming) for use with `<StreamRenderer>`.
export function useYouTubeMusicStream() {
    const [streamId, setStreamId] = useState<string | null>(null);

    useEffect(() => {
        let cancelled = false;
        let activeStreamId: string | null = null;

        async function poll() {
            if (cancelled) return;

            try {
                const res = await api.windows.$get();
                if (!res.ok || cancelled) return;
                const windows = (await res.json()) as unknown as WindowInfo[];

                const ytm = windows.find((w) =>
                    w.title.startsWith(YTM_TITLE_PREFIX));

                if (ytm && !activeStreamId) {
                    // YouTube Music appeared — create a crop stream for the
                    // playback bar (bottom 128px at full window width).
                    const createRes = await api.index.$post({
                        json: {
                            hwnd: String(ytm.hwnd),
                            cropWidth: "full" as const,
                            cropHeight: 128,
                            cropAlign: "bottom" as const,
                        },
                    });
                    if (createRes.ok && !cancelled) {
                        const { id } = await createRes.json();
                        activeStreamId = id;
                        setStreamId(id);
                    }
                } else if (!ytm && activeStreamId) {
                    // YouTube Music disappeared — tear down the stream.
                    await api[":id"].$delete({ param: { id: activeStreamId } });
                    activeStreamId = null;
                    setStreamId(null);
                }
            } catch (e) {
                console.error("YouTube Music poll failed:", e);
            }
        }

        poll();
        const intervalId = setInterval(poll, POLL_INTERVAL_MS);

        return () => {
            cancelled = true;
            clearInterval(intervalId);
            if (activeStreamId) {
                api[":id"].$delete({ param: { id: activeStreamId } }).catch(() => {});
            }
        };
    }, []);

    return { streamId };
}
