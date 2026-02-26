// Simple stream-availability hook.
//
// Polls GET /streams every 2 seconds and exposes boolean flags for whether
// each well-known stream exists.  Used by app.tsx to show/hide the YouTube
// Music island — the main stream always renders regardless (StreamRenderer
// handles 404 internally via retry).

import { useEffect, useState } from "react";

import { api } from "./api";

export interface StreamStatus {
    hasMain: boolean;
    hasYouTubeMusic: boolean;
}

const POLL_INTERVAL_MS = 2000;

export function useStreamStatus(): StreamStatus {
    const [status, setStatus] = useState<StreamStatus>({
        hasMain: false,
        hasYouTubeMusic: false,
    });

    useEffect(() => {
        let cancelled = false;

        async function poll() {
            if (cancelled) return;
            try {
                const res = await api.index.$get();
                if (!res.ok || cancelled) return;
                const streams = await res.json();
                setStatus({
                    hasMain: streams.some((s) => s.id === "main"),
                    hasYouTubeMusic: streams.some((s) => s.id === "youtube-music"),
                });
            } catch (e) {
                console.error("Failed to poll stream status:", e);
            }
        }

        poll();
        const intervalId = setInterval(poll, POLL_INTERVAL_MS);

        return () => {
            cancelled = true;
            clearInterval(intervalId);
        };
    }, []);

    return status;
}
