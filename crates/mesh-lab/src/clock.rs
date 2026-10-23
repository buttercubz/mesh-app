//! NTP-style clock-offset estimator (DESIGN §12.1): per link, from four timestamps
//! (`t1` local transmit, `t2` remote receive, `t3` remote transmit, `t4` local receive)
//! with a minimum-RTT filter. Offsets are `remote − local` in µs; one-way delays use
//! `owd = t_rx_local − (t_tx_remote − offset)`.

/// One four-timestamp exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exchange {
    /// Local transmit (local clock, µs).
    pub t1: u64,
    /// Remote receive (remote clock, µs).
    pub t2: u64,
    /// Remote transmit (remote clock, µs).
    pub t3: u64,
    /// Local receive (local clock, µs).
    pub t4: u64,
}

impl Exchange {
    /// Round-trip network delay `(t4 − t1) − (t3 − t2)` in µs (may be < 0 on garbage).
    pub fn delay_us(&self) -> i64 {
        (self.t4 as i64 - self.t1 as i64) - (self.t3 as i64 - self.t2 as i64)
    }

    /// Offset `((t2 − t1) + (t3 − t4)) / 2` = remote − local, in µs.
    pub fn offset_us(&self) -> i64 {
        ((self.t2 as i128 - self.t1 as i128 + self.t3 as i128 - self.t4 as i128) / 2) as i64
    }
}

/// A clock estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockEstimate {
    /// `remote − local` in µs.
    pub offset_us: i64,
    /// The minimum round-trip delay in the window, µs.
    pub min_delay_us: i64,
    /// Samples the estimate is based on.
    pub samples: u32,
}

/// Minimum-RTT-filtered offset estimator over a sliding window of exchanges.
#[derive(Debug, Clone)]
pub struct ClockEstimator {
    window: Vec<Exchange>,
    cap: usize,
    next: usize,
    slack_us: i64,
    accepted: u64,
}

/// Default window: the last 256 exchanges (≈ 26 s at 10 Hz, 51 s at 5 Hz). Long enough
/// for the minimum-delay filter to hold within 2 ms under 20 ms of uniform jitter per
/// direction; short enough that 20 ppm of relative drift stays near 1 ms.
pub const DEFAULT_WINDOW: usize = 256;
/// Samples within this much of the minimum delay join the estimate (median offset).
pub const DEFAULT_SLACK_US: i64 = 1_000;

impl Default for ClockEstimator {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW, DEFAULT_SLACK_US)
    }
}

impl ClockEstimator {
    pub fn new(window: usize, slack_us: i64) -> Self {
        ClockEstimator {
            window: Vec::with_capacity(window.max(1)),
            cap: window.max(1),
            next: 0,
            slack_us: slack_us.max(0),
            accepted: 0,
        }
    }

    /// Adds an exchange; returns `false` (and ignores it) when it is inconsistent
    /// (negative delay or a remote hold longer than the round trip).
    pub fn add(&mut self, x: Exchange) -> bool {
        if x.t4 < x.t1 || x.t3 < x.t2 || x.delay_us() < 0 {
            return false;
        }
        if self.window.len() < self.cap {
            self.window.push(x);
        } else {
            self.window[self.next] = x;
        }
        self.next = (self.next + 1) % self.cap;
        self.accepted += 1;
        true
    }

    /// Adds an exchange built from a LAB ping: the initiator knows `t1` and `t4`, the
    /// reply carries the responder's transmit time `t3` and `hold_us = t3 − t2`.
    pub fn add_ping(&mut self, t1: u64, t4: u64, t3_remote: u64, hold_us: u64) -> bool {
        let Some(t2) = t3_remote.checked_sub(hold_us) else {
            return false;
        };
        self.add(Exchange {
            t1,
            t2,
            t3: t3_remote,
            t4,
        })
    }

    /// Exchanges accepted so far.
    pub fn accepted(&self) -> u64 {
        self.accepted
    }

    /// The current estimate: the median offset of the window's samples whose delay is
    /// within `slack` of the window minimum (the minimum-delay sample alone when it is
    /// the only one). `None` before the first exchange.
    pub fn estimate(&self) -> Option<ClockEstimate> {
        let min_delay = self.window.iter().map(Exchange::delay_us).min()?;
        let mut best: Vec<i64> = self
            .window
            .iter()
            .filter(|x| x.delay_us() <= min_delay + self.slack_us)
            .map(Exchange::offset_us)
            .collect();
        best.sort_unstable();
        let n = best.len();
        let offset = if n % 2 == 1 {
            best[n / 2]
        } else {
            (best[n / 2 - 1] + best[n / 2]) / 2
        };
        Some(ClockEstimate {
            offset_us: offset,
            min_delay_us: min_delay,
            samples: n as u32,
        })
    }

    /// One-way delay in µs of a message sent at `t_tx_remote` (remote clock) and
    /// received at `t_rx_local`, or `None` without an estimate.
    pub fn one_way_us(&self, t_tx_remote: u64, t_rx_local: u64) -> Option<i64> {
        let e = self.estimate()?;
        Some(t_rx_local as i64 - t_tx_remote as i64 + e.offset_us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift64* in [0, 1).
    struct Rng(u64);
    impl Rng {
        fn next_f64(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Simulates pings over a link with a fixed base delay and up to `jitter_us` of
    /// independent uniform jitter per direction.
    fn simulate(
        offset_us: i64,
        base_us: u64,
        jitter_us: f64,
        n: usize,
        seed: u64,
    ) -> ClockEstimator {
        let mut rng = Rng(seed);
        let mut est = ClockEstimator::default();
        let remote = |local: u64| (local as i64 + offset_us) as u64;
        let mut t = 10_000_000u64;
        for _ in 0..n {
            let up = base_us + (rng.next_f64() * jitter_us) as u64;
            let down = base_us + (rng.next_f64() * jitter_us) as u64;
            let hold = 100 + (rng.next_f64() * 2_000.0) as u64;
            let t1 = t;
            let t2 = remote(t1 + up);
            let t3 = t2 + hold;
            let t4 = t1 + up + hold + down;
            assert!(est.add_ping(t1, t4, t3, hold));
            t += 100_000; // 10 Hz
        }
        est
    }

    #[test]
    fn recovers_offset_within_2_ms_under_20_ms_jitter() {
        for (seed, offset) in [
            (1u64, 123_456i64),
            (7, -4_321_000),
            (99, 0),
            (2024, 987_654_321),
        ] {
            let est = simulate(offset, 15_000, 20_000.0, 256, seed);
            let e = est.estimate().unwrap();
            assert!(
                (e.offset_us - offset).abs() <= 2_000,
                "seed {seed}: estimated {} vs true {offset}",
                e.offset_us
            );
            assert!(e.min_delay_us >= 30_000 && e.min_delay_us <= 50_000);
        }
    }

    #[test]
    fn exact_without_jitter_and_one_way() {
        let est = simulate(5_000_000, 10_000, 0.0, 10, 3);
        let e = est.estimate().unwrap();
        assert_eq!(e.offset_us, 5_000_000);
        assert_eq!(e.min_delay_us, 20_000);
        // Remote sends at remote time R; it arrives 10 ms later on the local clock.
        let local_send = 50_000_000u64;
        let r = local_send + 5_000_000;
        assert_eq!(est.one_way_us(r, local_send + 10_000), Some(10_000));
    }

    #[test]
    fn rejects_inconsistent_exchanges() {
        let mut est = ClockEstimator::default();
        assert!(est.estimate().is_none());
        assert!(
            !est.add(Exchange {
                t1: 10,
                t2: 0,
                t3: 100,
                t4: 20
            }),
            "hold > rtt"
        );
        assert!(!est.add_ping(10, 5, 100, 1), "t4 < t1");
        assert!(!est.add_ping(10, 50, 5, 10), "t3 < hold");
        assert_eq!(est.accepted(), 0);
    }

    #[test]
    fn window_slides() {
        let mut est = ClockEstimator::new(4, 0);
        for i in 0..4u64 {
            est.add(Exchange {
                t1: i * 100,
                t2: 1_000 + i * 100,
                t3: 1_000 + i * 100,
                t4: i * 100 + 10,
            });
        }
        assert_eq!(est.estimate().unwrap().samples, 4);
        // A new regime replaces the old samples after a full window.
        for i in 4..8u64 {
            est.add(Exchange {
                t1: i * 100,
                t2: 2_000 + i * 100,
                t3: 2_000 + i * 100,
                t4: i * 100 + 10,
            });
        }
        assert_eq!(est.estimate().unwrap().offset_us, 1_995);
    }
}
