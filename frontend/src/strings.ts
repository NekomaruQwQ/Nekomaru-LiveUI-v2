// Polling hook for the server-managed string store.
//
// Polls GET /api/v1/strings every 2 seconds and returns all key-value pairs.
// Used by app.tsx to display well-known string IDs at designated locations
// in the layout (e.g. "marquee" in the scrolling top banner).

import { useEffect, useState } from "react";

import { fetchStrings } from "./strings-api";

const POLL_INTERVAL_MS = 2000;

/// Returns all server-managed strings as a key-value record.
/// Polls every 2s — updates are reflected within one interval.
export function useStrings(): Record<string, string> {
    const [strings, setStrings] = useState<Record<string, string>>({});

    useEffect(() => {
        let cancelled = false;

        async function poll() {
            if (cancelled) return;
            try {
                const data = await fetchStrings();
                if (!cancelled) setStrings(data);
            } catch (e) {
                console.error("Failed to poll strings:", e);
            }
        }

        poll();
        const intervalId = setInterval(poll, POLL_INTERVAL_MS);

        return () => {
            cancelled = true;
            clearInterval(intervalId);
        };
    }, []);

    return strings;
}
