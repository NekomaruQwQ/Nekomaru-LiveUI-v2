//! Sliding window KPM calculator.
//!
//! Receives timestamped keystroke batches and computes keystrokes-per-minute
//! over a configurable sliding window.  The frontend handles peak hold + decay
//! animations on its own side.

use std::collections::VecDeque;

struct Entry {
    timestamp_us: u64,
    count: u32,
}

/// Computes keystrokes-per-minute from a sliding window of recent batches.
pub struct KpmCalculator {
    entries: VecDeque<Entry>,
    /// Sliding window duration in microseconds.
    window_us: u64,
    /// Maximum entries to keep.
    capacity: usize,
}

impl KpmCalculator {
    /// Create a new calculator.
    ///
    /// - `window_ms`: sliding window duration (e.g. 5000ms).
    /// - `batch_ms`: batch interval (e.g. 50ms) — used to size the buffer.
    pub const fn new(window_ms: u64, batch_ms: u64) -> Self {
        Self {
            entries: VecDeque::new(),
            window_us: window_ms * 1000,
            capacity: (window_ms / batch_ms + 10) as usize,
        }
    }

    /// Record a new batch from the capture process.
    pub fn push_batch(&mut self, timestamp_us: u64, count: u32) {
        self.entries.push_back(Entry { timestamp_us, count });

        // Evict entries older than the window.
        let cutoff = timestamp_us.saturating_sub(self.window_us);
        while self.entries.front().is_some_and(|e| e.timestamp_us < cutoff) {
            self.entries.pop_front();
        }

        // Hard cap.
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    /// Compute current KPM from the sliding window.
    ///
    /// Returns 0 if there's insufficient data (fewer than 2 entries or
    /// time span < 200ms).
    pub fn get_kpm(&self) -> f64 {
        if self.entries.len() < 2 { return 0.0; }

        let oldest = &self.entries[0];
        let newest = &self.entries[self.entries.len() - 1];
        let span_us = newest.timestamp_us - oldest.timestamp_us;

        // Need at least 200ms of data for a meaningful extrapolation.
        if span_us < 200_000 { return 0.0; }

        let total_count: u64 = self.entries.iter().map(|e| u64::from(e.count)).sum();
        let span_seconds = span_us as f64 / 1_000_000.0;
        (total_count as f64 / span_seconds) * 60.0
    }

    /// Clear all state (e.g. on process restart).
    pub fn reset(&mut self) {
        self.entries.clear();
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_zero() {
        let calc = KpmCalculator::new(5000, 50);
        assert_eq!(calc.get_kpm(), 0.0);
    }

    #[test]
    fn single_entry_returns_zero() {
        let mut calc = KpmCalculator::new(5000, 50);
        calc.push_batch(1_000_000, 5);
        assert_eq!(calc.get_kpm(), 0.0);
    }

    #[test]
    fn two_entries_one_second_apart() {
        let mut calc = KpmCalculator::new(5000, 50);
        calc.push_batch(1_000_000, 10);
        calc.push_batch(2_000_000, 10);

        // 20 keystrokes in 1 second = 1200 KPM.
        let kpm = calc.get_kpm();
        assert!((kpm - 1200.0).abs() < 1.0);
    }

    #[test]
    fn window_eviction() {
        let mut calc = KpmCalculator::new(1000, 50); // 1s window

        // Push batches spanning 2 seconds.
        for i in 0..40 {
            calc.push_batch(i * 50_000, 1); // every 50ms
        }

        // Only the last 1s of entries should remain (~20 entries).
        assert!(calc.entries.len() <= 30);
    }

    #[test]
    fn reset_clears_state() {
        let mut calc = KpmCalculator::new(5000, 50);
        calc.push_batch(1_000_000, 5);
        calc.push_batch(2_000_000, 5);

        calc.reset();
        assert_eq!(calc.get_kpm(), 0.0);
        assert!(calc.entries.is_empty());
    }
}
