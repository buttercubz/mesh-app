//! Deterministic synthesized speech-like signal (16 kHz mono): syllables of a voiced
//! fundamental with two formant-like harmonics and an attack/decay envelope,
//! separated by short pauses. It feeds simulated microphones (Lab `audio = full`)
//! and is the fallback voice of the Demo bots when no TTS PCM is supplied (no bundled
//! audio assets, DESIGN §13.2 / WP-11).

use mesh_core::BLOCK_SAMPLES;

use crate::rng::SimRng;

/// Sample rate of the RT plane (§3.6).
pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Copy)]
struct Syllable {
    /// Samples left in this syllable (voiced part + pause).
    voiced: u32,
    pause: u32,
    pos: u32,
    f0: f64,
    amp: f64,
}

/// A speech-like generator; the same seed yields the same samples.
#[derive(Debug, Clone)]
pub struct SpeechSynth {
    rng: SimRng,
    syl: Syllable,
    phase: f64,
    /// Peak amplitude (0..1 of full scale).
    pub level: f64,
    /// Probability that a syllable ends with a breath pause (300-700 ms) instead of a
    /// short one (30-90 ms).
    pub breath_prob: f64,
}

impl SpeechSynth {
    pub fn new(seed: u64, tag: u64) -> Self {
        let mut s = SpeechSynth {
            rng: SimRng::new(seed, 0x5917_0000 ^ tag),
            syl: Syllable {
                voiced: 0,
                pause: 0,
                pos: 0,
                f0: 120.0,
                amp: 0.0,
            },
            phase: 0.0,
            level: 0.3,
            breath_prob: 0.15,
        };
        s.next_syllable();
        s
    }

    fn next_syllable(&mut self) {
        let ms = |x: u64| (x * u64::from(SAMPLE_RATE) / 1_000) as u32;
        let voiced = ms(self.rng.range_u64(110, 260));
        // Mostly short pauses between syllables, sometimes a breath between phrases.
        let pause = if self.rng.chance(self.breath_prob) {
            ms(self.rng.range_u64(300, 700))
        } else {
            ms(self.rng.range_u64(30, 90))
        };
        self.syl = Syllable {
            voiced,
            pause,
            pos: 0,
            f0: self.rng.range_f64(105.0, 210.0),
            amp: self.rng.range_f64(0.6, 1.0),
        };
    }

    fn envelope(&self) -> f64 {
        let s = self.syl;
        if s.pos >= s.voiced {
            return 0.0;
        }
        let attack = (SAMPLE_RATE / 50) as f64; // 20 ms
        let release = (SAMPLE_RATE / 25) as f64; // 40 ms
        let p = f64::from(s.pos);
        let left = f64::from(s.voiced - s.pos);
        (p / attack).min(1.0) * (left / release).min(1.0)
    }

    /// Starts a fresh syllable now (a talk span begins after silence).
    pub fn restart(&mut self) {
        self.next_syllable();
    }

    /// Next sample.
    pub fn sample(&mut self) -> i16 {
        if self.syl.pos >= self.syl.voiced + self.syl.pause {
            self.next_syllable();
        }
        let env = self.envelope() * self.syl.amp;
        let tau = core::f64::consts::TAU;
        // Slight pitch glide inside the syllable.
        let glide = 1.0 - 0.15 * f64::from(self.syl.pos) / f64::from(self.syl.voiced.max(1));
        self.phase = (self.phase + self.syl.f0 * glide / f64::from(SAMPLE_RATE)).fract();
        let v = (tau * self.phase).sin() * 0.6
            + (tau * self.phase * 3.0).sin() * 0.25
            + (tau * self.phase * 7.0).sin() * 0.15;
        self.syl.pos += 1;
        (v * env * self.level * f64::from(i16::MAX)).round() as i16
    }

    /// Fills one 10 ms RT block.
    pub fn fill_block(&mut self, out: &mut [i16; BLOCK_SAMPLES]) {
        for s in out.iter_mut() {
            *s = self.sample();
        }
    }

    /// `ms` milliseconds of signal.
    pub fn render_ms(&mut self, ms: u32) -> Vec<i16> {
        (0..ms * SAMPLE_RATE / 1_000)
            .map(|_| self.sample())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_speech_like() {
        let a = SpeechSynth::new(1, 2).render_ms(3_000);
        let b = SpeechSynth::new(1, 2).render_ms(3_000);
        assert_eq!(a, b);
        assert_ne!(a, SpeechSynth::new(1, 3).render_ms(3_000));
        // Bursts and pauses: some 10 ms blocks are silent, most are not.
        let blocks: Vec<i64> = a
            .chunks(160)
            .map(|c| c.iter().map(|s| i64::from(*s).abs()).max().unwrap_or(0))
            .collect();
        let silent = blocks.iter().filter(|m| **m < 50).count();
        assert!(
            silent > 5 && silent < blocks.len() / 2,
            "{silent}/{}",
            blocks.len()
        );
        let peak = a.iter().map(|s| i32::from(*s).abs()).max().unwrap();
        assert!(peak > 3_000 && peak < 12_000, "{peak}");
    }
}
