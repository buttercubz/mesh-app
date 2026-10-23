//! Signal helpers for the media tests.

#![allow(dead_code)]

use super::world::Block;

/// Amplitude of the `hz` component of `x` (16 kHz), Goertzel, normalised to the sine
/// amplitude.
pub fn tone_amp(x: &[i16], hz: f64) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    let w = 2.0 * std::f64::consts::PI * hz / 16_000.0;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = f64::from(v) + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let p = s1 * s1 + s2 * s2 - c * s1 * s2;
    2.0 * p.max(0.0).sqrt() / x.len() as f64
}

pub fn rms(x: &[i16]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Concatenated samples of `blocks`.
pub fn concat(blocks: &[(u64, Block)]) -> Vec<i16> {
    blocks.iter().flat_map(|(_, b)| b.iter().copied()).collect()
}

/// Output blocks of a node pulled in `[from_ms, to_ms)`.
pub fn window(out: &[(u64, Block)], from_ms: u64, to_ms: u64) -> Vec<(u64, Block)> {
    out.iter()
        .filter(|(t, _)| *t >= from_ms * 1_000 && *t < to_ms * 1_000)
        .cloned()
        .collect()
}

/// First pull time (µs) at or after `from_us` whose block carries the `hz` tone above
/// `min_amp`.
pub fn first_tone(out: &[(u64, Block)], from_us: u64, hz: f64, min_amp: f64) -> Option<u64> {
    out.iter()
        .filter(|(t, _)| *t >= from_us)
        .find(|(_, b)| tone_amp(b, hz) >= min_amp)
        .map(|(t, _)| *t)
}

/// Largest `hz` amplitude over consecutive windows of `blocks` output blocks pulled in
/// `[from_us, to_us)`. A single 10 ms block resolves only 100 Hz, so neighbouring
/// talkers' energy leaks into it; 50 ms windows separate a cue from the speech.
pub fn max_windowed_amp(
    out: &[(u64, Block)],
    from_us: u64,
    to_us: u64,
    hz: f64,
    blocks: usize,
) -> f64 {
    let sel: Vec<(u64, Block)> = out
        .iter()
        .filter(|(t, _)| *t >= from_us && *t < to_us)
        .cloned()
        .collect();
    sel.chunks(blocks.max(1))
        .map(|c| tone_amp(&concat(c), hz))
        .fold(0.0, f64::max)
}
