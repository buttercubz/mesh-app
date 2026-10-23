//! Syllable-onset detection for the modeled mouth-to-ear latency (DESIGN §13.2).
//!
//! The scripted talkers speak the deterministic speech-like signal of [`crate::synth`]:
//! syllables with a 20 ms attack separated by pauses. The same detector runs on every
//! talker's capture stream ("mouth") and every listener's playout stream ("ear"); a
//! syllable onset is the instant the 10 ms RMS envelope crosses half of the syllable's
//! peak envelope after at least 15 ms of quiet, linearly interpolated between 2.5 ms
//! hops. The threshold is relative to each side's own peak, so codec and mixer gain do
//! not move it, and the envelope spans at least one pitch period, so waveform phase
//! does not either. Mouth-to-ear for a syllable is `ear onset − mouth onset`; the
//! constant delay of the envelope itself cancels.
//!
//! Time conventions (µs): a capture block pushed at `t` holds samples captured in
//! `[t − 10 ms, t)` (the 10 ms OS capture IO); a playout block pulled at `t` reaches the
//! ear from `t + RENDER_BUFFER_US` (the render ring target), as the in-app estimator
//! counts it (§12.6). Headset legs are excluded, as in every §13.2 threshold.

use mesh_core::BLOCK_SAMPLES;

/// Samples per envelope hop (2.5 ms at 16 kHz).
const HOP: usize = 40;
/// Envelope window (10 ms).
const WIN: usize = 160;
/// Envelope below this is quiet (≈ −47 dBFS RMS).
const QUIET_RMS: f64 = 150.0;
/// Quiet hops needed before an onset (15 ms).
const QUIET_HOPS: u32 = 6;
/// Hops collected after the rise to find the syllable's peak (50 ms).
const PEAK_HOPS: usize = 20;
/// A syllable's peak envelope must reach this multiple of the quiet level.
const MIN_PEAK_FACTOR: f64 = 4.0;
/// Sample period in µs.
pub const SAMPLE_US: f64 = 62.5;
/// Capture block span: samples of a block pushed at `t` start at `t − 10 ms`.
pub const CAPTURE_IO_US: f64 = 10_000.0;

/// A streaming onset detector.
#[derive(Debug, Clone)]
pub struct OnsetDetector {
    ring: Box<[i16; WIN]>,
    pos: usize,
    filled: usize,
    sumsq: f64,
    hop_ctr: usize,
    quiet_hops: u32,
    prev: (f64, f64),
    cand: Vec<(f64, f64)>,
    /// Detected onsets (µs, rounded).
    pub onsets: Vec<u64>,
}

impl Default for OnsetDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl OnsetDetector {
    pub fn new() -> Self {
        OnsetDetector {
            ring: Box::new([0; WIN]),
            pos: 0,
            filled: 0,
            sumsq: 0.0,
            hop_ctr: 0,
            quiet_hops: 0,
            prev: (0.0, 0.0),
            cand: Vec::with_capacity(PEAK_HOPS),
            onsets: Vec::new(),
        }
    }

    /// Feeds one 10 ms block whose first sample is at `t0_us`.
    pub fn push_block(&mut self, t0_us: f64, block: &[i16; BLOCK_SAMPLES]) {
        for (j, &s) in block.iter().enumerate() {
            self.push(t0_us + j as f64 * SAMPLE_US, s);
        }
    }

    fn push(&mut self, t_us: f64, s: i16) {
        let old = f64::from(self.ring[self.pos]);
        let v = f64::from(s);
        self.sumsq += v * v - old * old;
        self.ring[self.pos] = s;
        self.pos = (self.pos + 1) % WIN;
        self.filled = (self.filled + 1).min(WIN);
        self.hop_ctr += 1;
        if self.hop_ctr < HOP {
            return;
        }
        self.hop_ctr = 0;
        if self.sumsq < 0.0 {
            // Floating drift: recompute exactly.
            self.sumsq = self.ring.iter().map(|x| f64::from(*x).powi(2)).sum();
        }
        let env = (self.sumsq.max(0.0) / WIN as f64).sqrt();
        self.on_hop(t_us, env);
    }

    fn on_hop(&mut self, t: f64, env: f64) {
        if !self.cand.is_empty() {
            self.cand.push((t, env));
            if self.cand.len() >= PEAK_HOPS {
                self.finish();
            }
        } else if env >= QUIET_RMS && self.quiet_hops >= QUIET_HOPS {
            self.cand.push(self.prev);
            self.cand.push((t, env));
        }
        if env < QUIET_RMS {
            self.quiet_hops = self.quiet_hops.saturating_add(1);
        } else {
            self.quiet_hops = 0;
        }
        self.prev = (t, env);
    }

    fn finish(&mut self) {
        let peak = self.cand.iter().map(|c| c.1).fold(0.0, f64::max);
        if peak >= QUIET_RMS * MIN_PEAK_FACTOR {
            let thr = peak * 0.5;
            if let Some(k) = self.cand.iter().position(|c| c.1 >= thr) {
                let (t1, e1) = self.cand[k];
                let t = if k == 0 {
                    t1
                } else {
                    let (t0, e0) = self.cand[k - 1];
                    t0 + (t1 - t0) * ((thr - e0) / (e1 - e0).max(1e-9))
                };
                self.onsets.push(t.max(0.0).round() as u64);
            }
        }
        self.cand.clear();
    }

    /// Drops every detected onset (aggregation by the scenario).
    pub fn take(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.onsets)
    }
}

/// Matches mouth onsets of one talk span to a listener's ear onsets and returns the
/// mouth-to-ear delay of each matched syllable (µs).
///
/// The span's delay is acquired first, from the whole onset trains: the delay `d` in
/// `[min_us, max_us]` that aligns the most mouth onsets with an ear onset within
/// `ACQUIRE_TOL_US` (the earliest such delay on ties). Syllable lengths of the
/// speech-like signal vary, so a train shifted by a syllable aligns poorly, and a
/// stray ear onset (an earcon, another talker) cannot capture the match the way a
/// first-onset rule can. A span needs at least two aligned syllables. Each syllable is
/// then matched to the ear onset nearest to `m + d_prev` within `± tol_us`, `d_prev`
/// being the last matched delay (the jitter buffer may move it), falling back to the
/// acquired delay after a syllable that matched nothing.
pub fn match_onsets(mouth: &[u64], ear: &[u64], min_us: u64, max_us: u64, tol_us: u64) -> Vec<u64> {
    let Some(d0) = acquire(mouth, ear, min_us, max_us) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut d_prev = d0;
    let mut lo = 0usize;
    for &m in mouth {
        let target = m + d_prev;
        let pick = ear[lo..]
            .iter()
            .enumerate()
            .filter(|(_, e)| e.abs_diff(target) <= tol_us && **e >= m + min_us && **e <= m + max_us)
            .min_by_key(|(_, e)| e.abs_diff(target))
            .map(|(p, _)| lo + p);
        match pick {
            Some(j) => {
                let d = ear[j] - m;
                out.push(d);
                d_prev = d;
                lo = j + 1;
            }
            // A concealed or lost syllable: the next one is matched at the span delay.
            None => d_prev = d0,
        }
    }
    out
}

/// Tolerance of the span-delay acquisition.
const ACQUIRE_TOL_US: u64 = 20_000;

/// The delay aligning the most onsets (see [`match_onsets`]); `None` below 2.
fn acquire(mouth: &[u64], ear: &[u64], min_us: u64, max_us: u64) -> Option<u64> {
    let score = |d: u64| {
        mouth
            .iter()
            .filter(|&&m| {
                let t = m + d;
                let k = ear.partition_point(|&e| e + ACQUIRE_TOL_US < t);
                ear.get(k).is_some_and(|e| e.abs_diff(t) <= ACQUIRE_TOL_US)
            })
            .count()
    };
    let mut best: Option<(usize, u64)> = None;
    for &m in mouth {
        let lo = ear.partition_point(|&e| e < m + min_us);
        for &e in ear[lo..].iter().take_while(|&&e| e <= m + max_us) {
            let d = e - m;
            let s = score(d);
            let better = match best {
                None => true,
                Some((bs, bd)) => s > bs || (s == bs && d < bd),
            };
            if better {
                best = Some((s, d));
            }
        }
    }
    best.filter(|(s, _)| *s >= 2).map(|(_, d)| d)
}

/// The `p`-th percentile (0..=100, nearest rank) of `v`.
pub fn percentile(v: &[u64], p: f64) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    let rank = ((p / 100.0) * s.len() as f64).ceil() as usize;
    Some(s[rank.clamp(1, s.len()) - 1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::SpeechSynth;

    fn detect(samples: &[i16], gain: f64, t0: f64) -> Vec<u64> {
        let mut d = OnsetDetector::new();
        for (k, c) in samples.chunks(BLOCK_SAMPLES).enumerate() {
            let mut b = [0i16; BLOCK_SAMPLES];
            for (o, s) in b.iter_mut().zip(c) {
                *o = (f64::from(*s) * gain) as i16;
            }
            d.push_block(t0 + k as f64 * 10_000.0, &b);
        }
        d.onsets
    }

    #[test]
    fn delay_is_recovered_independently_of_gain() {
        let mut syn = SpeechSynth::new(5, 1);
        let mut sig = vec![0i16; 3_200];
        sig.extend(syn.render_ms(6_000));
        let mouth = detect(&sig, 1.0, 0.0);
        assert!(mouth.len() >= 15, "{} syllables", mouth.len());
        // The same signal 173.4 ms later, 7 dB quieter.
        let ear = detect(&sig, 0.45, 173_400.0);
        let d = match_onsets(&mouth, &ear, 20_000, 600_000, 60_000);
        assert_eq!(d.len(), mouth.len());
        for x in d {
            assert!(x.abs_diff(173_400) <= 300, "{x}");
        }
        // A stray ear onset (an earcon) before the first syllable and the first two
        // syllables lost: the span delay is still the one the train agrees on.
        let mut ear2: Vec<u64> = ear[2..].to_vec();
        ear2.insert(0, mouth[0] + 40_000);
        let d = match_onsets(&mouth, &ear2, 20_000, 700_000, 60_000);
        assert_eq!(d.len(), mouth.len() - 2);
        assert!(d.iter().all(|x| x.abs_diff(173_400) <= 300), "{d:?}");
        // Unrelated onsets alone give no samples.
        assert!(match_onsets(&mouth, &[mouth[3] + 50_000], 20_000, 700_000, 60_000).is_empty());
        assert_eq!(percentile(&[5, 1, 9, 3], 50.0), Some(3));
        assert_eq!(percentile(&[5, 1, 9, 3], 95.0), Some(9));
    }
}
