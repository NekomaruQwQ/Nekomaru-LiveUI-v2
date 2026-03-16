// Shared constants for the LiveServer.

import * as path from "node:path";

/// Directory for persisted runtime data (strings, selector config).
/// Created automatically by the persist module on first import.
export const dataDir = path.resolve(import.meta.dirname, "../data");

/// HTTP server port. Reads `LIVE_PORT` from the environment, falling back to 3000.
export const serverPort = Number(process.env.LIVE_PORT) || 3000;

/// Base URL for logging.
export const baseUrl = `http://localhost:${serverPort}`;

/// Path to the live-capture.exe binary.
/// Resolved relative to this file (server/), so `..` reaches the workspace root.
export const captureExePath =
    path.resolve(import.meta.dirname, "../target/debug/live-capture.app.exe");

/// Path to the live-audio.exe binary.
export const audioExePath =
    path.resolve(import.meta.dirname, "../target/debug/live-audio.app.exe");

/// WASAPI capture device name passed to live-audio.exe via --device.
export const audioDeviceName = "Loopback L + R (Focusrite USB Audio)";

/// Whether audio capture is enabled.  Opt-in via LIVE_AUDIO=1 to avoid
/// feedback loops during localhost development.
export const audioEnabled = !!process.env.LIVE_AUDIO;

/// Path to the live-kpm.exe binary.
export const kpmExePath =
    path.resolve(import.meta.dirname, "../target/debug/live-kpm.app.exe");

/// Number of frames to buffer per stream (~1 second at 60fps).
export const frameBufferCapacity = 60;
