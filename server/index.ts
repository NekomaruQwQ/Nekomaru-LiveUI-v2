// Entry point of the LiveServer.
//
// The LiveServer has two parts:
// 1. The stream API powered by Hono (spawn/manage live-capture.exe, serve frames);
// 2. The frontend asset server powered by Vite (hot-reload in development).
//
// To make them coexist on a single port, we create a node:http server via the
// Bun NodeJS Compat Layer and route requests to either Hono or Vite based on
// the URL path.
//
// Running the server with `bun --hot` enables hot reload for the server code.
// The frontend assets are always hot-reloaded by Vite, regardless of bun flags.

import { Hono } from "hono";

import * as path from "node:path";
import * as http from "node:http";
import * as vite from "vite";
import * as hono from "@hono/node-server";

import { serverPort, baseUrl } from "./common";
import { destroyAll } from "./process";
import api from "./api";

// ── Hono app ─────────────────────────────────────────────────────────────────

const honoApp =
    new Hono()
        .route("/streams", api);

const honoServer =
    hono.getRequestListener(honoApp.fetch);

// ── Vite dev server ──────────────────────────────────────────────────────────
// The frontend lives in a sibling directory (../frontend/).  We point Vite at
// its config file so it resolves root, aliases, and plugins correctly.

const viteServer =
    await vite.createServer({
        configFile: path.resolve(import.meta.dirname, "../frontend/vite.config.ts"),
        server: { middlewareMode: true },
    });

// ── HTTP server ──────────────────────────────────────────────────────────────

const httpServer =
    http.createServer(async (req, res) => {
        if (req.url?.startsWith("/streams")) {
            await honoServer(req, res);
        } else {
            viteServer.middlewares(req, res);
        }
    });

httpServer.listen(serverPort, () => {
    console.log(`LiveServer running at ${baseUrl}`);
});

// ── Cleanup ──────────────────────────────────────────────────────────────────
// Kill all child processes when the server is shut down.

process.on("SIGINT", () => {
    destroyAll();
    process.exit(0);
});

process.on("SIGTERM", () => {
    destroyAll();
    process.exit(0);
});
