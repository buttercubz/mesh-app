//! Capture-side filters (DESIGN §7.2-§7.3): 80 Hz high-pass and the wind guard.

use core::f32::consts::{FRAC_1_SQRT_2, PI};

use crate::{Block, SAMPLE_RATE, sat_i16};

/// Direct-form-I biquad (RBJ cookbook coefficients, normalised so `a0 = 1`).
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// 2nd-order Butterworth high-pass (Q = 1/√2).
    pub fn highpass(fc_hz: f32, fs_hz: f32) -> Self {
        Self::highpass_q(fc_hz, fs_hz, FRAC_1_SQRT_2)
    }

    /// High-pass with an explicit Q.
    pub fn highpass_q(fc_hz: f32, fs_hz: f32, q: f32) -> Self {
        let w = 2.0 * PI * fc_hz / fs_hz;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        Self::from_coeffs(
            (1.0 + c) / 2.0,
            -(1.0 + c),
            (1.0 + c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    /// 2nd-order Butterworth low-pass (Q = 1/√2).
    pub fn lowpass(fc_hz: f32, fs_hz: f32) -> Self {
        let w = 2.0 * PI * fc_hz / fs_hz;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * FRAC_1_SQRT_2);
        Self::from_coeffs(
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + alpha,
            -2.0 * c,
            1.0 - alpha,
        )
    }

    /// Filters one sample.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        // Flush denormals so a long silence never slows the RT path.
        self.y1 = if y.abs() < 1e-20 { 0.0 } else { y };
        y
    }

    /// Clears the filter history.
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Magnitude response at `f_hz` (for tests and diagnostics).
    pub fn magnitude_at(&self, f_hz: f32, fs_hz: f32) -> f32 {
        let w = 2.0 * PI * f_hz / fs_hz;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
    }
}

/// 80 Hz 2nd-order Butterworth high-pass at 16 kHz (wind rumble, §7.2).
#[derive(Debug, Clone, Copy)]
pub struct HighPass80 {
    bq: Biquad,
}

impl HighPass80 {
    pub const CUTOFF_HZ: f32 = 80.0;

    pub fn new() -> Self {
        Self {
            bq: Biquad::highpass(Self::CUTOFF_HZ, SAMPLE_RATE as f32),
        }
    }

    /// Filters a block in place.
    pub fn process(&mut self, block: &mut Block) {
        for s in block.iter_mut() {
            *s = sat_i16(self.bq.process(f32::from(*s)));
        }
    }

    pub fn reset(&mut self) {
        self.bq.reset();
    }

    /// The underlying biquad (for response tests).
    pub fn biquad(&self) -> &Biquad {
        &self.bq
    }
}

impl Default for HighPass80 {
    fn default() -> Self {
        Self::new()
    }
}

/// Wind guard (§7.3): on the pre-HPF signal, `E(<200 Hz) / E(300-3400 Hz) > 6 dB` marks
/// the block as windy, and the VAD then suppresses onset unless `p ≥ 0.9`.
#[derive(Debug, Clone, Copy)]
pub struct WindGuard {
    low: [Biquad; 2],
    band_hp: [Biquad; 2],
    band_lp: [Biquad; 2],
    ratio_db: f32,
}

impl WindGuard {
    /// Low/band energy ratio above which a block is windy.
    pub const THRESHOLD_DB: f32 = 6.0;

    pub fn new() -> Self {
        let fs = SAMPLE_RATE as f32;
        // 4th-order (two cascaded 2nd-order Butterworth sections) for each edge.
        Self {
            low: [Biquad::lowpass(200.0, fs); 2],
            band_hp: [Biquad::highpass(300.0, fs); 2],
            band_lp: [Biquad::lowpass(3400.0, fs); 2],
            ratio_db: f32::NEG_INFINITY,
        }
    }

    /// Analyses one raw (pre-HPF) block; returns true when it is windy.
    pub fn process(&mut self, raw: &Block) -> bool {
        let mut e_low = 1e-3f64;
        let mut e_band = 1e-3f64;
        for &s in raw.iter() {
            let x = f32::from(s);
            let l0 = self.low[0].process(x);
            let l = self.low[1].process(l0);
            let h0 = self.band_hp[0].process(x);
            let h1 = self.band_hp[1].process(h0);
            let b0 = self.band_lp[0].process(h1);
            let b = self.band_lp[1].process(b0);
            e_low += f64::from(l) * f64::from(l);
            e_band += f64::from(b) * f64::from(b);
        }
        self.ratio_db = (10.0 * (e_low / e_band).log10()) as f32;
        self.ratio_db > Self::THRESHOLD_DB
    }

    /// Ratio of the last block in dB.
    pub fn ratio_db(&self) -> f32 {
        self.ratio_db
    }

    pub fn reset(&mut self) {
        for b in self
            .low
            .iter_mut()
            .chain(self.band_hp.iter_mut())
            .chain(self.band_lp.iter_mut())
        {
            b.reset();
        }
        self.ratio_db = f32::NEG_INFINITY;
    }
}

impl Default for WindGuard {
    fn default() -> Self {
        Self::new()
    }
}
