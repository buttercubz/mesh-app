//! 1024-bit sliding replay window per direction and key phase (DESIGN §4.3).
//!
//! [`ReplayWindow::check`] never mutates; [`ReplayWindow::commit`] is called only after
//! the AEAD tag verified, so a forged frame can never move the window.

/// Window size in counters (Appendix A).
pub const REPLAY_WINDOW_BITS: u32 = 1024;
const WORDS: usize = (REPLAY_WINDOW_BITS / 64) as usize;

/// Sliding window over the 31-bit SEC counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayWindow {
    /// Highest accepted counter, `None` before the first accepted frame.
    top: Option<u32>,
    /// Bit `ctr mod 1024` is set when `ctr` (within the window) was accepted.
    bits: [u64; WORDS],
}

impl Default for ReplayWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayWindow {
    pub const fn new() -> Self {
        ReplayWindow {
            top: None,
            bits: [0; WORDS],
        }
    }

    /// Highest accepted counter.
    pub fn top(&self) -> Option<u32> {
        self.top
    }

    fn slot(ctr: u32) -> (usize, u64) {
        let i = ctr % REPLAY_WINDOW_BITS;
        ((i / 64) as usize, 1u64 << (i % 64))
    }

    /// `true` when `ctr` would be accepted (not seen, not older than the window).
    pub fn check(&self, ctr: u32) -> bool {
        match self.top {
            None => true,
            Some(top) if ctr > top => true,
            Some(top) if top - ctr >= REPLAY_WINDOW_BITS => false,
            Some(_) => {
                let (w, m) = Self::slot(ctr);
                self.bits[w] & m == 0
            }
        }
    }

    /// Marks `ctr` as accepted. Callers must have verified the tag and called
    /// [`ReplayWindow::check`]; a counter that fails `check` is ignored.
    pub fn commit(&mut self, ctr: u32) {
        if !self.check(ctr) {
            return;
        }
        match self.top {
            Some(top) if ctr <= top => {}
            Some(top) => {
                let advance = ctr - top;
                if advance >= REPLAY_WINDOW_BITS {
                    self.bits = [0; WORDS];
                } else {
                    for c in top + 1..ctr {
                        let (w, m) = Self::slot(c);
                        self.bits[w] &= !m;
                    }
                }
                self.top = Some(ctr);
            }
            None => {
                self.bits = [0; WORDS];
                self.top = Some(ctr);
            }
        }
        let (w, m) = Self::slot(ctr);
        self.bits[w] |= m;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::{RngCore, SeedableRng};
    use std::collections::BTreeSet;

    #[test]
    fn basic_window() {
        let mut w = ReplayWindow::new();
        assert!(w.check(5));
        w.commit(5);
        assert!(!w.check(5));
        assert!(w.check(4));
        assert!(w.check(6));
        w.commit(2000);
        assert!(!w.check(5), "older than the window");
        assert!(w.check(2000 - 1023));
        assert!(!w.check(2000 - 1024));
        w.commit(1999);
        assert!(!w.check(1999));
        w.commit(0x7FFF_FFFF);
        assert!(!w.check(0x7FFF_FFFF));
        assert!(w.check(0x7FFF_FFFE));
    }

    #[test]
    fn slots_are_cleared_when_sliding() {
        let mut w = ReplayWindow::new();
        w.commit(10);
        w.commit(10 + 1024 + 5);
        // 10 + 1024 maps to the same slot as 10 and must be acceptable once.
        assert!(w.check(10 + 1024));
        w.commit(10 + 1024);
        assert!(!w.check(10 + 1024));
    }

    /// Model-based property test (seeded ChaCha20Rng cases; proptest is not a dependency
    /// of this crate because it pulls OS randomness). The window must match an exact
    /// model: accept iff never accepted before and `top − ctr < 1024`.
    #[test]
    fn property_never_accepts_twice_and_matches_model() {
        let mut rng = ChaCha20Rng::from_seed([42; 32]);
        for case in 0..256 {
            let mut w = ReplayWindow::new();
            let mut accepted: BTreeSet<u32> = BTreeSet::new();
            let mut top: Option<u32> = None;
            let base = rng.next_u32() & 0x3FFF_FFFF;
            for _ in 0..400 {
                let r = rng.next_u32();
                let ctr = match r % 4 {
                    0 => base.saturating_add(r % 64),
                    1 => base.saturating_add(r % 4096),
                    2 => top.map_or(base, |t| t.saturating_sub(r % 1100)),
                    _ => accepted.iter().next_back().copied().unwrap_or(base),
                } & 0x7FFF_FFFF;
                let model_ok = !accepted.contains(&ctr)
                    && top.is_none_or(|t| ctr > t || t - ctr < REPLAY_WINDOW_BITS);
                assert_eq!(w.check(ctr), model_ok, "case {case} ctr {ctr}");
                // Forged frames (tag failure) never commit: the window is unchanged.
                if r % 7 == 0 {
                    let before = w.clone();
                    let _ = w.check(ctr);
                    assert_eq!(before, w);
                    continue;
                }
                if model_ok {
                    w.commit(ctr);
                    accepted.insert(ctr);
                    top = Some(top.map_or(ctr, |t| t.max(ctr)));
                    assert!(!w.check(ctr), "accepted twice");
                }
            }
        }
    }
}
