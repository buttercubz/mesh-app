//! Tone synthesis for earcons and the audio check (DESIGN §6.5, §1.3 F0).

use core::f32::consts::TAU;

use crate::{Block, SAMPLE_RATE, from_dbfs, sat_i16};

/// Ramp at each tone edge (avoids clicks).
pub const RAMP_SAMPLES: usize = 80; // 5 ms

/// One tone (or a linear chirp when `f_start != f_end`) followed by silence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneSegment {
    pub f_start_hz: f32,
    pub f_end_hz: f32,
    pub on_ms: u16,
    pub off_ms: u16,
}

impl ToneSegment {
    pub const fn tone(f_hz: f32, on_ms: u16, off_ms: u16) -> Self {
        Self {
            f_start_hz: f_hz,
            f_end_hz: f_hz,
            on_ms,
            off_ms,
        }
    }

    const fn on_samples(&self) -> usize {
        self.on_ms as usize * SAMPLE_RATE as usize / 1000
    }

    const fn total_samples(&self) -> usize {
        (self.on_ms as usize + self.off_ms as usize) * SAMPLE_RATE as usize / 1000
    }
}

/// Continuous sine oscillator (phase-accumulating, so frequency steps are click-free).
#[derive(Debug, Clone, Copy)]
pub struct ToneGenerator {
    phase: f32,
    freq_hz: f32,
    amp: f32,
}

impl ToneGenerator {
    /// A sine at `freq_hz` and `level_dbfs` (peak).
    pub fn new(freq_hz: f32, level_dbfs: f32) -> Self {
        Self {
            phase: 0.0,
            freq_hz,
            amp: from_dbfs(level_dbfs) * 32767.0,
        }
    }

    pub fn set_freq(&mut self, freq_hz: f32) {
        self.freq_hz = freq_hz;
    }

    /// Next sample.
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        let s = self.phase.sin() * self.amp;
        self.phase += TAU * self.freq_hz / SAMPLE_RATE as f32;
        if self.phase >= TAU {
            self.phase -= TAU;
        }
        s
    }

    /// Fills a block.
    pub fn fill(&mut self, out: &mut Block) {
        for o in out.iter_mut() {
            *o = sat_i16(self.next_sample());
        }
    }
}

/// Plays a static sequence of [`ToneSegment`]s block by block.
#[derive(Debug, Clone, Copy)]
pub struct TonePlayer {
    segs: &'static [ToneSegment],
    idx: usize,
    pos: usize,
    osc: ToneGenerator,
}

impl TonePlayer {
    pub fn new(segs: &'static [ToneSegment], level_dbfs: f32) -> Self {
        let f0 = segs.first().map_or(0.0, |s| s.f_start_hz);
        Self {
            segs,
            idx: 0,
            pos: 0,
            osc: ToneGenerator::new(f0, level_dbfs),
        }
    }

    pub fn is_done(&self) -> bool {
        self.idx >= self.segs.len()
    }

    /// Total length in samples.
    pub fn len_samples(&self) -> usize {
        self.segs.iter().map(ToneSegment::total_samples).sum()
    }

    /// Writes the next 10 ms (silence once done). Returns true when the block carries
    /// part of the sequence (false once it had already finished).
    pub fn render(&mut self, out: &mut Block) -> bool {
        out.fill(0);
        let active = !self.is_done();
        for o in out.iter_mut() {
            let Some(seg) = self.segs.get(self.idx) else {
                break;
            };
            let on = seg.on_samples();
            if self.pos < on {
                let t = self.pos as f32 / on.max(1) as f32;
                self.osc
                    .set_freq(seg.f_start_hz + (seg.f_end_hz - seg.f_start_hz) * t);
                let edge = self.pos.min(on - 1 - self.pos);
                let ramp = if edge < RAMP_SAMPLES {
                    0.5 - 0.5 * (core::f32::consts::PI * edge as f32 / RAMP_SAMPLES as f32).cos()
                } else {
                    1.0
                };
                *o = sat_i16(self.osc.next_sample() * ramp);
            }
            self.pos += 1;
            if self.pos >= seg.total_samples() {
                self.idx += 1;
                self.pos = 0;
            }
        }
        active
    }
}
