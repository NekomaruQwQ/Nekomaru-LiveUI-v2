// Low-level WebSocket helpers for binary streaming (video, audio).
//
// Provides `openWebSocket` (promise-based connect) and `wsMessages`
// (async generator yielding ArrayBuffer).  Tied to AbortSignal for
// clean teardown (React effect cleanup, component unmount, etc.).

/// Open a WebSocket and wait for the connection to be established.
/// Rejects on error or abort.
export function openWebSocket(path: string, signal: AbortSignal): Promise<WebSocket> {
    return new Promise<WebSocket>((resolve, reject) => {
        if (signal.aborted) { reject(new Error("aborted")); return; }

        const proto = location.protocol === "https:" ? "wss:" : "ws:";
        const ws = new WebSocket(`${proto}//${location.host}${path}`);
        ws.binaryType = "arraybuffer";

        const onAbort = () => { ws.close(); reject(new Error("aborted")); };
        signal.addEventListener("abort", onAbort, { once: true });

        ws.onopen = () => {
            signal.removeEventListener("abort", onAbort);
            resolve(ws);
        };
        ws.onerror = () => {
            signal.removeEventListener("abort", onAbort);
            reject(new Error("WebSocket connection error"));
        };
    });
}

/// Async generator that yields binary ArrayBuffer messages from a WebSocket.
/// Ends when the WebSocket closes or the signal fires.
export async function* wsMessages(
    ws: WebSocket,
    signal: AbortSignal,
): AsyncGenerator<ArrayBuffer> {
    const queue: ArrayBuffer[] = [];
    let resolve: (() => void) | null = null;
    let done = false;

    ws.onmessage = (ev: MessageEvent) => {
        if (ev.data instanceof ArrayBuffer) {
            queue.push(ev.data);
            resolve?.();
        }
    };
    ws.onclose = () => { done = true; resolve?.(); };
    ws.onerror = () => { done = true; resolve?.(); };

    const onAbort = () => { ws.close(); };
    signal.addEventListener("abort", onAbort, { once: true });

    try {
        while (!done && !signal.aborted) {
            if (queue.length > 0) {
                // biome-ignore lint/style/noNonNullAssertion: length check above
                yield queue.shift()!;
            } else {
                await new Promise<void>(r => { resolve = r; });
                resolve = null;
            }
        }
    } finally {
        signal.removeEventListener("abort", onAbort);
    }
}
