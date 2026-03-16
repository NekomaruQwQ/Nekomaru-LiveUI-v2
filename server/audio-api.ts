// HTTP API routes for audio streaming.
//
// Mounted at /api/v1/audio in index.ts.  All routes are relative to that base:
//   GET /init          → audio format params (sample rate, channels, bit depth)
//   GET /chunks?after=N → binary audio chunks since sequence N

import { Hono } from "hono";
import { z } from "zod";
import { zValidator } from "@hono/zod-validator";

import { audioManager } from "./audio";

const audioApi = new Hono()

    /// Return audio format parameters for the frontend's AudioContext setup.
    /// Returns 503 if the capture process hasn't sent params yet.
    .get("/init", (c) => {
        const params = audioManager.buffer.getAudioParams();
        if (!params) return c.json({ error: "audio params not yet available" }, 503);

        return c.json({
            sampleRate: params.sampleRate,
            channels: params.channels,
            bitsPerSample: params.bitsPerSample,
        });
    })

    /// Return audio chunks after a given sequence number as a binary blob.
    /// The frontend polls this endpoint at ~16ms intervals with ?after=lastSeq.
    ///
    /// Binary layout (all little-endian):
    ///   [u32: num_chunks]
    ///   per chunk: [u32: sequence][u32: payload_length][payload bytes]
    ///
    /// Payload per chunk: [u64 LE: timestamp_us][raw PCM s16le bytes]
    .get("/chunks",
        zValidator("query", z.object({ after: z.string().optional() })),
        (c) => {
            const after = parseInt(c.req.valid("query").after ?? "0", 10) || 0;
            const chunks = audioManager.buffer.getChunksAfter(after);

            // Pre-compute total size: 4-byte header + (8 + payload) per chunk.
            let totalSize = 4;
            for (const ch of chunks) totalSize += 8 + ch.payload.length;

            const buf = new Uint8Array(totalSize);
            const view = new DataView(buf.buffer);
            let pos = 0;

            // Header: chunk count.
            view.setUint32(pos, chunks.length, true); pos += 4;

            // Each chunk: sequence + payload length + raw payload bytes.
            for (const ch of chunks) {
                view.setUint32(pos, ch.sequence, true);       pos += 4;
                view.setUint32(pos, ch.payload.length, true); pos += 4;
                buf.set(ch.payload, pos);                     pos += ch.payload.length;
            }

            return c.body(buf, 200, { "Content-Type": "application/octet-stream" });
        });

export type AudioApiType = typeof audioApi;
export default audioApi;
