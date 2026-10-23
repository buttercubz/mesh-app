//! Audio check (`audioCheck{start, mode}`, DESIGN §1.3 F0, §3.7, §7.3): headset tone
//! check, 1 s echo test and the VOX meter. Each emits `audioCheckSample` data at 20 Hz.
//!
//! The caller runs its [`crate::CaptureChain`] on every captured block first and passes the
//! filtered block plus the [`CaptureFrame`]; the check writes what to play into `out`.

use crate::capture::CaptureFrame;
use crate::tone::ToneGenerator;
use crate::{BLOCK_SAMPLES, Block, SAMPLE_RATE, to_dbfs};

/// Blocks between two samples (20 Hz).
pub const SAMPLE_EVERY_BLOCKS: u32 = 5;
/// Tone phase length.
pub const TONE_US: u64 = 1_000_000;
/// Record phase length (§1.3: "records 2 s").
pub const RECORD_US: u64 = 2_000_000;
/// Echo delay (§1.1: "hear yourself after 1 s").
pub const ECHO_DELAY_SAMPLES: usize = SAMPLE_RATE as usize;
/// Check tone frequency and level.
pub const TONE_HZ: f32 = 660.0;
pub const TONE_LEVEL_DBFS: f32 = -12.0;

/// Mode requested by the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCheckMode {
    Tone,
    Echo,
    VoxMeter,
}

/// `audioCheckSample.phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCheckPhase {
    Tone,
    Record,
    Echo,
    Meter,
}

/// One `audioCheckSample` (the route fields are added by the engine).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioCheckSample {
    /// Input level, linear 0..1 (max block RMS over the last 50 ms).
    pub level: f32,
    pub level_dbfs: f32,
    /// Max VAD probability over the last 50 ms.
    pub vad_p: f32,
    pub gate_open: bool,
    pub phase: AudioCheckPhase,
}

/// Accumulates 50 ms of capture statistics.
#[derive(Debug, Clone, Copy, Default)]
struct Meter {
    blocks: u32,
    level: f32,
    p: f32,
}

impl Meter {
    fn feed(&mut self, frame: &CaptureFrame, phase: AudioCheckPhase) -> Option<AudioCheckSample> {
        self.level = self.level.max(frame.level);
        self.p = self.p.max(frame.vad_p);
        self.blocks += 1;
        if self.blocks < SAMPLE_EVERY_BLOCKS {
            return None;
        }
        let s = AudioCheckSample {
            level: self.level.clamp(0.0, 1.0),
            level_dbfs: to_dbfs(self.level),
            vad_p: self.p,
            gate_open: frame.gate_open,
            phase,
        };
        *self = Meter::default();
        Some(s)
    }
}

/// Plays a tone for 1 s, then records (meters) for 2 s, then finishes.
#[derive(Debug, Clone)]
pub struct Tone {
    osc: ToneGenerator,
    elapsed_us: u64,
    meter: Meter,
}

impl Tone {
    pub fn new() -> Self {
        Self {
            osc: ToneGenerator::new(TONE_HZ, TONE_LEVEL_DBFS),
            elapsed_us: 0,
            meter: Meter::default(),
        }
    }

    pub fn phase(&self) -> Option<AudioCheckPhase> {
        if self.elapsed_us < TONE_US {
            Some(AudioCheckPhase::Tone)
        } else if self.elapsed_us < TONE_US + RECORD_US {
            Some(AudioCheckPhase::Record)
        } else {
            None
        }
    }

    pub fn process(
        &mut self,
        _captured: &Block,
        frame: &CaptureFrame,
        out: &mut Block,
    ) -> Option<AudioCheckSample> {
        out.fill(0);
        let phase = self.phase()?;
        if phase == AudioCheckPhase::Tone {
            self.osc.fill(out);
        }
        self.elapsed_us += crate::BLOCK_US;
        self.meter.feed(frame, phase)
    }
}

impl Default for Tone {
    fn default() -> Self {
        Self::new()
    }
}

/// Plays the capture back 1 s later (echo test).
#[derive(Clone)]
pub struct Echo {
    line: Box<[i16]>,
    pos: usize,
    meter: Meter,
}

impl Echo {
    pub fn new() -> Self {
        Self {
            line: vec![0; ECHO_DELAY_SAMPLES].into_boxed_slice(),
            pos: 0,
            meter: Meter::default(),
        }
    }

    pub fn process(
        &mut self,
        captured: &Block,
        frame: &CaptureFrame,
        out: &mut Block,
    ) -> Option<AudioCheckSample> {
        for (o, &c) in out.iter_mut().zip(captured.iter()) {
            *o = self.line[self.pos];
            self.line[self.pos] = c;
            self.pos = (self.pos + 1) % self.line.len();
        }
        self.meter.feed(frame, AudioCheckPhase::Echo)
    }
}

impl Default for Echo {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for Echo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Echo")
            .field("delay_samples", &self.line.len())
            .finish()
    }
}

/// VOX test meter: live `p` and gate state, silent output.
#[derive(Debug, Clone, Default)]
pub struct VoxMeter {
    meter: Meter,
}

impl VoxMeter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(
        &mut self,
        _captured: &Block,
        frame: &CaptureFrame,
        out: &mut Block,
    ) -> Option<AudioCheckSample> {
        out.fill(0);
        self.meter.feed(frame, AudioCheckPhase::Meter)
    }
}

/// A running audio check.
#[derive(Debug, Clone)]
pub enum AudioCheck {
    Tone(Tone),
    Echo(Echo),
    VoxMeter(VoxMeter),
}

impl AudioCheck {
    /// Starts a check (control path: the echo line is allocated here).
    pub fn new(mode: AudioCheckMode) -> Self {
        match mode {
            AudioCheckMode::Tone => AudioCheck::Tone(Tone::new()),
            AudioCheckMode::Echo => AudioCheck::Echo(Echo::new()),
            AudioCheckMode::VoxMeter => AudioCheck::VoxMeter(VoxMeter::new()),
        }
    }

    pub fn mode(&self) -> AudioCheckMode {
        match self {
            AudioCheck::Tone(_) => AudioCheckMode::Tone,
            AudioCheck::Echo(_) => AudioCheckMode::Echo,
            AudioCheck::VoxMeter(_) => AudioCheckMode::VoxMeter,
        }
    }

    /// True once a tone check has played and recorded (echo and meter run until stopped).
    pub fn is_finished(&self) -> bool {
        matches!(self, AudioCheck::Tone(t) if t.phase().is_none())
    }

    /// One 10 ms step: `captured` is the filtered capture block, `out` what to play.
    pub fn process(
        &mut self,
        captured: &Block,
        frame: &CaptureFrame,
        out: &mut Block,
    ) -> Option<AudioCheckSample> {
        match self {
            AudioCheck::Tone(t) => t.process(captured, frame, out),
            AudioCheck::Echo(e) => e.process(captured, frame, out),
            AudioCheck::VoxMeter(m) => m.process(captured, frame, out),
        }
    }
}

const _: () = assert!(ECHO_DELAY_SAMPLES.is_multiple_of(BLOCK_SAMPLES));
