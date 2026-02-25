// Shared constants for the LiveServer.

import * as path from "node:path";

/// HTTP server port. Reads `LIVE_PORT` from the environment, falling back to 3000.
export const serverPort = Number(process.env.LIVE_PORT) || 3000;

/// Base URL for logging.
export const baseUrl = `http://localhost:${serverPort}`;

/// Path to the live-capture.exe binary.
/// Resolved relative to this file (server/), so `..` reaches the workspace root.
export const captureExePath =
    path.resolve(import.meta.dirname, "../target/release/live-capture.exe");

/// Number of frames to buffer per stream (~1 second at 60fps).
export const frameBufferCapacity = 60;
