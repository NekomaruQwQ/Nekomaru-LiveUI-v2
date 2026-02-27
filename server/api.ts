// HTTP API routes for stream management.
//
// Mounted at /streams in index.ts.  All routes are relative to that base:
//   GET  /              → list streams
//   POST /              → create a new capture
//   DELETE /:id         → destroy a capture
//   GET  /:id/init      → codec params (SPS/PPS/resolution)
//   GET  /:id/frames    → encoded frames (polling)
//   GET  /windows       → enumerate capturable windows
//   GET  /auto          → auto-selector status
//   POST /auto          → start auto-selector
//   DELETE /auto        → stop auto-selector
//   GET  /auto/config   → auto-selector include/exclude lists
//   PUT  /auto/config   → replace include/exclude lists
//
// Routes are method-chained so TypeScript infers the full route schema into
// `typeof api`.  The frontend imports ApiType to create a typed Hono RPC client.
//
// Uses relative imports (not @/) so the frontend's tsconfig can resolve this
// file when importing ApiType — its @/* alias points elsewhere.

import { zValidator } from "@hono/zod-validator";
import { Hono } from "hono";
import { z } from "zod";

import * as proc from "./process";
import { selector } from "./selector";

const api = new Hono()

    // ── Stream management ────────────────────────────────────────────────

    /// List all active capture streams.
    .get("/", (c) => {
        const streams = proc.listStreams();
        return c.json(streams.map((s) => ({
            id: s.id,
            hwnd: s.hwnd,
            status: s.status,
            generation: s.generation,
        })));
    })

    /// Create a new capture stream (spawns a live-capture.exe instance).
    /// Accepts either resample mode (`width` + `height`) or crop mode
    /// (`cropMinX/Y` + `cropMaxX/Y` — absolute bounding box).
    .post("/",
        zValidator("json", z.union([
            z.object({
                hwnd: z.string(),
                width: z.number().int().positive(),
                height: z.number().int().positive(),
            }),
            z.object({
                hwnd: z.string(),
                cropMinX: z.number().int().nonnegative(),
                cropMinY: z.number().int().nonnegative(),
                cropMaxX: z.number().int().positive(),
                cropMaxY: z.number().int().positive(),
            }),
        ])),
        (c) => {
            const body = c.req.valid("json");
            if ("cropMinX" in body) {
                const stream = proc.createCropStream(
                    body.hwnd, body.cropMinX, body.cropMinY, body.cropMaxX, body.cropMaxY);
                return c.json({ id: stream.id }, 201);
            }
            const stream = proc.createStream(body.hwnd, body.width, body.height);
            return c.json({ id: stream.id }, 201);
        })

    // ── Window enumeration ───────────────────────────────────────────────

    /// List capturable windows.  One-shot spawn of live-capture.exe --enumerate-windows.
    /// Placed before /:id routes so the static path takes priority.
    .get("/windows", async (c) => {
        const windows = await proc.enumerateWindows();
        return c.json(windows);
    })

    // ── Auto-selector ─────────────────────────────────────────────────────

    /// Get auto-selector status.
    .get("/auto", (c) => {
        return c.json(selector.status());
    })

    /// Start the automatic window selector (polls foreground every 2s).
    .post("/auto", (c) => {
        selector.start();
        return c.json(selector.status(), 201);
    })

    /// Stop the automatic window selector (kills the managed stream).
    .delete("/auto", (c) => {
        selector.stop();
        return c.json({ ok: true });
    })

    /// Get the auto-selector's include/exclude pattern lists.
    .get("/auto/config", (c) => {
        return c.json(selector.getConfig());
    })

    /// Replace the auto-selector's include/exclude pattern lists.
    .put("/auto/config",
        zValidator("json", z.object({
            includeList: z.array(z.string()),
            excludeList: z.array(z.string()),
        })),
        async (c) => {
            await selector.setConfig(c.req.valid("json"));
            return c.json({ ok: true });
        })

    // ── Stream lifecycle ─────────────────────────────────────────────────

    /// Destroy a capture stream (kills the child process).
    .delete("/:id", (c) => {
        const id = c.req.param("id");
        const stream = proc.getStream(id);
        if (!stream) return c.json({ error: "stream not found" }, 404);
        proc.destroyStream(id);
        return c.json({ ok: true });
    })

    // ── Stream data ──────────────────────────────────────────────────────

    /// Return codec initialization parameters for the decoder.
    /// Returns 503 if the encoder hasn't produced its first IDR frame yet —
    /// the frontend has retry logic and will poll again.
    .get("/:id/init", (c) => {
        const stream = proc.getStream(c.req.param("id"));
        if (!stream) return c.json({ error: "stream not found" }, 404);

        const params = stream.buffer.getCodecParams();
        if (!params) return c.json({ error: "codec params not yet available" }, 503);

        return c.json({
            sps: uint8ToBase64(params.sps),
            pps: uint8ToBase64(params.pps),
            width: params.width,
            height: params.height,
        });
    })

    /// Return encoded frames after a given sequence number.
    /// The frontend polls this endpoint at ~60fps with ?after=lastSequence.
    .get("/:id/frames",
        zValidator("query", z.object({ after: z.string().optional() })),
        (c) => {
            const stream = proc.getStream(c.req.param("id"));
            if (!stream) return c.json({ error: "stream not found" }, 404);

            const after = parseInt(c.req.valid("query").after ?? "0", 10) || 0;
            const frames = stream.buffer.getFramesAfter(after);

            return c.json({
                generation: stream.generation,
                frames: frames.map((f) => ({
                    sequence: f.sequence,
                    data: uint8ToBase64(f.payload),
                })),
            });
        });

/// Route type for Hono RPC — the frontend imports this to create a typed
/// client via `hc<ApiType>("/streams")`.
export type ApiType = typeof api;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Encode a Uint8Array to standard base64.
/// The frontend decodes with Uint8Array.fromBase64() (TC39 Stage 3, Chrome 117+).
function uint8ToBase64(data: Uint8Array): string {
    return Buffer.from(data).toString("base64");
}

export default api;
