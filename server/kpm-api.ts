// HTTP API route for KPM (keystrokes-per-minute).
//
// Mounted at /api/v1/kpm in index.ts.
//   GET /  → { kpm: number }

import { Hono } from "hono";

import { kpmManager } from "./kpm";

const kpmApi = new Hono()

    /// Return the current KPM value computed from the sliding window.
    /// Returns 404 if the KPM capture process is not running.
    .get("/", (c) => {
        if (!kpmManager.active) return c.json({ error: "kpm not available" }, 404);

        const kpm = kpmManager.calculator.getKpm();
        return c.json({ kpm: Math.round(kpm) });
    });

export type KpmApiType = typeof kpmApi;
export default kpmApi;
