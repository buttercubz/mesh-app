//! Dedup windows (DESIGN §5.3), updated **only** for authenticated records.
//!
//! A key is `(origin, sess)`: highest seq `H`, a 1024-bit bitmap behind `H` and the last
//! update time. For `d = (seq − H) mod 2¹⁶`: `1 ≤ d < 32768` ⇒ newer (shift the window);
//! otherwise, within 1024 behind ⇒ check and set the bit; otherwise ⇒ too old. There is
//! no "far behind means restart" heuristic: a restarted origin draws a new `sess`, which
//! starts a fresh window, and PRESENCE from the new `sess` evicts the old window.
//! Windows expire after 5 s of silence; at most 2 windows per origin are kept.

use std::collections::BTreeMap;

use crate::consts::{DEDUP_EXPIRY_US, DEDUP_WINDOW, DEDUP_WINDOWS_PER_ORIGIN};
use crate::types::{Instant, NodeId};

const WORDS: usize = DEDUP_WINDOW as usize / 64;

/// Result of a dedup check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupVerdict {
    /// First copy: deliver/forward.
    New,
    /// Already seen.
    Duplicate,
    /// More than 1024 behind the highest seq (dropped).
    TooOld,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Window {
    high: u16,
    /// Bit `i` (0-based) set ⇔ `high − (i + 1)` was seen.
    bits: [u64; WORDS],
    last_update: Instant,
}

impl Window {
    fn new(seq: u16, now: Instant) -> Self {
        Window {
            high: seq,
            bits: [0; WORDS],
            last_update: now,
        }
    }

    fn get(&self, behind: u16) -> bool {
        let i = usize::from(behind - 1);
        self.bits[i / 64] & (1 << (i % 64)) != 0
    }

    fn set(&mut self, behind: u16) {
        let i = usize::from(behind - 1);
        self.bits[i / 64] |= 1 << (i % 64);
    }

    /// Shifts the window forward by `d ≥ 1` (the old high becomes `d` behind): every
    /// bit index moves up by `d`, and bit `d − 1` records the old high.
    fn advance(&mut self, d: u16) {
        let d = usize::from(d);
        let mut out = [0u64; WORDS];
        if d < usize::from(DEDUP_WINDOW) {
            let (ws, bs) = (d / 64, d % 64);
            for i in (ws..WORDS).rev() {
                let mut v = self.bits[i - ws] << bs;
                if bs > 0 && i > ws {
                    v |= self.bits[i - ws - 1] >> (64 - bs);
                }
                out[i] = v;
            }
        }
        if d <= usize::from(DEDUP_WINDOW) {
            let i = d - 1;
            out[i / 64] |= 1 << (i % 64);
        }
        self.bits = out;
    }

    fn check(&self, seq: u16) -> DedupVerdict {
        let d = seq.wrapping_sub(self.high);
        if d == 0 {
            return DedupVerdict::Duplicate;
        }
        if d < 0x8000 {
            return DedupVerdict::New;
        }
        let behind = self.high.wrapping_sub(seq);
        if behind <= DEDUP_WINDOW {
            if self.get(behind) {
                DedupVerdict::Duplicate
            } else {
                DedupVerdict::New
            }
        } else {
            DedupVerdict::TooOld
        }
    }

    fn mark(&mut self, now: Instant, seq: u16) -> DedupVerdict {
        let v = self.check(seq);
        if v != DedupVerdict::New {
            return v;
        }
        let d = seq.wrapping_sub(self.high);
        if d < 0x8000 {
            self.advance(d);
            self.high = seq;
        } else {
            self.set(self.high.wrapping_sub(seq));
        }
        self.last_update = now;
        DedupVerdict::New
    }
}

/// All dedup windows of a node (≤ 2 per origin).
#[derive(Debug, Clone, Default)]
pub struct Dedup {
    windows: BTreeMap<(NodeId, u8), Window>,
}

impl Dedup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Non-mutating check.
    pub fn seen(&self, now: Instant, origin: NodeId, sess: u8, seq: u16) -> bool {
        match self.windows.get(&(origin, sess)) {
            Some(w) if now.since(w.last_update) < DEDUP_EXPIRY_US => {
                w.check(seq) != DedupVerdict::New
            }
            _ => false,
        }
    }

    /// Checks and marks `(origin, sess, seq)`. Call only for authenticated records.
    pub fn check_and_mark(
        &mut self,
        now: Instant,
        origin: NodeId,
        sess: u8,
        seq: u16,
    ) -> DedupVerdict {
        let key = (origin, sess);
        if let Some(w) = self.windows.get_mut(&key) {
            if now.since(w.last_update) < DEDUP_EXPIRY_US {
                return w.mark(now, seq);
            }
            self.windows.remove(&key);
        }
        // A fresh window: evict expired ones and keep ≤ 2 per origin.
        let mut same_origin: Vec<(u8, Instant)> = self
            .windows
            .range((origin, 0)..=(origin, u8::MAX))
            .map(|(&(_, s), w)| (s, w.last_update))
            .collect();
        same_origin.sort_by_key(|(_, t)| *t);
        while same_origin.len() >= DEDUP_WINDOWS_PER_ORIGIN {
            let (s, _) = same_origin.remove(0);
            self.windows.remove(&(origin, s));
        }
        self.windows.insert(key, Window::new(seq, now));
        DedupVerdict::New
    }

    /// PRESENCE from `(origin, sess)`: evicts every other window of that origin.
    pub fn on_presence(&mut self, origin: NodeId, sess: u8) {
        self.windows.retain(|&(o, s), _| o != origin || s == sess);
    }

    /// Drops windows silent for 5 s.
    pub fn expire(&mut self, now: Instant) {
        self.windows
            .retain(|_, w| now.since(w.last_update) < DEDUP_EXPIRY_US);
    }

    /// Number of live windows (diagnostics, memory bound tests).
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn window_rules() {
        let mut d = Dedup::new();
        let t = Instant::from_secs(1);
        assert_eq!(d.check_and_mark(t, 1, 7, 100), DedupVerdict::New);
        assert_eq!(d.check_and_mark(t, 1, 7, 100), DedupVerdict::Duplicate);
        assert_eq!(d.check_and_mark(t, 1, 7, 99), DedupVerdict::New);
        assert_eq!(d.check_and_mark(t, 1, 7, 99), DedupVerdict::Duplicate);
        assert_eq!(d.check_and_mark(t, 1, 7, 105), DedupVerdict::New);
        assert_eq!(d.check_and_mark(t, 1, 7, 100), DedupVerdict::Duplicate);
        assert_eq!(d.check_and_mark(t, 1, 7, 101), DedupVerdict::New);
        // 1024 behind is still inside, 1025 behind is too old.
        assert_eq!(
            d.check_and_mark(t, 1, 7, 105u16.wrapping_sub(1024)),
            DedupVerdict::New
        );
        assert_eq!(
            d.check_and_mark(t, 1, 7, 105u16.wrapping_sub(1025)),
            DedupVerdict::TooOld
        );
        // Wrap-around: 0xFFFF → 3 is newer.
        assert_eq!(d.check_and_mark(t, 2, 1, 0xFFFF), DedupVerdict::New);
        assert_eq!(d.check_and_mark(t, 2, 1, 3), DedupVerdict::New);
        assert_eq!(d.check_and_mark(t, 2, 1, 0xFFFF), DedupVerdict::Duplicate);
        assert!(d.seen(t, 2, 1, 3));
        assert!(!d.seen(t, 2, 1, 2));
    }

    #[test]
    fn new_sess_starts_fresh_and_presence_evicts() {
        let mut d = Dedup::new();
        let t = Instant::from_secs(1);
        d.check_and_mark(t, 1, 7, 500);
        // A restarted origin with a new sess: seq far behind is still new.
        assert_eq!(d.check_and_mark(t, 1, 9, 10), DedupVerdict::New);
        assert_eq!(d.len(), 2);
        // A third sess evicts the oldest window (≤ 2 per origin).
        d.check_and_mark(t.plus_ms(1), 1, 11, 10);
        assert_eq!(d.len(), 2);
        assert!(!d.seen(t, 1, 7, 500));
        d.on_presence(1, 11);
        assert_eq!(d.len(), 1);
        assert!(d.seen(t, 1, 11, 10));
    }

    #[test]
    fn expiry_after_five_seconds() {
        let mut d = Dedup::new();
        let t = Instant::from_secs(1);
        d.check_and_mark(t, 1, 7, 500);
        assert!(d.seen(t.plus_ms(4_999), 1, 7, 500));
        assert!(!d.seen(t.plus_ms(5_000), 1, 7, 500));
        assert_eq!(
            d.check_and_mark(t.plus_ms(5_000), 1, 7, 500),
            DedupVerdict::New
        );
        d.expire(t.plus_ms(20_000));
        assert!(d.is_empty());
    }

    proptest! {
        /// Against a reference model: every seq within the window is reported New exactly
        /// once; nothing more than 1024 behind the high is ever New.
        #[test]
        fn matches_reference(start in any::<u16>(), steps in proptest::collection::vec(-1100i32..1100, 1..300)) {
            let mut d = Dedup::new();
            let t = Instant::from_secs(1);
            let mut seen = std::collections::BTreeSet::new();
            let mut high: Option<i64> = None;
            let mut cur = i64::from(start);
            for s in steps {
                cur += i64::from(s);
                if let Some(h) = high {
                    // Stay well inside serial-number range of the high.
                    cur = cur.max(h - 2_000);
                }
                let seq = cur.rem_euclid(65_536) as u16;
                let v = d.check_and_mark(t, 3, 1, seq);
                let h = high.unwrap_or(cur);
                let expected = if cur >= h - 1024 {
                    if seen.contains(&cur) { DedupVerdict::Duplicate } else { DedupVerdict::New }
                } else {
                    DedupVerdict::TooOld
                };
                prop_assert_eq!(v, expected);
                if v == DedupVerdict::New {
                    seen.insert(cur);
                    high = Some(high.map_or(cur, |h| h.max(cur)));
                }
            }
        }
    }
}
