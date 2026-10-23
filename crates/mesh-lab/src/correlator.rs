//! Loopback correlator (DESIGN §12.2 T5a, T5b): the core plays an MLS click train into
//! the mixer and finds it in the capture by cross-correlation. T5a measures the round
//! trip (OS + both HFP legs); T5b measures the echo return (delay, gain, echo return
//! loss) with VPIO on.

/// Maximum-length-sequence click train at the 16 kHz core rate.
#[derive(Debug, Clone, PartialEq)]
pub struct ClickTrain {
    samples: Vec<i16>,
    energy: f64,
}

/// Feedback taps of primitive polynomials (exponents, highest first) per order.
fn taps(order: u32) -> &'static [u32] {
    match order {
        5 => &[5, 3],
        6 => &[6, 5],
        7 => &[7, 6],
        8 => &[8, 6, 5, 4],
        9 => &[9, 5],
        11 => &[11, 9],
        12 => &[12, 6, 4, 1],
        _ => &[10, 7],
    }
}

/// The ±1 maximum-length sequence of `order` (5-12), length `2^order − 1`.
///
/// Fibonacci LFSR: the register holds `s[n..n+order]` with `s[n]` in bit 0 (the output);
/// the recurrence is `s[n+order] = XOR over taps t of s[n+order−t]`.
pub fn mls(order: u32) -> Vec<i8> {
    let order = order.clamp(5, 12);
    let len = (1usize << order) - 1;
    let taps = taps(order);
    let mut state: u32 = 1;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        out.push(if state & 1 == 1 { 1 } else { -1 });
        let fb = taps
            .iter()
            .fold(0u32, |acc, t| acc ^ ((state >> (order - t)) & 1));
        state = (state >> 1) | (fb << (order - 1));
    }
    out
}

impl ClickTrain {
    /// An MLS of `order` where every chip is a raised-cosine click of `samples_per_chip`
    /// samples (≥ 1) with the chip's sign, at `amplitude` (0..1 of full scale). With 4
    /// samples per chip the energy sits below 4 kHz, so narrowband HFP passes it.
    pub fn new(order: u32, samples_per_chip: usize, amplitude: f64) -> Self {
        let spc = samples_per_chip.max(1);
        let amp = amplitude.clamp(0.0, 1.0) * f64::from(i16::MAX);
        let pulse: Vec<f64> = (0..spc)
            .map(|i| {
                if spc == 1 {
                    1.0
                } else {
                    let x = (i as f64 + 0.5) / spc as f64;
                    0.5 - 0.5 * (2.0 * core::f64::consts::PI * x).cos()
                }
            })
            .collect();
        let mut samples = Vec::new();
        for chip in mls(order) {
            for p in &pulse {
                samples.push((f64::from(chip) * p * amp).round() as i16);
            }
        }
        let energy = samples.iter().map(|s| f64::from(*s).powi(2)).sum();
        ClickTrain { samples, energy }
    }

    /// The default bench train: order 10 (1023 chips), 4 samples per chip, −6 dBFS
    /// (256 ms at 16 kHz).
    pub fn bench_default() -> Self {
        Self::new(10, 4, 0.5)
    }

    /// The samples to render.
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Result of one correlation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopbackResult {
    /// Delay in whole samples from the emit index to the train in the capture.
    pub delay_samples: u64,
    /// Sub-sample refinement (parabolic interpolation), in samples (|x| ≤ 0.5).
    pub fraction: f64,
    /// `delay` in ms at `sample_rate`.
    pub delay_ms: f64,
    /// Normalized cross-correlation at the peak (0..1).
    pub ncc: f64,
    /// Linear echo gain (captured amplitude / rendered amplitude).
    pub gain: f64,
    /// Echo return loss in dB (`−20·log10|gain|`).
    pub erl_db: f64,
    /// Peak-to-sidelobe ratio (peak / largest |corr| outside ±2 chips), linear.
    pub psr: f64,
}

/// Streaming correlator for one emission.
#[derive(Debug, Clone)]
pub struct LoopbackCorrelator {
    train: ClickTrain,
    sample_rate: u32,
    max_delay: usize,
    emit_at: Option<u64>,
    captured: Vec<i16>,
    next_index: u64,
    /// Minimum normalized correlation to accept a detection.
    pub min_ncc: f64,
}

impl LoopbackCorrelator {
    /// `max_delay_ms` bounds the search (500 ms covers HFP round trips).
    pub fn new(train: ClickTrain, sample_rate: u32, max_delay_ms: u32) -> Self {
        let max_delay = (u64::from(sample_rate) * u64::from(max_delay_ms) / 1_000) as usize;
        LoopbackCorrelator {
            train,
            sample_rate: sample_rate.max(1),
            max_delay,
            emit_at: None,
            captured: Vec::new(),
            next_index: 0,
            min_ncc: 0.2,
        }
    }

    pub fn train(&self) -> &ClickTrain {
        &self.train
    }

    /// The train's first sample was handed to render when the capture stream was at
    /// sample index `capture_index` (same counter as [`LoopbackCorrelator::push_capture`]).
    pub fn emit(&mut self, capture_index: u64) {
        self.emit_at = Some(capture_index);
        self.captured.clear();
        self.captured.reserve(self.needed());
    }

    fn needed(&self) -> usize {
        self.max_delay + self.train.len()
    }

    /// Feeds capture samples that start at capture sample index `index`.
    pub fn push_capture(&mut self, index: u64, samples: &[i16]) {
        self.next_index = index + samples.len() as u64;
        let Some(start) = self.emit_at else { return };
        let need = self.needed();
        for (k, s) in samples.iter().enumerate() {
            let i = index + k as u64;
            if i >= start && self.captured.len() < need && i - start == self.captured.len() as u64 {
                self.captured.push(*s);
            }
        }
    }

    /// Enough capture has arrived to search the whole delay range.
    pub fn ready(&self) -> bool {
        self.emit_at.is_some() && self.captured.len() >= self.needed()
    }

    /// Correlates (once [`LoopbackCorrelator::ready`]); `None` when nothing correlates
    /// above `min_ncc`.
    pub fn result(&self) -> Option<LoopbackResult> {
        if !self.ready() || self.train.energy <= 0.0 {
            return None;
        }
        correlate(
            &self.train,
            &self.captured,
            self.max_delay,
            self.sample_rate,
            self.min_ncc,
        )
    }
}

/// Cross-correlates `train` against `cap` for lags `0..=max_delay`.
fn correlate(
    train: &ClickTrain,
    cap: &[i16],
    max_delay: usize,
    sample_rate: u32,
    min_ncc: f64,
) -> Option<LoopbackResult> {
    let r: Vec<f64> = train.samples.iter().map(|s| f64::from(*s)).collect();
    let c: Vec<f64> = cap.iter().map(|s| f64::from(*s)).collect();
    let n = r.len();
    let lags = max_delay.min(c.len().saturating_sub(n)) + 1;
    // Prefix sums of capture energy for the per-lag normalization.
    let mut e = Vec::with_capacity(c.len() + 1);
    e.push(0.0f64);
    for v in &c {
        let last = *e.last().unwrap_or(&0.0);
        e.push(last + v * v);
    }
    let corr = cross_correlate(&r, &c, lags);
    let (peak, &pv) = corr
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))?;
    let cap_energy = e[peak + n] - e[peak];
    if cap_energy <= 0.0 {
        return None;
    }
    let ncc = pv.abs() / (train.energy * cap_energy).sqrt();
    if ncc < min_ncc {
        return None;
    }
    let fraction = if peak > 0 && peak + 1 < lags {
        let (a, b, d) = (corr[peak - 1].abs(), pv.abs(), corr[peak + 1].abs());
        let den = a - 2.0 * b + d;
        if den.abs() > f64::EPSILON {
            (0.5 * (a - d) / den).clamp(-0.5, 0.5)
        } else {
            0.0
        }
    } else {
        0.0
    };
    // Sidelobes: everything farther than two chips (8 samples) from the peak.
    let guard = 8usize;
    let side = corr
        .iter()
        .enumerate()
        .filter(|(i, _)| i.abs_diff(peak) > guard)
        .map(|(_, v)| v.abs())
        .fold(0.0f64, f64::max);
    let gain = pv / train.energy;
    let delay = peak as f64 + fraction;
    Some(LoopbackResult {
        delay_samples: peak as u64,
        fraction,
        delay_ms: delay * 1_000.0 / f64::from(sample_rate),
        ncc,
        gain,
        erl_db: if gain == 0.0 {
            f64::INFINITY
        } else {
            -20.0 * gain.abs().log10()
        },
        psr: if side > 0.0 {
            pv.abs() / side
        } else {
            f64::INFINITY
        },
    })
}

/// `out[lag] = Σ r[n]·c[lag+n]` for `lag < lags`, via a zero-padded radix-2 FFT
/// (`O(N log N)`; the direct sum is 30 M multiply-adds per 500 ms search).
fn cross_correlate(r: &[f64], c: &[f64], lags: usize) -> Vec<f64> {
    let n = (c.len() + r.len()).next_power_of_two();
    let (mut rr, mut ri) = (vec![0.0; n], vec![0.0; n]);
    let (mut cr, mut ci) = (vec![0.0; n], vec![0.0; n]);
    rr[..r.len()].copy_from_slice(r);
    cr[..c.len()].copy_from_slice(c);
    fft(&mut rr, &mut ri, false);
    fft(&mut cr, &mut ci, false);
    // conj(R) · C
    for k in 0..n {
        let (a, b) = (rr[k], -ri[k]);
        let (x, y) = (cr[k], ci[k]);
        cr[k] = a * x - b * y;
        ci[k] = a * y + b * x;
    }
    fft(&mut cr, &mut ci, true);
    cr.truncate(lags);
    cr
}

/// In-place iterative radix-2 FFT (`re.len()` a power of two). The inverse is scaled
/// by `1/n`.
fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * 2.0 * core::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
    if inverse {
        let inv = 1.0 / n as f64;
        re.iter_mut().for_each(|v| *v *= inv);
        im.iter_mut().for_each(|v| *v *= inv);
    }
}

/// T5a series: round trips of repeated click trains and the per-leg estimate.
#[derive(Debug, Clone, Default)]
pub struct LoopbackSeries {
    delays_ms: Vec<f64>,
    misses: u32,
}

impl LoopbackSeries {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one repetition (`None` = not detected).
    pub fn add(&mut self, r: Option<LoopbackResult>) {
        match r {
            Some(r) => self.delays_ms.push(r.delay_ms),
            None => self.misses += 1,
        }
    }

    pub fn detected(&self) -> usize {
        self.delays_ms.len()
    }

    pub fn misses(&self) -> u32 {
        self.misses
    }

    /// Round-trip p50/p95/p99 in ms.
    pub fn round_trip_ms(&self) -> Option<[f64; 3]> {
        crate::stats::p50_95_99(&self.delays_ms)
    }

    /// Per-leg estimate: half the median round trip (OS output + HFP down, HFP up + OS
    /// input), compared with the 25 ms/leg budget.
    pub fn per_leg_ms(&self) -> Option<f64> {
        self.round_trip_ms().map(|p| p[0] / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mls_is_maximal() {
        for order in 5..=12u32 {
            let m = mls(order);
            let len = (1usize << order) - 1;
            assert_eq!(m.len(), len);
            // Balance: 2^(n-1) ones.
            assert_eq!(
                m.iter().filter(|c| **c == 1).count(),
                1 << (order - 1),
                "order {order}"
            );
            // Two-valued periodic autocorrelation: −1 at every non-zero shift.
            for shift in [1usize, 2, 3, len / 2, len - 1] {
                let ac: i32 = (0..len)
                    .map(|i| i32::from(m[i]) * i32::from(m[(i + shift) % len]))
                    .sum();
                assert_eq!(ac, -1, "order {order} shift {shift}");
            }
        }
    }

    /// Deterministic noise in [-1, 1).
    fn noise(seed: &mut u64) -> f64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }

    fn capture_with(
        train: &ClickTrain,
        delay: usize,
        gain: f64,
        noise_amp: f64,
        total: usize,
    ) -> Vec<i16> {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        (0..total)
            .map(|i| {
                let echo = if i >= delay && i - delay < train.len() {
                    f64::from(train.samples()[i - delay]) * gain
                } else {
                    0.0
                };
                (echo + noise(&mut seed) * noise_amp)
                    .round()
                    .clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn finds_delayed_click_train_within_one_sample() {
        let train = ClickTrain::new(9, 4, 0.5);
        for (delay, gain) in [
            (0usize, 1.0),
            (1, 0.8),
            (1_234, 0.3),
            (2_791, 0.05),
            (3_999, 0.6),
        ] {
            let mut c = LoopbackCorrelator::new(train.clone(), 16_000, 250);
            let emit = 10_000u64;
            c.emit(emit);
            let cap = capture_with(&train, delay, gain, 300.0, 4_000 + train.len() + 100);
            // Feed in 160-sample blocks starting before the emit index.
            c.push_capture(emit - 320, &[0; 320]);
            for (k, block) in cap.chunks(160).enumerate() {
                c.push_capture(emit + (k * 160) as u64, block);
            }
            assert!(c.ready());
            let r = c.result().expect("detected");
            assert!(
                r.delay_samples.abs_diff(delay as u64) <= 1,
                "delay {delay} -> {}",
                r.delay_samples
            );
            assert!(((r.delay_ms * 16.0) - delay as f64).abs() <= 1.0);
            assert!((r.gain - gain).abs() < 0.05, "gain {gain} -> {}", r.gain);
            assert!((r.erl_db - (-20.0 * gain.log10())).abs() < 1.5);
            assert!(r.psr > 2.0, "psr {}", r.psr);
        }
    }

    #[test]
    fn silence_and_inverted_echo() {
        let train = ClickTrain::new(8, 2, 0.5);
        let mut c = LoopbackCorrelator::new(train.clone(), 16_000, 50);
        c.emit(0);
        assert!(!c.ready());
        assert!(c.result().is_none());
        c.push_capture(0, &vec![0; 2_000]);
        assert!(c.ready());
        assert!(c.result().is_none(), "silence is not a detection");
        let mut inv = LoopbackCorrelator::new(train.clone(), 16_000, 50);
        inv.emit(0);
        inv.push_capture(0, &capture_with(&train, 77, -0.5, 0.0, 2_000));
        let r = inv.result().unwrap();
        assert_eq!(r.delay_samples, 77);
        assert!(r.gain < 0.0);
    }

    #[test]
    fn series_per_leg() {
        let mut s = LoopbackSeries::new();
        for ms in [100.0, 102.0, 98.0, 101.0] {
            s.add(Some(LoopbackResult {
                delay_samples: 0,
                fraction: 0.0,
                delay_ms: ms,
                ncc: 1.0,
                gain: 1.0,
                erl_db: 0.0,
                psr: 10.0,
            }));
        }
        s.add(None);
        assert_eq!((s.detected(), s.misses()), (4, 1));
        assert_eq!(s.per_leg_ms(), Some(50.0));
        assert_eq!(ClickTrain::bench_default().len(), 1023 * 4);
    }
}
