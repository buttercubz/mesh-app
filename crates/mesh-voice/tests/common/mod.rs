//! Shared test helpers: fixture loading, deterministic noise, spectral distance, fakes.
#![allow(dead_code)]

use std::path::PathBuf;

use mesh_voice::codec::{CodecError, VoiceDecoder};
use mesh_voice::fsm::TalkEnv;
use mesh_voice::{BLOCK_SAMPLES, Block};

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

/// Reads a 16 kHz mono 16-bit PCM WAV (the fixture format).
pub fn read_wav(name: &str) -> Vec<i16> {
    let bytes = std::fs::read(fixture_path(name)).expect("fixture");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut pos = 12;
    let mut fmt_ok = false;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..pos + 8 + len];
        if id == b"fmt " {
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let bits = u16::from_le_bytes([body[14], body[15]]);
            assert_eq!((channels, rate, bits), (1, 16_000, 16), "fixture format");
            fmt_ok = true;
        } else if id == b"data" {
            assert!(fmt_ok);
            return body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes(*c))
                .collect();
        }
        pos += 8 + len + (len & 1);
    }
    panic!("no data chunk in {name}");
}

/// Speech label spans in samples.
pub fn speech_labels() -> Vec<(usize, usize)> {
    std::fs::read_to_string(fixture_path("speech.labels"))
        .unwrap()
        .lines()
        .map(|l| {
            let mut it = l
                .split_whitespace()
                .map(|v| v.parse::<usize>().unwrap() * 16);
            (it.next().unwrap(), it.next().unwrap())
        })
        .collect()
}

/// Splits PCM into whole 10 ms blocks.
pub fn blocks(pcm: &[i16]) -> Vec<Block> {
    pcm.as_chunks::<BLOCK_SAMPLES>().0.to_vec()
}

/// Deterministic xorshift noise.
pub struct Noise(u64);

impl Noise {
    pub fn new(seed: u64) -> Self {
        Noise(seed | 1)
    }

    pub fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    /// White-ish noise at `level_dbfs` RMS.
    pub fn pcm(&mut self, n: usize, level_dbfs: f32) -> Vec<i16> {
        let amp = 32768.0 * 10f32.powf(level_dbfs / 20.0) * 3f32.sqrt();
        (0..n).map(|_| (self.next_f32() * amp) as i16).collect()
    }
}

/// `a + gain_db(b)` with `b` looped to `a`'s length.
pub fn mix(a: &[i16], b: &[i16], b_gain_db: f32) -> Vec<i16> {
    let g = 10f32.powf(b_gain_db / 20.0);
    a.iter()
        .enumerate()
        .map(|(i, &x)| {
            (f32::from(x) + g * f32::from(b[i % b.len()])).clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}

pub fn sine(freq: f32, level_dbfs: f32, n: usize) -> Vec<i16> {
    let amp = 32767.0 * 10f32.powf(level_dbfs / 20.0);
    (0..n)
        .map(|i| (amp * (2.0 * std::f32::consts::PI * freq * i as f32 / 16_000.0).sin()) as i16)
        .collect()
}

/// In-place radix-2 FFT (re, im).
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Power spectrum (dB) of 512-sample Hann frames, hop 256.
pub fn spectrogram(x: &[i16]) -> Vec<Vec<f64>> {
    const N: usize = 512;
    let mut out = Vec::new();
    let mut start = 0;
    while start + N <= x.len() {
        let mut re: Vec<f64> = (0..N)
            .map(|i| {
                let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N as f64).cos();
                f64::from(x[start + i]) / 32768.0 * w
            })
            .collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im);
        out.push(
            (0..=N / 2)
                .map(|k| 10.0 * (re[k] * re[k] + im[k] * im[k] + 1e-10).log10())
                .collect(),
        );
        start += N / 2;
    }
    out
}

/// Mean log-spectral distance (dB) between `reference` and `test` over frames where the
/// reference is active (> −50 dBFS), for bins up to `max_hz`.
pub fn log_spectral_distance(reference: &[i16], test: &[i16], max_hz: f64) -> f64 {
    let n = reference.len().min(test.len());
    let a = spectrogram(&reference[..n]);
    let b = spectrogram(&test[..n]);
    let max_bin = (max_hz / 8000.0 * 256.0) as usize;
    let mut total = 0.0;
    let mut frames = 0;
    for (fa, fb) in a.iter().zip(b.iter()) {
        let energy: f64 = fa[1..max_bin].iter().map(|db| 10f64.powf(db / 10.0)).sum();
        if 10.0 * energy.log10() < -30.0 {
            continue;
        }
        let d: f64 = fa[1..max_bin]
            .iter()
            .zip(fb[1..max_bin].iter())
            .map(|(x, y)| (x.max(-90.0) - y.max(-90.0)).powi(2))
            .sum::<f64>()
            / (max_bin - 1) as f64;
        total += d.sqrt();
        frames += 1;
    }
    assert!(frames > 10, "too few active frames");
    total / frames as f64
}

/// Energy fraction (dB) above `hz` relative to the whole spectrum.
pub fn band_energy_db(x: &[i16], lo_hz: f64, hi_hz: f64) -> f64 {
    let spec = spectrogram(x);
    let (lo, hi) = (
        (lo_hz / 8000.0 * 256.0) as usize,
        (hi_hz / 8000.0 * 256.0) as usize,
    );
    let mut band = 0.0;
    let mut all = 0.0;
    for f in &spec {
        for (k, db) in f.iter().enumerate().skip(1) {
            let p = 10f64.powf(db / 10.0);
            all += p;
            if k >= lo && k < hi {
                band += p;
            }
        }
    }
    10.0 * (band / all).log10()
}

/// SNR (dB) of `test` against `reference`.
pub fn snr_db(reference: &[i16], test: &[i16]) -> f64 {
    let (mut s, mut e) = (0.0, 0.0);
    for (&r, &t) in reference.iter().zip(test) {
        s += f64::from(r).powi(2);
        e += (f64::from(r) - f64::from(t)).powi(2);
    }
    10.0 * (s / e.max(1.0)).log10()
}

/// Test decoder: a packet is `[tag]`; decoding writes `tag` into every sample, FEC writes
/// `1000 + tag − 1` (the previous frame), concealment writes −1.
#[derive(Debug, Default)]
pub struct TagDecoder {
    pub resets: u32,
}

impl VoiceDecoder for TagDecoder {
    fn decode(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize, CodecError> {
        let tag = u16::from_le_bytes([packet[0], packet[1]]) as i16;
        out.fill(tag);
        Ok(out.len())
    }

    fn decode_fec(&mut self, next: &[u8], out: &mut [i16]) -> Result<usize, CodecError> {
        let tag = u16::from_le_bytes([next[0], next[1]]) as i16;
        out.fill(1000 + tag - 1);
        Ok(out.len())
    }

    fn conceal(&mut self, out: &mut [i16]) -> Result<usize, CodecError> {
        out.fill(-1);
        Ok(out.len())
    }

    fn reset(&mut self) -> Result<(), CodecError> {
        self.resets += 1;
        Ok(())
    }
}

/// Talk environment with scripted floor decisions.
pub struct FakeEnv {
    pub admit: bool,
    pub jitter_us: u64,
    pub admits_asked: u32,
}

impl FakeEnv {
    pub fn admitting() -> Self {
        Self {
            admit: true,
            jitter_us: 0,
            admits_asked: 0,
        }
    }

    pub fn refusing() -> Self {
        Self {
            admit: false,
            jitter_us: 250_000,
            admits_asked: 0,
        }
    }
}

impl TalkEnv for FakeEnv {
    fn local_admit(&mut self, _now_us: u64) -> bool {
        self.admits_asked += 1;
        self.admit
    }

    fn holdoff_jitter_us(&mut self) -> u64 {
        self.jitter_us
    }
}

/// RMS level in dBFS as f64.
pub fn rms_db(x: &[i16]) -> f64 {
    f64::from(mesh_voice::rms_dbfs(x))
}
