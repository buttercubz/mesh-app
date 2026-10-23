//! Mixer and limiter (DESIGN §7.6).
//!
//! The mixer sums ≤ 4 decoded streams (chosen by floor rank upstream), each attenuated
//! −3 dB when ≥ 2 are active. Earcons and prompts are summed **before** the limiter so they
//! are part of the platform AEC reference. The limiter is feed-forward: threshold −3 dBFS,
//! attack 1 ms, release 50 ms, no look-ahead (a final hard clip bounds the 1 ms attack).

use crate::{BLOCK_SAMPLES, Block, SAMPLE_RATE, from_dbfs, sat_i16};

/// Streams mixed at most (§6.3, §7.6).
pub const MAX_MIX_STREAMS: usize = 4;
/// Per-stream gain when two or more streams are active (−3 dB).
pub const MULTI_STREAM_GAIN_DB: f32 = -3.0;

/// Feed-forward peak limiter on normalised samples (full scale 1.0).
#[derive(Debug, Clone, Copy)]
pub struct Limiter {
    threshold: f32,
    attack: f32,
    release: f32,
    env: f32,
}

impl Limiter {
    pub const THRESHOLD_DBFS: f32 = -3.0;
    pub const ATTACK_MS: f32 = 1.0;
    pub const RELEASE_MS: f32 = 50.0;

    pub fn new() -> Self {
        Self::with_params(Self::THRESHOLD_DBFS, Self::ATTACK_MS, Self::RELEASE_MS)
    }

    pub fn with_params(threshold_dbfs: f32, attack_ms: f32, release_ms: f32) -> Self {
        let coef = |ms: f32| (-1.0 / (ms * 1e-3 * SAMPLE_RATE as f32)).exp();
        Self {
            threshold: from_dbfs(threshold_dbfs),
            attack: coef(attack_ms),
            release: coef(release_ms),
            env: 0.0,
        }
    }

    /// Limits one sample (normalised) and returns the result in `[-1, 1]`.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let a = x.abs();
        let c = if a > self.env {
            self.attack
        } else {
            self.release
        };
        self.env = c * self.env + (1.0 - c) * a;
        let g = if self.env > self.threshold {
            self.threshold / self.env
        } else {
            1.0
        };
        (x * g).clamp(-1.0, 1.0)
    }

    /// Current gain reduction in dB (≤ 0).
    pub fn gain_db(&self) -> f32 {
        if self.env > self.threshold {
            20.0 * (self.threshold / self.env).log10()
        } else {
            0.0
        }
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Playout mixer.
#[derive(Debug, Clone)]
pub struct Mixer {
    limiter: Limiter,
    acc: [f32; BLOCK_SAMPLES],
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            limiter: Limiter::new(),
            acc: [0.0; BLOCK_SAMPLES],
        }
    }

    /// Mixes up to [`MAX_MIX_STREAMS`] voice streams (extra ones are ignored: the caller
    /// passes them in rank order) plus `cues` (earcons, prompts, audio-check signals) at
    /// unity, then limits into `out`.
    pub fn mix(&mut self, streams: &[&Block], cues: &[&Block], out: &mut Block) {
        self.acc = [0.0; BLOCK_SAMPLES];
        let streams = &streams[..streams.len().min(MAX_MIX_STREAMS)];
        let g = if streams.len() >= 2 {
            from_dbfs(MULTI_STREAM_GAIN_DB)
        } else {
            1.0
        };
        for s in streams {
            for (a, &x) in self.acc.iter_mut().zip(s.iter()) {
                *a += g * f32::from(x);
            }
        }
        for c in cues {
            for (a, &x) in self.acc.iter_mut().zip(c.iter()) {
                *a += f32::from(x);
            }
        }
        for (o, &a) in out.iter_mut().zip(self.acc.iter()) {
            *o = sat_i16(self.limiter.process(a / 32768.0) * 32767.0);
        }
    }

    pub fn limiter(&self) -> &Limiter {
        &self.limiter
    }

    pub fn reset(&mut self) {
        self.limiter.reset();
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}
