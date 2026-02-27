// Server-managed string store with well-known IDs.
//
// Provides a simple key-value store that the control panel can write to and
// the frontend can poll.  Follows the same well-known ID pattern as streams
// ("main", "youtube-music") — each string key maps to a specific display
// location in the frontend.
//
// Persisted to data/strings.json — loaded on server start, written on every
// mutation.  Falls back to an empty store if the file is missing or corrupt.
//
// Mounted at /strings in index.ts.  All routes are relative to that base:
//   GET    /       → all key-value pairs as a JSON object
//   PUT    /:key   → set a string value
//   DELETE /:key   → delete a string
//
// Routes are method-chained so TypeScript infers the full route schema into
// `typeof api`.  The frontend imports StringsApiType to create a typed Hono
// RPC client.

import * as path from "node:path";
import { zValidator } from "@hono/zod-validator";
import { Hono } from "hono";
import { z } from "zod";
import { dataDir } from "./common";
import { loadJson, saveJson } from "./persist";

// ── Store ────────────────────────────────────────────────────────────────────

const stringsPath = path.join(dataDir, "strings.json");

/// Backing store: string key → string value.
/// Hydrated from disk on module load; falls back to empty if no file exists.
const initial = await loadJson<Record<string, string>>(stringsPath, {});
const store = new Map<string, string>(Object.entries(initial));

/// Serialize the entire store to disk.
function saveStore(): Promise<void> {
    return saveJson(stringsPath, Object.fromEntries(store));
}

// ── Routes ───────────────────────────────────────────────────────────────────

const api = new Hono()

    /// Return all key-value pairs as a flat JSON object.
    .get("/", (c) => {
        return c.json(Object.fromEntries(store));
    })

    /// Set a string value by key (idempotent).  Persists to disk.
    .put("/:key",
        zValidator("json", z.object({ value: z.string() })),
        async (c) => {
            const key = c.req.param("key");
            const { value } = c.req.valid("json");
            store.set(key, value);
            await saveStore();
            return c.json({ ok: true });
        })

    /// Delete a string by key.  Persists to disk.
    .delete("/:key", async (c) => {
        const key = c.req.param("key");
        store.delete(key);
        await saveStore();
        return c.json({ ok: true });
    });

/// Route type for Hono RPC — the frontend imports this to create a typed
/// client via `hc<StringsApiType>("/strings")`.
export type StringsApiType = typeof api;

export default api;
