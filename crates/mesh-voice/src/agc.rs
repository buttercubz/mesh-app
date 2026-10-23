//! Capture-side digital AGC (field test 2026-10-23: the Galaxy Tab S10 FE built-in mic
//! under Android's `VOICE_COMMUNICATION` preset delivers speech around -45..-40 dBFS
//! with no effective platform AGC, so riders had to shout).
//!
//! Slow, noise-floor-aware, boost-only gain stage between the HPF and the VAD:
//! - levels are measured on the input (pre-gain), so the gain never feeds back on itself;
//! - the speech level is learnt only from blocks well above the tracked noise floor, and
//!   the gain only moves on those blocks (it holds in pauses: no noise pumping);
//! - the gain never attenuates (0 dB minimum; loud input passes untouched) and never
//!   exceeds [`Agc::MAX_GAIN_DB`], nor lifts the noise floor above [`Agc::NOISE_CEIL_DBFS`];
//! - a per-block peak check keeps the boosted signal below [`Agc::PEAK_LIMIT`] (no clipping).
//!
//! On routes whose platform already runs AGC (iOS VPIO) the learnt speech level sits
//! near the target, so the stage stays at or near unity gain.

use crate::{BLOCK_SAMPLES, BLOCK_US, Block, rms_dbfs, sat_i16};

/// Boost-only automatic gain control on 10 ms capture blocks (deterministic, no allocation).
#[derive(Debug, Clone)]
pub struct Agc {
    enabled: bool,
    floor_db: Option<f32>,
    speech_db: Option<f32>,
    gain_db: f32,
    applied: f32,
}

impl Agc {
    /// Target RMS of active speech blocks after the gain.
    pub const TARGET_DBFS: f32 = -22.0;
    /// Maximum boost.
    pub const MAX_GAIN_DB: f32 = 24.0;
    /// The boosted noise floor may not exceed this level.
    pub const NOISE_CEIL_DBFS: f32 = -48.0;
    /// A block counts as speech when it is this far above the noise floor...
    pub const ACTIVE_MARGIN_DB: f32 = 9.0;
    /// ...and above this absolute level (digital silence never trains the AGC).
    pub const ACTIVE_MIN_DBFS: f32 = -72.0;
    /// Gain rise rate while speech is present (dB per second).
    pub const RISE_DB_PER_S: f32 = 12.0;
    /// Gain fall rate (dB per second) when the speech gets louder.
    pub const FALL_DB_PER_S: f32 = 40.0;
    /// Noise-floor rise rate (dB per second); the fall is instant.
    pub const FLOOR_RISE_DB_PER_S: f32 = 2.0;
    /// Highest absolute sample value the boost may produce (≈ -1 dBFS).
    pub const PEAK_LIMIT: f32 = 29_000.0;
    /// Smoothing of the speech-level estimate per active block (≈ 100 ms).
    const SPEECH_ALPHA: f32 = 0.1;

    pub fn new() -> Self {
        Self {
            enabled: true,
            floor_db: None,
            speech_db: None,
            gain_db: 0.0,
            applied: 1.0,
        }
    }

    /// Enables or bypasses the stage (bypass leaves blocks untouched and resets the gain).
    pub fn set_enabled(&mut self, on: bool) {
        if self.enabled != on {
            *self = Self::new();
            self.enabled = on;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Current smoothed gain in dB.
    pub fn gain_db(&self) -> f32 {
        if self.enabled { self.gain_db } else { 0.0 }
    }

    /// Input noise floor in dBFS (pre-gain).
    pub fn noise_floor_db(&self) -> f32 {
        self.floor_db.unwrap_or(-120.0)
    }

    /// Learns from the block (pre-gain) and applies the gain in place. Windy blocks
    /// (wind guard fired) never train the speech level or move the gain.
    pub fn process(&mut self, block: &mut Block, windy: bool) {
        if !self.enabled {
            return;
        }
        let dt = BLOCK_US as f32 / 1e6;
        let level = rms_dbfs(block);
        let prev_floor = self.floor_db.unwrap_or(level);
        self.floor_db = Some(if level >= prev_floor {
            (prev_floor + Self::FLOOR_RISE_DB_PER_S * dt).min(level)
        } else {
            level
        });
        let active = !windy
            && level >= Self::ACTIVE_MIN_DBFS
            && level >= prev_floor + Self::ACTIVE_MARGIN_DB;
        if active {
            let s = match self.speech_db {
                Some(s) => s + Self::SPEECH_ALPHA * (level - s),
                None => level,
            };
            self.speech_db = Some(s);
            let noise_cap = (Self::NOISE_CEIL_DBFS - prev_floor).max(0.0);
            let desired = (Self::TARGET_DBFS - s).clamp(0.0, Self::MAX_GAIN_DB.min(noise_cap));
            let step = if desired > self.gain_db {
                (Self::RISE_DB_PER_S * dt).min(desired - self.gain_db)
            } else {
                -(Self::FALL_DB_PER_S * dt).min(self.gain_db - desired)
            };
            self.gain_db += step;
        }
        let peak = block
            .iter()
            .map(|s| i32::from(*s).unsigned_abs())
            .max()
            .unwrap_or(0) as f32;
        let mut target = crate::from_dbfs(self.gain_db);
        if peak * target > Self::PEAK_LIMIT {
            target = (Self::PEAK_LIMIT / peak.max(1.0)).max(1.0);
            // A peak that needs less gain pulls the smoothed gain down with it.
            self.gain_db = self.gain_db.min(crate::to_dbfs(target));
        }
        let start = if peak * self.applied > Self::PEAK_LIMIT {
            target
        } else {
            self.applied
        };
        if start == 1.0 && target == 1.0 {
            self.applied = 1.0;
            return;
        }
        // Linear ramp across the block so gain changes never click.
        let step = (target - start) / BLOCK_SAMPLES as f32;
        for (i, s) in block.iter_mut().enumerate() {
            let g = start + step * (i + 1) as f32;
            *s = sat_i16(f32::from(*s) * g);
        }
        self.applied = target;
    }

    /// Forgets the learnt levels and returns to unity gain (route change, restart).
    pub fn reset(&mut self) {
        let on = self.enabled;
        *self = Self::new();
        self.enabled = on;
    }
}

impl Default for Agc {
    fn default() -> Self {
        Self::new()
    }
}
