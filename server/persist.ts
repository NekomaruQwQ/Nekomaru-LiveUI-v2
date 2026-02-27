// Thin JSON file persistence for server runtime state.
//
// Provides loadJson/saveJson for reading and writing JSON files in the
// data/ directory.  Used by the string store and auto-selector config to
// survive server restarts.
//
// The data directory is created eagerly on module load so callers never
// need to worry about it.

import { existsSync, mkdirSync } from "node:fs";
import { dataDir } from "./common";

// ── Bootstrap ────────────────────────────────────────────────────────────────

/// Ensure the data directory exists before any reads or writes.
if (!existsSync(dataDir)) mkdirSync(dataDir, { recursive: true });

// ── API ──────────────────────────────────────────────────────────────────────

/// Read and parse a JSON file, returning `fallback` on any error
/// (missing file, permission error, malformed JSON).
export async function loadJson<T>(filePath: string, fallback: T): Promise<T> {
	try {
		return await Bun.file(filePath).json() as T;
	} catch {
		return fallback;
	}
}

/// Write `data` as pretty-printed JSON (tab-indented).
export async function saveJson(filePath: string, data: unknown): Promise<void> {
	await Bun.write(filePath, JSON.stringify(data, null, "\t"));
}
