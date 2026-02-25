// Typed API client for the LiveServer stream API.
//
// Uses Hono RPC (hc) to get end-to-end type safety from the server route
// definitions.  The server exports its route type as ApiType; we pass it to
// hc() so every endpoint call is fully typed — URL construction, path/query
// params, request body, and response shape.

import { hc } from "hono/client";
import type { ApiType } from "../../server/api";

/// Typed Hono RPC client.  All endpoints under /streams are accessible via
/// this client (e.g. `api.index.$get()`, `api[":id"].init.$get(...)`).
export const api = hc<ApiType>("/streams");

// ── Init with retry ─────────────────────────────────────────────────────

/// Codec init params returned by the server.
export interface InitParams {
    sps: string;
    pps: string;
    width: number;
    height: number;
}

/// Fetch codec initialization params, retrying on 503 (stream starting up).
///
/// The server returns 503 while the capture process is initializing (the
/// encoder hasn't produced its first IDR frame yet).  This wrapper retries
/// with exponential backoff up to ~5 seconds before giving up.
export async function fetchInit(streamId: string): Promise<InitParams> {
    const maxRetries = 20;
    const baseDelayMs = 250;

    for (let attempt = 0; attempt < maxRetries; attempt++) {
        const res = await api[":id"].init.$get({ param: { id: streamId } });

        if (res.ok) {
            // Narrow from the union of all possible response types to the
            // success case — safe because we've confirmed res.ok (status 200).
            return await res.json() as InitParams;
        }

        if (res.status === 503) {
            const delay = baseDelayMs * Math.min(2 ** attempt, 8);
            await new Promise((r) => setTimeout(r, delay));
            continue;
        }

        throw new Error(`Failed to fetch codec params: ${res.status} ${res.statusText}`);
    }

    throw new Error("Timed out waiting for codec params");
}
