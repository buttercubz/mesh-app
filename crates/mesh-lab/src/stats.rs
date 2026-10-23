//! Per-link statistics (DESIGN §12.2 T0 columns): kbps, loss, reorder, duplicates,
//! stalls > 100 ms, RTT and one-way percentiles, and the BLE connection interval
//! inferred from the RTT quantization histogram.

/// Percentile (nearest rank, `q` in 0..=1) of an ascending slice.
pub fn percentile_sorted(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let q = q.clamp(0.0, 1.0);
    let rank = (q * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// p50 / p95 / p99 of unsorted samples.
pub fn p50_95_99(samples: &[f64]) -> Option<[f64; 3]> {
    let mut s: Vec<f64> = samples.iter().copied().filter(|v| v.is_finite()).collect();
    if s.is_empty() {
        return None;
    }
    s.sort_by(f64::total_cmp);
    Some([
        percentile_sorted(&s, 0.50)?,
        percentile_sorted(&s, 0.95)?,
        percentile_sorted(&s, 0.99)?,
    ])
}

/// Bounded-memory latency histogram in µs: 10 µs bins up to 100 ms, 100 µs bins up to
/// 1 s, 1 ms bins up to 10 s, then one overflow bin. Percentiles are bin upper edges,
/// so the error is below one bin width.
#[derive(Debug, Clone)]
pub struct Histogram {
    bins: Vec<u32>,
    count: u64,
    sum_us: u128,
    min_us: u64,
    max_us: u64,
}

const H1: u64 = 100_000; // 10 µs bins
const H2: u64 = 1_000_000; // 100 µs bins
const H3: u64 = 10_000_000; // 1 ms bins
const N1: usize = (H1 / 10) as usize;
const N2: usize = ((H2 - H1) / 100) as usize;
const N3: usize = ((H3 - H2) / 1_000) as usize;

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    pub fn new() -> Self {
        Histogram {
            bins: vec![0; N1 + N2 + N3 + 1],
            count: 0,
            sum_us: 0,
            min_us: u64::MAX,
            max_us: 0,
        }
    }

    fn bin_of(us: u64) -> usize {
        if us < H1 {
            (us / 10) as usize
        } else if us < H2 {
            N1 + ((us - H1) / 100) as usize
        } else if us < H3 {
            N1 + N2 + ((us - H2) / 1_000) as usize
        } else {
            N1 + N2 + N3
        }
    }

    fn upper_edge(bin: usize) -> u64 {
        if bin < N1 {
            (bin as u64 + 1) * 10
        } else if bin < N1 + N2 {
            H1 + ((bin - N1) as u64 + 1) * 100
        } else if bin < N1 + N2 + N3 {
            H2 + ((bin - N1 - N2) as u64 + 1) * 1_000
        } else {
            u64::MAX
        }
    }

    pub fn record(&mut self, us: u64) {
        let b = Self::bin_of(us);
        self.bins[b] = self.bins[b].saturating_add(1);
        self.count += 1;
        self.sum_us += u128::from(us);
        self.min_us = self.min_us.min(us);
        self.max_us = self.max_us.max(us);
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    /// Adds every sample of `other`.
    pub fn merge(&mut self, other: &Histogram) {
        for (a, b) in self.bins.iter_mut().zip(&other.bins) {
            *a = a.saturating_add(*b);
        }
        self.count += other.count;
        self.sum_us += other.sum_us;
        self.min_us = self.min_us.min(other.min_us);
        self.max_us = self.max_us.max(other.max_us);
    }

    /// Mean in µs.
    pub fn mean_us(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum_us as f64 / self.count as f64)
    }

    /// Nearest-rank percentile in µs (bin upper edge, clamped to the observed max).
    pub fn percentile_us(&self, q: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let rank = ((q.clamp(0.0, 1.0) * self.count as f64).ceil() as u64).max(1);
        let mut acc = 0u64;
        for (i, n) in self.bins.iter().enumerate() {
            acc += u64::from(*n);
            if acc >= rank {
                return Some(Self::upper_edge(i).min(self.max_us).max(self.min_us));
            }
        }
        Some(self.max_us)
    }

    /// p50/p95/p99 in ms.
    pub fn p50_95_99_ms(&self) -> Option<[f64; 3]> {
        Some([
            self.percentile_us(0.50)? as f64 / 1e3,
            self.percentile_us(0.95)? as f64 / 1e3,
            self.percentile_us(0.99)? as f64 / 1e3,
        ])
    }

    /// Fraction of samples (by bin) whose value lies within `tol_us` of a multiple of
    /// `period_us` (used for the connection-interval inference).
    fn alignment(&self, period_us: u64, tol_us: u64) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let mut hit = 0u64;
        for (i, n) in self.bins.iter().enumerate() {
            if *n == 0 || i >= N1 + N2 + N3 {
                continue;
            }
            let mid = Self::upper_edge(i).saturating_sub(Self::bin_width(i) / 2);
            let r = mid % period_us;
            if r <= tol_us || period_us - r <= tol_us {
                hit += u64::from(*n);
            }
        }
        hit as f64 / self.count as f64
    }

    fn bin_width(bin: usize) -> u64 {
        if bin < N1 {
            10
        } else if bin < N1 + N2 {
            100
        } else {
            1_000
        }
    }
}

/// Infers the BLE connection interval from RTT quantization: RTTs cluster near
/// multiples of the interval. Candidates are the legal intervals (multiples of
/// 1.25 ms from 7.5 to 100 ms); the longest candidate that explains at least 80 % of
/// the samples within ±1 ms wins. Returns ms, `None` with fewer than 20 samples or no
/// fit.
pub fn infer_conn_interval_ms(rtt: &Histogram) -> Option<f64> {
    if rtt.count() < 20 {
        return None;
    }
    let mut best = None;
    let mut k = 6u64; // 6 × 1.25 = 7.5 ms
    while k <= 80 {
        let period = k * 1_250;
        if rtt.alignment(period, 1_000) >= 0.8 {
            best = Some(period);
        }
        k += 1;
    }
    best.map(|p| p as f64 / 1e3)
}

/// Sequence tracker for received probe ids (one per link and direction): counts
/// received, lost (gaps), late arrivals that fill a gap (reorder) and duplicates over a
/// 1024-id window.
#[derive(Debug, Clone, Default)]
pub struct SeqTracker {
    started: bool,
    highest: u32,
    window: [u64; 16],
    pub received: u64,
    pub lost: u64,
    pub reordered: u64,
    pub duplicates: u64,
}

const WINDOW: u32 = 1024;

impl SeqTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn bit(&self, id: u32) -> bool {
        let i = (id % WINDOW) as usize;
        self.window[i / 64] & (1 << (i % 64)) != 0
    }

    fn set(&mut self, id: u32, on: bool) {
        let i = (id % WINDOW) as usize;
        if on {
            self.window[i / 64] |= 1 << (i % 64);
        } else {
            self.window[i / 64] &= !(1 << (i % 64));
        }
    }

    /// Records `id`; returns `false` for a duplicate or an id too old to judge.
    pub fn on_id(&mut self, id: u32) -> bool {
        if !self.started {
            self.started = true;
            self.highest = id;
            self.set(id, true);
            self.received += 1;
            return true;
        }
        let ahead = id.wrapping_sub(self.highest);
        if ahead == 0 {
            self.duplicates += 1;
            return false;
        }
        if ahead < u32::MAX / 2 {
            // New highest: ids in between are (provisionally) lost.
            let gap = ahead - 1;
            let clear = ahead.min(WINDOW);
            for k in 1..=clear {
                self.set(self.highest.wrapping_add(k), false);
            }
            self.lost += u64::from(gap);
            self.highest = id;
            self.set(id, true);
            self.received += 1;
            return true;
        }
        let behind = self.highest.wrapping_sub(id);
        if behind >= WINDOW {
            return false;
        }
        if self.bit(id) {
            self.duplicates += 1;
            return false;
        }
        self.set(id, true);
        self.received += 1;
        self.reordered += 1;
        self.lost = self.lost.saturating_sub(1);
        true
    }

    /// Loss ratio `lost / (received + lost)`.
    pub fn loss(&self) -> f64 {
        let total = self.received + self.lost;
        if total == 0 {
            0.0
        } else {
            self.lost as f64 / total as f64
        }
    }
}

/// Stall detector: a gap > `threshold` between consecutive arrivals while traffic is
/// expected is one stall.
#[derive(Debug, Clone)]
pub struct StallDetector {
    threshold_us: u64,
    last_us: Option<u64>,
    pub stalls: u32,
    pub longest_us: u64,
}

/// Stalls are gaps longer than 100 ms (§12.2).
pub const STALL_US: u64 = 100_000;

impl StallDetector {
    pub fn new(threshold_us: u64) -> Self {
        StallDetector {
            threshold_us,
            last_us: None,
            stalls: 0,
            longest_us: 0,
        }
    }

    /// Records an arrival; returns the gap if it was a stall.
    pub fn on_arrival(&mut self, t_us: u64) -> Option<u64> {
        let gap = self.last_us.map(|l| t_us.saturating_sub(l));
        self.last_us = Some(t_us);
        let g = gap?;
        if g > self.threshold_us {
            self.stalls += 1;
            self.longest_us = self.longest_us.max(g);
            Some(g)
        } else {
            None
        }
    }

    /// Forgets the last arrival (link down: the reconnect gap is not a stall).
    pub fn reset(&mut self) {
        self.last_us = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(p50_95_99(&v), Some([50.0, 95.0, 99.0]));
        assert_eq!(p50_95_99(&[]), None);
        assert_eq!(percentile_sorted(&[3.0], 0.99), Some(3.0));
        let mut h = Histogram::new();
        for ms in 1..=100u64 {
            h.record(ms * 1_000);
        }
        let [p50, p95, p99] = h.p50_95_99_ms().unwrap();
        assert!(
            (p50 - 50.0).abs() <= 0.1 && (p95 - 95.0).abs() <= 0.1 && (p99 - 99.0).abs() <= 0.1
        );
        assert!((h.mean_us().unwrap() - 50_500.0).abs() < 1e-6);
        h.record(60_000_000);
        assert_eq!(h.percentile_us(1.0), Some(60_000_000));
    }

    #[test]
    fn sequence_loss_reorder_duplicates() {
        let mut s = SeqTracker::new();
        for id in [0u32, 1, 2, 5, 3, 4, 4, 6] {
            s.on_id(id);
        }
        assert_eq!(s.received, 7);
        assert_eq!(s.lost, 0, "3 and 4 arrived late");
        assert_eq!(s.reordered, 2);
        assert_eq!(s.duplicates, 1);
        s.on_id(10);
        assert_eq!(s.lost, 3);
        assert!((s.loss() - 3.0 / 11.0).abs() < 1e-12);
        // Too old to judge.
        s.on_id(10u32.wrapping_sub(2000));
        assert_eq!(s.received, 8);
        // Wrap-around keeps counting forward.
        let mut w = SeqTracker::new();
        w.on_id(u32::MAX);
        w.on_id(0);
        assert_eq!((w.received, w.lost), (2, 0));
    }

    #[test]
    fn stalls_over_100_ms() {
        let mut d = StallDetector::new(STALL_US);
        assert_eq!(d.on_arrival(0), None);
        assert_eq!(d.on_arrival(40_000), None);
        assert_eq!(d.on_arrival(190_000), Some(150_000));
        d.reset();
        assert_eq!(d.on_arrival(900_000), None);
        assert_eq!((d.stalls, d.longest_us), (1, 150_000));
    }

    #[test]
    fn connection_interval_from_rtt_quantization() {
        let mut h = Histogram::new();
        // RTTs quantized to multiples of 30 ms (with ±0.3 ms noise).
        for i in 0..100u64 {
            let k = 1 + (i % 3);
            let noise = (i % 7) * 100;
            h.record(k * 30_000 + noise - 300);
        }
        assert_eq!(infer_conn_interval_ms(&h), Some(30.0));
        let mut few = Histogram::new();
        few.record(30_000);
        assert_eq!(infer_conn_interval_ms(&few), None);
    }
}
