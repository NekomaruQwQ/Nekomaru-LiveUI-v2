// Typed API client for the LiveServer stream API.
//
// Uses Hono RPC (hc) to get end-to-end type safety from the server route
// definitions.  The server exports its route type as ApiType; we pass it to
// hc() so every endpoint call is fully typed — URL construction, path/query
// params, request body, and response shape.

import { hc } from "hono/client";
import type { ApiType } from "../../server/api";

/// Typed Hono RPC client.  All endpoints under /api/v1/streams are accessible
/// via this client (e.g. `api.index.$get()`, `api[":id"].init.$get(...)`).
export const api = hc<ApiType>("/api/v1/streams");
