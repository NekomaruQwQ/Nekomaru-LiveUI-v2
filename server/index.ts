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
import { createLogger } from "./log";
import { destroyAll } from "./process";
import { selector } from "./selector";
import { ytmManager } from "./youtube-music";
import api from "./api";
import stringsApi, { reloadStore } from "./strings";

const log = createLogger("server::server");

// ── Hono app ─────────────────────────────────────────────────────────────────

const honoApp =
    new Hono()
        .route("/api/v1/streams", api)
        .route("/api/v1/strings", stringsApi)

        /// Reload selector config and string store from disk.
        .post("/api/v1/refresh", async (c) => {
            await selector.loadPersistedConfig();
            await reloadStore();
            return c.json({ ok: true });
        });

const honoServer =
    hono.getRequestListener(honoApp.fetch);

// ── Vite dev server ──────────────────────────────────────────────────────────
// The frontend lives in a sibling directory (../frontend/).  We point Vite at
// its config file so it resolves root, aliases, and plugins correctly.

const viteServer =
    await vite.createServer({
        configFile: path.resolve(import.meta.dirname, "../frontend/vite.config.ts"),
        server: {
            middlewareMode: true,
            hmr: {
                port: serverPort + 10000,
            },
        },
    });

// ── HTTP server ──────────────────────────────────────────────────────────────

const httpServer =
    http.createServer(async (req, res) => {
        if (req.url?.startsWith("/api/")) {
            await honoServer(req, res);
        } else {
            viteServer.middlewares(req, res);
        }
    });

httpServer.listen(serverPort, () => {
    log.info(`LiveServer running at ${baseUrl}`);

    // Auto-start the window selector and YouTube Music manager once the
    // HTTP server is listening, so streams are created before any client
    // connects.
    selector.start();
    ytmManager.start();
});

// ── Cleanup ──────────────────────────────────────────────────────────────────
// Stop both managers and kill all child processes when the server shuts down.

process.on("SIGINT", () => {
    selector.stop();
    ytmManager.stop();
    destroyAll();
    process.exit(0);
});

process.on("SIGTERM", () => {
    selector.stop();
    ytmManager.stop();
    destroyAll();
    process.exit(0);
});
