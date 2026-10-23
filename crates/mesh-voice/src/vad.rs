//! VAD / VOX (DESIGN §7.3).
//!
//! * [`Vad`] yields a per-block voice probability `p`. [`RnnVad`] is the v1 detector:
//!   `nnnoiseless` (pure-Rust RNNoise) used only for its probability (the denoised output
//!   is discarded), fed by a 48-tap polyphase ×3 upsampler (16 → 48 kHz).
//!   [`EnergyVad`] is a deterministic energy detector for the simulator and gate tests.
//! * [`VadGate`] turns `p` into VOX decisions: onset when `p ≥ θ_on` for 2 consecutive
//!   blocks and the block RMS is ≥ noise floor + margin (`p ≥ 0.9` when the wind guard
//!   fires); release when `p < θ_off` for the hangover. This 300 ms hangover is the only
//!   one in the system: the talk FSM releases on the gate's `Off` at once.

use nnnoiseless::DenoiseState;

use crate::{BLOCK_SAMPLES, BLOCK_US, Block};

/// A voice-activity detector over 10 ms blocks.
pub trait Vad: Send {
    /// Voice probability of `block` in `[0, 1]`.
    fn process(&mut self, block: &Block) -> f32;
    /// Clears the detector history.
    fn reset(&mut self);
}

/// Number of taps of the ×3 interpolation filter.
pub const UPSAMPLE_TAPS: usize = 48;
const PHASES: usize = 3;
const PHASE_TAPS: usize = UPSAMPLE_TAPS / PHASES;
const FRAME_48K: usize = BLOCK_SAMPLES * PHASES;

/// 48-tap polyphase interpolator, 16 kHz → 48 kHz (Blackman-windowed sinc, 7.6 kHz cutoff).
#[derive(Debug, Clone)]
pub struct Upsampler3 {
    /// `phase[k][j] = 3 · h[3j + k]` (the ×3 restores the zero-stuffing gain).
    phase: [[f32; PHASE_TAPS]; PHASES],
    hist: [f32; PHASE_TAPS],
}

impl Upsampler3 {
    pub fn new() -> Self {
        let mut h = [0f32; UPSAMPLE_TAPS];
        let fc = 7600.0 / 48_000.0;
        let mid = (UPSAMPLE_TAPS - 1) as f32 / 2.0;
        let mut sum = 0.0;
        for (i, t) in h.iter_mut().enumerate() {
            let n = i as f32 - mid;
            let x = 2.0 * core::f32::consts::PI * fc * n;
            let sinc = if n.abs() < 1e-6 {
                2.0 * fc
            } else {
                x.sin() / (core::f32::consts::PI * n)
            };
            let a = 2.0 * core::f32::consts::PI * i as f32 / (UPSAMPLE_TAPS - 1) as f32;
            let w = 0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos();
            *t = sinc * w;
            sum += *t;
        }
        let mut phase = [[0f32; PHASE_TAPS]; PHASES];
        for (k, ph) in phase.iter_mut().enumerate() {
            for (j, c) in ph.iter_mut().enumerate() {
                *c = PHASES as f32 * h[PHASES * j + k] / sum;
            }
        }
        Self {
            phase,
            hist: [0.0; PHASE_TAPS],
        }
    }

    /// Upsamples one 160-sample block into 480 samples (input scale is kept).
    pub fn process(&mut self, block: &Block, out: &mut [f32; FRAME_48K]) {
        for (n, &s) in block.iter().enumerate() {
            self.hist.copy_within(0..PHASE_TAPS - 1, 1);
            self.hist[0] = f32::from(s);
            for k in 0..PHASES {
                let mut acc = 0.0;
                for j in 0..PHASE_TAPS {
                    acc += self.phase[k][j] * self.hist[j];
                }
                out[PHASES * n + k] = acc;
            }
        }
    }

    pub fn reset(&mut self) {
        self.hist = [0.0; PHASE_TAPS];
    }
}

impl Default for Upsampler3 {
    fn default() -> Self {
        Self::new()
    }
}

/// RNNoise-based VAD (the v1 detector).
pub struct RnnVad {
    state: Box<DenoiseState<'static>>,
    up: Upsampler3,
    frame: [f32; FRAME_48K],
    scratch: [f32; FRAME_48K],
}

impl RnnVad {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            up: Upsampler3::new(),
            frame: [0.0; FRAME_48K],
            scratch: [0.0; FRAME_48K],
        }
    }
}

impl Default for RnnVad {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for RnnVad {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RnnVad")
    }
}

impl Vad for RnnVad {
    fn process(&mut self, block: &Block) -> f32 {
        self.up.process(block, &mut self.frame);
        // `nnnoiseless` expects i16-scale floats; the denoised output is discarded.
        self.state
            .process_frame(&mut self.scratch, &self.frame)
            .clamp(0.0, 1.0)
    }

    fn reset(&mut self) {
        *self.state = *DenoiseState::new();
        self.up.reset();
    }
}

/// Deterministic energy detector: `p` is a logistic of the block level above a tracked
/// floor (≈ 0.5 at floor + 12 dB). Used by the simulator and by gate/FSM tests.
#[derive(Debug, Clone)]
pub struct EnergyVad {
    floor: Option<f32>,
}

impl EnergyVad {
    pub fn new() -> Self {
        Self { floor: None }
    }
}

impl Default for EnergyVad {
    fn default() -> Self {
        Self::new()
    }
}

impl Vad for EnergyVad {
    fn process(&mut self, block: &Block) -> f32 {
        let db = crate::rms_dbfs(block).max(-90.0);
        let floor = match self.floor {
            Some(f) if db >= f => (f + 0.02).min(db),
            _ => db,
        };
        self.floor = Some(floor);
        let x = (db - floor.max(-75.0) - 12.0) / 2.0;
        1.0 / (1.0 + (-x).exp())
    }

    fn reset(&mut self) {
        self.floor = None;
    }
}

/// VOX sensitivity presets (§7.3 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum VadSensitivity {
    High,
    #[default]
    Medium,
    Low,
}

impl VadSensitivity {
    /// Onset probability threshold.
    pub const fn theta_on(self) -> f32 {
        match self {
            VadSensitivity::High => 0.50,
            VadSensitivity::Medium => 0.65,
            VadSensitivity::Low => 0.80,
        }
    }

    /// Release probability threshold.
    pub const fn theta_off(self) -> f32 {
        match self {
            VadSensitivity::High => 0.30,
            VadSensitivity::Medium => 0.40,
            VadSensitivity::Low => 0.50,
        }
    }

    /// Required level above the noise floor for onset (dB).
    pub const fn margin_db(self) -> f32 {
        match self {
            VadSensitivity::High => 4.0,
            VadSensitivity::Medium => 6.0,
            VadSensitivity::Low => 9.0,
        }
    }

    /// Release hangover (the single 300 ms hangover of the system).
    pub const fn hangover_us(self) -> u64 {
        300_000
    }

    /// One level less sensitive (wind-hog protection, §6.5); `Low` stays `Low`.
    pub const fn raised(self) -> Self {
        match self {
            VadSensitivity::High => VadSensitivity::Medium,
            _ => VadSensitivity::Low,
        }
    }
}

/// Edge produced by [`VadGate::update`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    /// Speech onset (`vad_on`).
    On,
    /// Speech released after the hangover (`vad_off`).
    Off,
}

/// Onset probability required while the wind guard fires.
pub const WIND_ONSET_P: f32 = 0.9;
/// Noise-floor rise rate (dB per second); the fall is instant.
pub const FLOOR_RISE_DB_PER_S: f32 = 3.0;
const ONSET_BLOCKS: u8 = 2;

/// VOX decision logic on top of a [`Vad`] probability.
#[derive(Debug, Clone)]
pub struct VadGate {
    sens: VadSensitivity,
    floor_db: Option<f32>,
    onset_run: u8,
    active: bool,
    below_off_us: u64,
    last_p: f32,
}

impl VadGate {
    pub fn new(sens: VadSensitivity) -> Self {
        Self {
            sens,
            floor_db: None,
            onset_run: 0,
            active: false,
            below_off_us: 0,
            last_p: 0.0,
        }
    }

    pub fn sensitivity(&self) -> VadSensitivity {
        self.sens
    }

    /// Changes the preset (takes effect on the next block).
    pub fn set_sensitivity(&mut self, sens: VadSensitivity) {
        self.sens = sens;
    }

    /// True while VOX considers the rider to be speaking (between `On` and `Off`).
    pub fn is_open(&self) -> bool {
        self.active
    }

    /// Tracked noise floor in dBFS.
    pub fn noise_floor_db(&self) -> f32 {
        self.floor_db.unwrap_or(-120.0)
    }

    pub fn last_p(&self) -> f32 {
        self.last_p
    }

    /// Feeds one block's probability, its (post-HPF) RMS in dBFS and the wind flag.
    pub fn update(&mut self, p: f32, rms_db: f32, windy: bool) -> Option<VadEvent> {
        self.last_p = p;
        let rise = FLOOR_RISE_DB_PER_S * (BLOCK_US as f32 / 1e6);
        // Onset compares against the floor as it stood before this block.
        let prev_floor = self.floor_db.unwrap_or(rms_db);
        self.floor_db = Some(if rms_db >= prev_floor {
            (prev_floor + rise).min(rms_db)
        } else {
            rms_db
        });
        if self.active {
            if p < self.sens.theta_off() {
                self.below_off_us += BLOCK_US;
                if self.below_off_us >= self.sens.hangover_us() {
                    self.active = false;
                    self.below_off_us = 0;
                    self.onset_run = 0;
                    return Some(VadEvent::Off);
                }
            } else {
                self.below_off_us = 0;
            }
            return None;
        }
        let theta = if windy {
            self.sens.theta_on().max(WIND_ONSET_P)
        } else {
            self.sens.theta_on()
        };
        if p >= theta && rms_db >= prev_floor + self.sens.margin_db() {
            self.onset_run += 1;
            if self.onset_run >= ONSET_BLOCKS {
                self.active = true;
                self.below_off_us = 0;
                return Some(VadEvent::On);
            }
        } else {
            self.onset_run = 0;
        }
        None
    }

    /// Forgets all state (route change, new session).
    pub fn reset(&mut self) {
        *self = Self::new(self.sens);
    }
}
