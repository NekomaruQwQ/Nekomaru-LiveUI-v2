//! JSON line protocol for `live-kpm`.
//!
//! Defines the message format used to stream keystroke counts from the
//! capture process to the server over stdout.
//!
//! ## Wire Format
//!
//! Unlike `live-capture` and `live-audio` (binary envelope), `live-kpm` uses
//! JSON lines for easier debugging — each stdout line is a single JSON object:
//!
//! ```json
//! {"t":1710590400123456,"c":5}
//! ```
//!
//! - `t`: wall-clock timestamp in microseconds since Unix epoch
//! - `c`: number of keystrokes in the batch interval

use serde::{Deserialize, Serialize};
use std::io;
use std::io::Write;

// ── Protocol Types ───────────────────────────────────────────────────────────

/// A single batch of keystroke counts covering one batch interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    /// Wall-clock timestamp in microseconds since Unix epoch.
    /// Uses `SystemTime::now().duration_since(UNIX_EPOCH)` — same clock as
    /// `live-capture` and `live-audio` timestamps.
    pub t: u64,
    /// Number of keystrokes counted during this batch interval.
    pub c: u32,
}

// ── Serialization (write to stdout) ──────────────────────────────────────────

/// Write a `Batch` as a JSON line (newline-terminated).
pub fn write_batch(w: &mut impl Write, batch: &Batch) -> io::Result<()> {
    serde_json::to_writer(&mut *w, batch)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    w.write_all(b"\n")?;
    w.flush()
}

// ── Deserialization (read from stdout pipe) ──────────────────────────────────

/// Parse a single JSON line into a `Batch`.
///
/// The line should NOT include the trailing newline.
pub fn parse_batch(line: &str) -> Result<Batch, serde_json::Error> {
    serde_json::from_str(line)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_batch() {
        let batch = Batch { t: 1_710_590_400_123_456, c: 5 };

        let mut buf = Vec::new();
        write_batch(&mut buf, &batch).unwrap();

        let line = std::str::from_utf8(&buf).unwrap().trim_end();
        let decoded = parse_batch(line).unwrap();

        assert_eq!(decoded, batch);
    }

    #[test]
    fn round_trip_zero_count() {
        let batch = Batch { t: 0, c: 0 };

        let mut buf = Vec::new();
        write_batch(&mut buf, &batch).unwrap();

        let line = std::str::from_utf8(&buf).unwrap().trim_end();
        let decoded = parse_batch(line).unwrap();

        assert_eq!(decoded, batch);
    }

    #[test]
    fn round_trip_high_count() {
        let batch = Batch { t: u64::MAX, c: u32::MAX };

        let mut buf = Vec::new();
        write_batch(&mut buf, &batch).unwrap();

        let line = std::str::from_utf8(&buf).unwrap().trim_end();
        let decoded = parse_batch(line).unwrap();

        assert_eq!(decoded, batch);
    }

    #[test]
    fn sequential_batches() {
        let batches = vec![
            Batch { t: 1000, c: 3 },
            Batch { t: 2000, c: 0 },
            Batch { t: 3000, c: 10 },
        ];

        let mut buf = Vec::new();
        for b in &batches {
            write_batch(&mut buf, b).unwrap();
        }

        let text = std::str::from_utf8(&buf).unwrap();
        let decoded: Vec<Batch> = text
            .lines()
            .map(|line| parse_batch(line).unwrap())
            .collect();

        assert_eq!(decoded, batches);
    }

    #[test]
    fn json_format_is_compact() {
        let batch = Batch { t: 100, c: 7 };

        let mut buf = Vec::new();
        write_batch(&mut buf, &batch).unwrap();

        let line = std::str::from_utf8(&buf).unwrap().trim_end();
        // Verify compact JSON (no spaces) with exactly these two fields
        assert_eq!(line, r#"{"t":100,"c":7}"#);
    }
}
