//! Binary IPC protocol for `live-kpm`.
//!
//! Fixed-size 12-byte messages — no length prefix needed.
//!
//! ## Wire Format
//!
//! ```text
//! [u64 LE: timestamp_us]  — wall-clock timestamp (microseconds since Unix epoch)
//! [u32 LE: count]         — number of keystrokes in this batch interval
//! ```
//!
//! Both ends are Rust, so we use raw little-endian bytes instead of JSON.

use std::io;
use std::io::{Read, Write};

/// Size of one batch message in bytes.
pub const BATCH_SIZE: usize = 12;

// ── Protocol Types ───────────────────────────────────────────────────────────

/// A single batch of keystroke counts covering one batch interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// Wall-clock timestamp in microseconds since Unix epoch.
    /// Uses `SystemTime::now().duration_since(UNIX_EPOCH)` — same clock as
    /// `live-video` and `live-audio` timestamps.
    pub t: u64,
    /// Number of keystrokes counted during this batch interval.
    pub c: u32,
}

// ── Serialization (write to stdout) ──────────────────────────────────────────

/// Write a `Batch` as a 12-byte binary message.
pub fn write_batch(w: &mut impl Write, batch: &Batch) -> io::Result<()> {
    w.write_all(&batch.t.to_le_bytes())?;
    w.write_all(&batch.c.to_le_bytes())?;
    w.flush()
}

// ── Deserialization (read from stdout pipe) ──────────────────────────────────

/// Read one `Batch` from a byte stream.
///
/// Returns `Ok(None)` on clean EOF.
/// Returns `Err` on unexpected EOF mid-message.
pub fn read_batch(r: &mut impl Read) -> io::Result<Option<Batch>> {
    let mut buf = [0u8; BATCH_SIZE];
    match r.read_exact(&mut buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let t = u64::from_le_bytes(buf[0..8].try_into().unwrap());
    let c = u32::from_le_bytes(buf[8..12].try_into().unwrap());

    Ok(Some(Batch { t, c }))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_batch() {
        let batch = Batch { t: 1_710_590_400_123_456, c: 5 };

        let mut buf = Vec::new();
        write_batch(&mut buf, &batch).unwrap();

        assert_eq!(buf.len(), BATCH_SIZE);

        let mut cursor = Cursor::new(&buf);
        let decoded = read_batch(&mut cursor).unwrap().unwrap();
        assert_eq!(decoded, batch);
    }

    #[test]
    fn round_trip_zero_count() {
        let batch = Batch { t: 0, c: 0 };

        let mut buf = Vec::new();
        write_batch(&mut buf, &batch).unwrap();

        let mut cursor = Cursor::new(&buf);
        let decoded = read_batch(&mut cursor).unwrap().unwrap();
        assert_eq!(decoded, batch);
    }

    #[test]
    fn round_trip_max_values() {
        let batch = Batch { t: u64::MAX, c: u32::MAX };

        let mut buf = Vec::new();
        write_batch(&mut buf, &batch).unwrap();

        let mut cursor = Cursor::new(&buf);
        let decoded = read_batch(&mut cursor).unwrap().unwrap();
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
            write_batch(&mut buf, &b).unwrap();
        }

        assert_eq!(buf.len(), BATCH_SIZE * 3);

        let mut cursor = Cursor::new(&buf);
        for expected in &batches {
            let decoded = read_batch(&mut cursor).unwrap().unwrap();
            assert_eq!(&decoded, expected);
        }
        assert!(read_batch(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn eof_returns_none() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        assert!(read_batch(&mut cursor).unwrap().is_none());
    }
}
