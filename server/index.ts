// Entry point of the TurboDoc server.
//
// The TurboDoc server has three parts:
// 1. The API server powered by Hono (workspace/cache CRUD);
// 2. The HTTP proxy for documentation pages (with SQLite caching);
// 3. The frontend asset server powered by Vite;
//
// To make the three parts work together in Bun, we create a node:http server
// via the Bun NodeJS Compat Layer and route requests to either Hono or Vite
// based on the URL path.
//
// Running the server with `bun --hot` enables hot reload for the server.
// The frontend assets are always hot-reloaded by Vite, regardless of the
// bun flags.

import { Hono } from "hono";

import * as http from "node:http";
import * as vite from "vite";
import * as hono from "@hono/node-server";

import { serverPort, baseUrl } from "@/server/common";
import api from "@/server/api";
import proxy from "@/server/proxy";

const honoApp =
    new Hono()
        .route("/proxy", proxy)
        .route("/api/v1", api);
export type HonoApp = typeof honoApp;

const honoServer =
    hono.getRequestListener(honoApp.fetch);
const viteServer =
    await vite.createServer({ server: { middlewareMode: true } });
const httpServer =
    http.createServer(async (req, res) => {
        if (req.url?.startsWith("/api") ||
            req.url?.startsWith("/proxy")) {
            await honoServer(req, res);
        } else {
            viteServer.middlewares(req, res);
        }
    });

httpServer.listen(serverPort, () => {
    console.log(`Server running at ${baseUrl}`);
});
