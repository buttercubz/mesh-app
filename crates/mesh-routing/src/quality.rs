//! Link quality (DESIGN §5.1): RTT estimation from PING/PONG, windowed `min_rtt`, a
//! per-second loss EWMA and the `q` formula
//! `q = 100 · (1 − loss)² · f_rtt · f_rssi`.

use crate::consts::{LOSS_ALPHA_PERMILLE, MIN_RTT_WINDOW_US, Q_GOOD};
use crate::types::{Instant, LinkKind};

/// Smoothed RTT (RFC 6298 gains 1/8 and 1/4) plus `min_rtt` over a 30 s window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RttEstimator {
    srtt_us: Option<u64>,
    rttvar_us: u64,
    /// Minimum per 5 s bucket: `(bucket start, min)`; six buckets cover 30 s.
    buckets: [(u64, u64); 6],
    nbuckets: usize,
}

const BUCKET_US: u64 = MIN_RTT_WINDOW_US / 6;

impl RttEstimator {
    /// Feeds one RTT sample.
    pub fn sample(&mut self, now: Instant, rtt_us: u64) {
        match self.srtt_us {
            None => {
                self.srtt_us = Some(rtt_us);
                self.rttvar_us = rtt_us / 2;
            }
            Some(s) => {
                let err = s.abs_diff(rtt_us);
                self.rttvar_us = (3 * self.rttvar_us + err) / 4;
                self.srtt_us = Some((7 * s + rtt_us) / 8);
            }
        }
        let start = now.us() - now.us() % BUCKET_US;
        if self.nbuckets > 0 && self.buckets[self.nbuckets - 1].0 == start {
            let b = &mut self.buckets[self.nbuckets - 1];
            b.1 = b.1.min(rtt_us);
        } else {
            if self.nbuckets == self.buckets.len() {
                self.buckets.rotate_left(1);
                self.nbuckets -= 1;
            }
            self.buckets[self.nbuckets] = (start, rtt_us);
            self.nbuckets += 1;
        }
    }

    /// Smoothed RTT, if any sample was seen.
    pub fn srtt_us(&self) -> Option<u64> {
        self.srtt_us
    }

    /// RTT variation.
    pub fn rttvar_us(&self) -> u64 {
        self.rttvar_us
    }

    /// Minimum RTT over the last 30 s (`None` without samples in the window).
    pub fn min_rtt_us(&self, now: Instant) -> Option<u64> {
        self.buckets[..self.nbuckets]
            .iter()
            .filter(|(start, _)| now.us() < start + MIN_RTT_WINDOW_US)
            .map(|(_, m)| *m)
            .min()
    }

    /// Queueing delay estimate `srtt − min_rtt` (0 without samples).
    pub fn queue_delay_us(&self, now: Instant) -> u64 {
        match (self.srtt_us, self.min_rtt_us(now)) {
            (Some(s), Some(m)) => s.saturating_sub(m),
            _ => 0,
        }
    }

    /// One-way delay estimate `srtt / 2` (age rewrite, §4.5).
    pub fn owd_us(&self) -> u64 {
        self.srtt_us.unwrap_or(0) / 2
    }
}

/// Loss EWMA (α = 0.2 per second) of SEC counter gaps and PING loss.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LossEstimator {
    /// Loss in parts per million.
    ewma_ppm: u32,
    expected: u32,
    lost: u32,
    last_update: Option<Instant>,
}

impl LossEstimator {
    /// Records `expected` units (frames or PINGs) of which `lost` were missing.
    pub fn observe(&mut self, expected: u32, lost: u32) {
        self.expected = self.expected.saturating_add(expected);
        self.lost = self.lost.saturating_add(lost.min(expected));
    }

    /// Folds the observations of each elapsed second into the EWMA.
    pub fn tick(&mut self, now: Instant) {
        let Some(last) = self.last_update else {
            self.last_update = Some(now);
            return;
        };
        if now.since(last) < 1_000_000 {
            return;
        }
        let sample = if self.expected == 0 {
            None
        } else {
            Some((u64::from(self.lost) * 1_000_000 / u64::from(self.expected)) as u32)
        };
        if let Some(s) = sample {
            let a = u64::from(LOSS_ALPHA_PERMILLE);
            self.ewma_ppm =
                ((a * u64::from(s) + (1_000 - a) * u64::from(self.ewma_ppm)) / 1_000) as u32;
        }
        self.expected = 0;
        self.lost = 0;
        self.last_update = Some(now);
    }

    /// Smoothed loss in ppm.
    pub fn loss_ppm(&self) -> u32 {
        self.ewma_ppm
    }
}

/// Everything `q` depends on, for one link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkQuality {
    pub kind: LinkKind,
    pub rtt: RttEstimator,
    pub loss: LossEstimator,
    pub rssi: Option<i16>,
}

impl LinkQuality {
    /// A new link with no samples (q = 100 until evidence says otherwise).
    pub fn new(kind: LinkKind) -> Self {
        LinkQuality {
            kind,
            rtt: RttEstimator::default(),
            loss: LossEstimator::default(),
            rssi: None,
        }
    }

    /// `q = 100 · (1 − loss)² · f_rtt · f_rssi`, rounded to 0-100.
    pub fn q(&self, now: Instant) -> u8 {
        let one_minus = 1_000_000u64.saturating_sub(u64::from(self.loss.loss_ppm()));
        let f_rtt = f_rtt_permille(self.rtt.queue_delay_us(now));
        let f_rssi = if self.kind == LinkKind::Ble {
            f_rssi_permille(self.rssi)
        } else {
            1_000
        };
        q_from(one_minus, f_rtt, f_rssi)
    }

    /// `true` when `q ≥ Q_GOOD`.
    pub fn good(&self, now: Instant) -> bool {
        self.q(now) >= Q_GOOD
    }
}

fn q_from(one_minus_ppm: u64, f_rtt: u64, f_rssi: u64) -> u8 {
    // 100 · (x/1e6)² · (f_rtt/1e3) · (f_rssi/1e3), rounded to nearest.
    let sq = one_minus_ppm * one_minus_ppm / 1_000_000; // ppm
    let num = sq * 100 * f_rtt * f_rssi; // ppm · 1e6
    let q = (num + 500_000_000_000) / 1_000_000_000_000;
    q.min(100) as u8
}

/// `f_rtt = 1` below 30 ms of queueing delay, falling linearly to 0.5 at 150 ms (and
/// staying there beyond), in permille.
pub fn f_rtt_permille(qd_us: u64) -> u64 {
    const LO: u64 = 30_000;
    const HI: u64 = 150_000;
    if qd_us < LO {
        1_000
    } else if qd_us >= HI {
        500
    } else {
        1_000 - (qd_us - LO) * 500 / (HI - LO)
    }
}

/// `f_rssi = 1` at ≥ −75 dBm, falling linearly to 0.6 at −95 dBm (BLE only), permille.
/// Unknown RSSI counts as 1.
pub fn f_rssi_permille(rssi: Option<i16>) -> u64 {
    match rssi {
        None => 1_000,
        Some(r) if r >= -75 => 1_000,
        Some(r) if r <= -95 => 600,
        Some(r) => 1_000 - (i64::from(-75 - r) as u64) * 400 / 20,
    }
}

/// UI bars from `q`: ≥ 80 → 4, ≥ 60 → 3, ≥ 40 → 2, > 0 → 1, else 0.
pub const fn bars(q: u8) -> u8 {
    match q {
        80..=u8::MAX => 4,
        60..=79 => 3,
        40..=59 => 2,
        1..=39 => 1,
        0 => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_formula() {
        let now = Instant::from_secs(10);
        let mut l = LinkQuality::new(LinkKind::Ble);
        assert_eq!(l.q(now), 100);
        l.rssi = Some(-95);
        assert_eq!(l.q(now), 60);
        l.rssi = Some(-85);
        assert_eq!(l.q(now), 80);
        l.rssi = Some(-70);
        // 10 % loss → 81.
        l.loss.tick(Instant::ZERO);
        l.loss.observe(100, 50);
        l.loss.tick(Instant::from_secs(1));
        assert_eq!(l.loss.loss_ppm(), 100_000);
        assert_eq!(l.q(now), 81);
        assert!(l.good(now));
        // RSSI is ignored on Aware.
        let mut a = LinkQuality::new(LinkKind::Aware);
        a.rssi = Some(-100);
        assert_eq!(a.q(now), 100);
    }

    #[test]
    fn rtt_factor() {
        assert_eq!(f_rtt_permille(0), 1_000);
        assert_eq!(f_rtt_permille(90_000), 750);
        assert_eq!(f_rtt_permille(150_000), 500);
        assert_eq!(f_rtt_permille(1_000_000), 500);
        let mut l = LinkQuality::new(LinkKind::Ble);
        let t = Instant::from_secs(1);
        l.rtt.sample(t, 40_000);
        for _ in 0..60 {
            l.rtt.sample(t, 190_000);
        }
        // srtt → ~190 ms, min 40 ms ⇒ qd ≈ 150 ms ⇒ f_rtt 0.5.
        assert!(l.q(t) <= 51);
        // The 40 ms minimum ages out after 30 s.
        let later = Instant::from_secs(40);
        l.rtt.sample(later, 190_000);
        assert_eq!(l.rtt.min_rtt_us(later), Some(190_000));
    }

    #[test]
    fn rtt_estimator() {
        let mut r = RttEstimator::default();
        assert_eq!(r.srtt_us(), None);
        assert_eq!(r.queue_delay_us(Instant::ZERO), 0);
        r.sample(Instant::ZERO, 80_000);
        assert_eq!(r.srtt_us(), Some(80_000));
        assert_eq!(r.owd_us(), 40_000);
        r.sample(Instant::from_ms(500), 160_000);
        assert_eq!(r.srtt_us(), Some(90_000));
        assert_eq!(r.queue_delay_us(Instant::from_ms(500)), 10_000);
    }

    #[test]
    fn loss_ewma() {
        let mut l = LossEstimator::default();
        l.tick(Instant::ZERO);
        l.observe(10, 10);
        l.tick(Instant::from_ms(500)); // not a full second yet
        l.tick(Instant::from_secs(1));
        assert_eq!(l.loss_ppm(), 200_000);
        // Seconds without traffic leave the estimate unchanged.
        l.tick(Instant::from_secs(2));
        assert_eq!(l.loss_ppm(), 200_000);
        l.observe(10, 0);
        l.tick(Instant::from_secs(3));
        assert_eq!(l.loss_ppm(), 160_000);
    }

    #[test]
    fn ui_bars() {
        assert_eq!(bars(100), 4);
        assert_eq!(bars(80), 4);
        assert_eq!(bars(79), 3);
        assert_eq!(bars(60), 3);
        assert_eq!(bars(40), 2);
        assert_eq!(bars(1), 1);
        assert_eq!(bars(0), 0);
    }
}
