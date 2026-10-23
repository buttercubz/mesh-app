//! Seeded randomness of the simulator (DESIGN §3.1, §13.2): one `ChaCha20Rng` per
//! purpose, every one derived from the scenario seed, plus stateless hashing for
//! values that are looked up lazily (per-pair shadowing windows).
//!
//! Nothing here touches OS randomness or clocks, so a seed fully determines a run.

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

/// SplitMix64 step: a fast, well-mixed bijection on `u64`.
pub const fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Mixes several words into one (order-sensitive).
pub fn mix(words: &[u64]) -> u64 {
    words.iter().fold(0x6A09_E667_F3BC_C908, |acc, w| {
        splitmix64(acc ^ splitmix64(*w))
    })
}

/// A uniform `f64` in `[0, 1)` from a hash value (53 random bits).
pub fn unit_f64(h: u64) -> f64 {
    (h >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// A standard normal value from two hash values (Box-Muller).
pub fn normal_from(h1: u64, h2: u64) -> f64 {
    let u1 = unit_f64(h1).max(f64::MIN_POSITIVE);
    let u2 = unit_f64(h2);
    (-2.0 * u1.ln()).sqrt() * (core::f64::consts::TAU * u2).cos()
}

/// 32-byte ChaCha seed derived from a scenario seed and a purpose tag.
pub fn derive_seed(seed: u64, tag: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut z = mix(&[seed, tag]);
    for chunk in out.chunks_mut(8) {
        z = splitmix64(z);
        chunk.copy_from_slice(&z.to_le_bytes());
    }
    out
}

/// A seeded random stream with the distributions the models need.
#[derive(Debug, Clone)]
pub struct SimRng {
    inner: ChaCha20Rng,
}

impl SimRng {
    /// The stream for `tag` of scenario seed `seed`.
    pub fn new(seed: u64, tag: u64) -> Self {
        SimRng {
            inner: ChaCha20Rng::from_seed(derive_seed(seed, tag)),
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    pub fn next_u32(&mut self) -> u32 {
        self.inner.next_u32()
    }

    /// Fills `buf` with random bytes.
    pub fn fill(&mut self, buf: &mut [u8]) {
        self.inner.fill_bytes(buf);
    }

    /// Uniform in `[0, 1)`.
    pub fn f64(&mut self) -> f64 {
        unit_f64(self.next_u64())
    }

    /// Uniform in `[lo, hi)`.
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }

    /// Uniform integer in `[lo, hi]` (inclusive; `lo` when `hi < lo`).
    pub fn range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + self.next_u64() % (hi - lo + 1)
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    /// Exponential with the given mean.
    pub fn exp(&mut self, mean: f64) -> f64 {
        let u = self.f64();
        -mean * (1.0 - u).max(f64::MIN_POSITIVE).ln()
    }

    /// Standard normal.
    pub fn normal(&mut self) -> f64 {
        let a = self.next_u64();
        let b = self.next_u64();
        normal_from(a, b)
    }

    /// Log-normal with the given median and log-space sigma.
    pub fn lognormal(&mut self, median: f64, sigma: f64) -> f64 {
        median * (sigma * self.normal()).exp()
    }

    /// Number of failures before the first success of a Bernoulli(`p`) trial, capped
    /// at `cap` (returned when `p` is ~0 or the draw exceeds it).
    pub fn geometric_failures(&mut self, p: f64, cap: u32) -> u32 {
        if p >= 1.0 {
            return 0;
        }
        if p <= 1e-9 {
            return cap;
        }
        let u = (1.0 - self.f64()).max(f64::MIN_POSITIVE);
        let k = (u.ln() / (1.0 - p).ln()).floor();
        if k.is_finite() && k < f64::from(cap) {
            k as u32
        } else {
            cap
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_reproducible_and_distinct() {
        let mut a = SimRng::new(7, 1);
        let mut b = SimRng::new(7, 1);
        let mut c = SimRng::new(7, 2);
        let va: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let vb: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        let vc: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(va, vb);
        assert_ne!(va, vc);
    }

    #[test]
    fn distributions_have_the_right_moments() {
        let mut r = SimRng::new(42, 0);
        let n = 20_000;
        let m: f64 = (0..n).map(|_| r.exp(1.5)).sum::<f64>() / f64::from(n);
        assert!((m - 1.5).abs() < 0.06, "exp mean {m}");
        let xs: Vec<f64> = (0..n).map(|_| r.normal()).collect();
        let mean = xs.iter().sum::<f64>() / f64::from(n);
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / f64::from(n);
        assert!(
            mean.abs() < 0.03 && (var - 1.0).abs() < 0.05,
            "{mean} {var}"
        );
        // Geometric failures with p = 0.8: mean (1-p)/p = 0.25.
        let g: f64 = (0..n)
            .map(|_| f64::from(r.geometric_failures(0.8, 100)))
            .sum::<f64>()
            / f64::from(n);
        assert!((g - 0.25).abs() < 0.03, "geometric mean {g}");
        assert_eq!(r.geometric_failures(0.0, 9), 9);
        assert_eq!(r.geometric_failures(1.0, 9), 0);
        let mut ls: Vec<f64> = (0..n).map(|_| r.lognormal(40.0, 0.8)).collect();
        ls.sort_by(f64::total_cmp);
        let med = ls[ls.len() / 2];
        assert!((med - 40.0).abs() < 2.0, "lognormal median {med}");
    }

    #[test]
    fn hashing_is_stable() {
        assert_eq!(mix(&[1, 2]), mix(&[1, 2]));
        assert_ne!(mix(&[1, 2]), mix(&[2, 1]));
        let u = unit_f64(u64::MAX);
        assert!((0.0..1.0).contains(&u));
    }
}
