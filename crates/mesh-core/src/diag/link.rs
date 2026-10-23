//! Per-link diagnostics counters kept by the link layer while diagnostics are on.

use mesh_lab::ridelog::{DirCounters, FrameType};

use crate::Instant;

/// A WouldBlock burst is reported when more than this many answers land in one
/// [`WB_WINDOW_US`] window.
pub const WB_BURST: u32 = 10;
/// WouldBlock burst window (µs).
pub const WB_WINDOW_US: u64 = 1_000_000;

/// Counters of one link (cumulative since the link was added).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkDiag {
    pub tx: DirCounters,
    pub rx: DirCounters,
    /// WouldBlock answers (cumulative).
    pub would_block: u64,
    /// Current WouldBlock window: start and answers in it, and whether it was reported.
    wb_start: Option<Instant>,
    wb_count: u32,
    wb_reported: bool,
    /// When the link became Up.
    pub up_at: Option<Instant>,
}

impl LinkDiag {
    /// Counts one sent frame.
    pub fn tx(&mut self, ty: FrameType, len: usize) {
        self.tx.count(ty, len);
    }

    /// Counts one received frame.
    pub fn rx(&mut self, ty: FrameType, len: usize) {
        self.rx.count(ty, len);
    }

    /// Counts one frame that failed to decode or authenticate.
    pub fn bad(&mut self) {
        self.rx.bad = self.rx.bad.saturating_add(1);
    }

    /// Counts one WouldBlock answer; returns `Some((count, window ms))` once per window
    /// when the burst threshold is crossed.
    pub fn would_block(&mut self, now: Instant) -> Option<(u32, u32)> {
        self.would_block = self.would_block.saturating_add(1);
        match self.wb_start {
            Some(t) if now.since(t) < WB_WINDOW_US => self.wb_count += 1,
            _ => {
                self.wb_start = Some(now);
                self.wb_count = 1;
                self.wb_reported = false;
            }
        }
        if self.wb_count > WB_BURST && !self.wb_reported {
            self.wb_reported = true;
            let ms = self.wb_start.map_or(0, |t| now.since(t) / 1_000);
            return Some((self.wb_count, u32::try_from(ms).unwrap_or(u32::MAX)));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_would_block_burst_is_reported_once_per_window() {
        let mut d = LinkDiag::default();
        let mut hits = Vec::new();
        for i in 0..30u64 {
            if let Some(h) = d.would_block(Instant(i * 10_000)) {
                hits.push(h);
            }
        }
        assert_eq!(hits, vec![(11, 100)]);
        assert_eq!(d.would_block, 30);
        assert!(d.would_block(Instant(2_000_000)).is_none(), "new window");
    }
}
