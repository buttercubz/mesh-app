//! `SimCodec`: a small, deterministic, pure-Rust codec for wasm32 and the simulator.
//!
//! It is a real (lossy, intelligible) codec so the browser demo and `mesh-sim` move audio
//! through the same packetizer / jitter-buffer / mixer code as the phones, but it is not
//! interoperable with Opus and never runs on devices.
//!
//! * 16 kHz → 8 kHz decimation (31-tap windowed-sinc low-pass), adaptive ADPCM with a
//!   Jayant step adaptation, 2 bits/sample below 20 kbps and 3 bits/sample above
//!   (so packet sizes track the profile bitrate: BLE16 40 ms ≈ 85 B, WIFI20 20 ms ≈ 45 B).
//! * Every packet is self-contained (predictor and step are in its header), so loss never
//!   desynchronises the decoder.
//! * With `inband_fec` on, a packet also carries the previous frame at 2 bits/sample, the
//!   SimCodec counterpart of SILK LBRR, exercised by `decode_fec`.
//! * Concealment repeats the last frame with a −6 dB per frame decay.
//!
//! Packet layout: `hdr u8` (b1-0 bits−2, b2 FEC section present, b7-4 frame ticks),
//! `pred i16 LE`, `step u16 LE`, packed codes; then optionally the FEC section
//! `pred i16 LE`, `step u16 LE`, packed 2-bit codes of the previous frame.

use super::{Codec, CodecError, VoiceDecoder, VoiceEncoder};
use crate::profile::{EncoderConfig, Profile};
use crate::{MAX_FRAME_SAMPLES, SAMPLE_RATE, sat_i16};

const HALF: usize = MAX_FRAME_SAMPLES / 2;
const TAPS: usize = 31;
const MIN_STEP: f32 = 8.0;
const MAX_STEP: f32 = 16_384.0;
const SECTION_HDR: usize = 4;

/// The pure-Rust simulation codec family.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimCodec;

impl SimCodec {
    pub fn new() -> Self {
        SimCodec
    }
}

impl Codec for SimCodec {
    fn name(&self) -> &'static str {
        "sim"
    }

    fn new_encoder(&self) -> Result<Box<dyn VoiceEncoder>, CodecError> {
        Ok(Box::new(SimEncoder::new()))
    }

    fn new_decoder(&self) -> Result<Box<dyn VoiceDecoder>, CodecError> {
        Ok(Box::new(SimDecoder::new()))
    }
}

/// Low-pass FIR (cutoff 3.6 kHz at 16 kHz), Hamming-windowed sinc, unity DC gain.
fn lowpass_taps() -> [f32; TAPS] {
    let mut h = [0f32; TAPS];
    let fc = 3600.0 / SAMPLE_RATE as f32;
    let mid = (TAPS / 2) as f32;
    let mut sum = 0.0;
    for (i, t) in h.iter_mut().enumerate() {
        let n = i as f32 - mid;
        let sinc = if n == 0.0 {
            2.0 * fc
        } else {
            (2.0 * core::f32::consts::PI * fc * n).sin() / (core::f32::consts::PI * n)
        };
        let w = 0.54 - 0.46 * (2.0 * core::f32::consts::PI * i as f32 / (TAPS - 1) as f32).cos();
        *t = sinc * w;
        sum += *t;
    }
    for t in &mut h {
        *t /= sum;
    }
    h
}

/// FIR with a circular history, one sample in, one sample out.
#[derive(Debug, Clone)]
struct Fir {
    h: [f32; TAPS],
    x: [f32; TAPS],
    pos: usize,
}

impl Fir {
    fn new() -> Self {
        Self {
            h: lowpass_taps(),
            x: [0.0; TAPS],
            pos: 0,
        }
    }

    fn push(&mut self, s: f32) -> f32 {
        self.x[self.pos] = s;
        let mut acc = 0.0;
        let mut idx = self.pos;
        for &c in &self.h {
            acc += c * self.x[idx];
            idx = if idx == 0 { TAPS - 1 } else { idx - 1 };
        }
        self.pos = (self.pos + 1) % TAPS;
        acc
    }

    fn reset(&mut self) {
        self.x = [0.0; TAPS];
        self.pos = 0;
    }
}

/// Adaptive (Jayant) ADPCM state for `bits` bits per sample.
#[derive(Debug, Clone, Copy)]
struct Adpcm {
    pred: f32,
    step: f32,
    bits: u32,
}

impl Adpcm {
    fn levels(bits: u32) -> i32 {
        1 << (bits - 1)
    }

    /// Step multiplier for a code magnitude (0 = smallest level).
    fn multiplier(bits: u32, mag: i32) -> f32 {
        let n = Self::levels(bits) - 1;
        if n == 0 {
            return 1.0;
        }
        0.85 + 0.85 * (mag as f32 / n as f32)
    }

    fn encode(&mut self, x: f32) -> u8 {
        let levels = Self::levels(self.bits);
        let e = (x - self.pred) / self.step;
        // Mid-rise quantiser: codes 0..levels-1 positive, levels..2*levels-1 negative.
        let mag = (e.abs()).floor().min((levels - 1) as f32) as i32;
        let code = if e >= 0.0 { mag } else { mag + levels };
        self.apply(code);
        code as u8
    }

    fn apply(&mut self, code: i32) {
        let levels = Self::levels(self.bits);
        let (mag, neg) = if code >= levels {
            (code - levels, true)
        } else {
            (code, false)
        };
        let q = (mag as f32 + 0.5) * self.step;
        self.pred = (self.pred + if neg { -q } else { q }).clamp(-32768.0, 32767.0) * 0.998;
        self.step = (self.step * Self::multiplier(self.bits, mag)).clamp(MIN_STEP, MAX_STEP);
    }
}

/// Writes `bits`-wide codes LSB-first.
struct BitWriter<'a> {
    out: &'a mut [u8],
    pos: usize,
    acc: u32,
    n: u32,
}

impl<'a> BitWriter<'a> {
    fn put(&mut self, v: u8, bits: u32) -> Result<(), CodecError> {
        self.acc |= u32::from(v) << self.n;
        self.n += bits;
        while self.n >= 8 {
            *self
                .out
                .get_mut(self.pos)
                .ok_or(CodecError::BufferTooSmall)? = self.acc as u8;
            self.pos += 1;
            self.acc >>= 8;
            self.n -= 8;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<usize, CodecError> {
        if self.n > 0 {
            *self
                .out
                .get_mut(self.pos)
                .ok_or(CodecError::BufferTooSmall)? = self.acc as u8;
            self.pos += 1;
        }
        Ok(self.pos)
    }
}

/// Reads `bits`-wide codes LSB-first.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    n: u32,
}

impl BitReader<'_> {
    fn get(&mut self, bits: u32) -> Result<u8, CodecError> {
        while self.n < bits {
            let b = *self.data.get(self.pos).ok_or(CodecError::InvalidPacket)?;
            self.acc |= u32::from(b) << self.n;
            self.pos += 1;
            self.n += 8;
        }
        let v = (self.acc & ((1 << bits) - 1)) as u8;
        self.acc >>= bits;
        self.n -= bits;
        Ok(v)
    }
}

fn packed_len(samples: usize, bits: u32) -> usize {
    (samples * bits as usize).div_ceil(8)
}

/// Encodes `half` (8 kHz samples) as one section; returns bytes written.
fn encode_section(half: &[f32], bits: u32, out: &mut [u8]) -> Result<usize, CodecError> {
    if out.len() < SECTION_HDR + packed_len(half.len(), bits) {
        return Err(CodecError::BufferTooSmall);
    }
    // Start each packet from the first sample, so it decodes alone.
    let pred = half
        .first()
        .copied()
        .unwrap_or(0.0)
        .clamp(-32768.0, 32767.0);
    let mean_abs = half.iter().map(|s| s.abs()).sum::<f32>() / half.len().max(1) as f32;
    let step = (mean_abs * 0.5).clamp(MIN_STEP, MAX_STEP);
    let pred_i = pred.round() as i16;
    let step_u = step.round() as u16;
    out[0..2].copy_from_slice(&pred_i.to_le_bytes());
    out[2..4].copy_from_slice(&step_u.to_le_bytes());
    let mut st = Adpcm {
        pred: f32::from(pred_i),
        step: f32::from(step_u),
        bits,
    };
    let mut w = BitWriter {
        out: &mut out[SECTION_HDR..],
        pos: 0,
        acc: 0,
        n: 0,
    };
    for &s in half {
        w.put(st.encode(s), bits)?;
    }
    Ok(SECTION_HDR + w.finish()?)
}

/// Decodes one section into `half`; returns bytes consumed.
fn decode_section(data: &[u8], bits: u32, half: &mut [f32]) -> Result<usize, CodecError> {
    if data.len() < SECTION_HDR {
        return Err(CodecError::InvalidPacket);
    }
    let pred = i16::from_le_bytes([data[0], data[1]]);
    let step = u16::from_le_bytes([data[2], data[3]]);
    let mut st = Adpcm {
        pred: f32::from(pred),
        step: f32::from(step).max(MIN_STEP),
        bits,
    };
    let mut r = BitReader {
        data: &data[SECTION_HDR..],
        pos: 0,
        acc: 0,
        n: 0,
    };
    for s in half.iter_mut() {
        st.apply(i32::from(r.get(bits)?));
        *s = st.pred;
    }
    Ok(SECTION_HDR + packed_len(half.len(), bits))
}

/// SimCodec encoder.
#[derive(Debug, Clone)]
pub struct SimEncoder {
    cfg: EncoderConfig,
    fir: Fir,
    prev: [f32; HALF],
    prev_len: usize,
}

impl SimEncoder {
    pub fn new() -> Self {
        Self {
            cfg: EncoderConfig::for_profile(Profile::Ble16, SAMPLE_RATE, 0.0),
            fir: Fir::new(),
            prev: [0.0; HALF],
            prev_len: 0,
        }
    }

    fn bits(&self) -> u32 {
        if self.cfg.bitrate_bps >= 20_000 { 3 } else { 2 }
    }
}

impl Default for SimEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceEncoder for SimEncoder {
    fn configure(&mut self, cfg: &EncoderConfig) -> Result<(), CodecError> {
        if cfg.frame.samples() > MAX_FRAME_SAMPLES {
            return Err(CodecError::BadArg);
        }
        self.cfg = *cfg;
        Ok(())
    }

    fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    fn reset(&mut self) -> Result<(), CodecError> {
        self.fir.reset();
        self.prev_len = 0;
        Ok(())
    }

    fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, CodecError> {
        if pcm.len() != self.cfg.frame.samples() || out.is_empty() {
            return Err(CodecError::BadArg);
        }
        let mut half = [0f32; HALF];
        let n = pcm.len() / 2;
        for i in 0..n {
            let _ = self.fir.push(f32::from(pcm[2 * i]));
            half[i] = self.fir.push(f32::from(pcm[2 * i + 1]));
        }
        let bits = self.bits();
        let fec = self.cfg.inband_fec && self.prev_len == n;
        out[0] = ((bits - 2) as u8) | (u8::from(fec) << 2) | ((self.cfg.frame.ticks() as u8) << 4);
        let mut len = 1 + encode_section(&half[..n], bits, &mut out[1..])?;
        if fec {
            len += encode_section(&self.prev[..n], 2, &mut out[len..])?;
        }
        self.prev[..n].copy_from_slice(&half[..n]);
        self.prev_len = n;
        Ok(len)
    }

    fn lookahead(&self) -> usize {
        // Decimation FIR group delay (input rate) plus the interpolation FIR in the decoder.
        2 * (TAPS / 2)
    }
}

/// SimCodec decoder.
#[derive(Debug, Clone)]
pub struct SimDecoder {
    fir: Fir,
    last: [f32; HALF],
    last_len: usize,
    conceal_gain: f32,
}

impl SimDecoder {
    pub fn new() -> Self {
        Self {
            fir: Fir::new(),
            last: [0.0; HALF],
            last_len: 0,
            conceal_gain: 1.0,
        }
    }

    fn render(&mut self, half_len: usize, gain: f32, out: &mut [i16]) {
        for i in 0..half_len {
            let s = self.last[i] * gain;
            out[2 * i] = sat_i16(2.0 * self.fir.push(s));
            out[2 * i + 1] = sat_i16(2.0 * self.fir.push(0.0));
        }
    }

    fn check(out: &[i16]) -> Result<usize, CodecError> {
        if out.is_empty() || !out.len().is_multiple_of(2) || out.len() > MAX_FRAME_SAMPLES {
            return Err(CodecError::BadArg);
        }
        Ok(out.len() / 2)
    }

    fn parse_hdr(packet: &[u8], n: usize) -> Result<(u32, bool), CodecError> {
        let hdr = *packet.first().ok_or(CodecError::InvalidPacket)?;
        let bits = u32::from(hdr & 3) + 2;
        if bits > 4 || usize::from(hdr >> 4) * 80 != n {
            return Err(CodecError::InvalidPacket);
        }
        Ok((bits, hdr & 4 != 0))
    }
}

impl Default for SimDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceDecoder for SimDecoder {
    fn decode(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize, CodecError> {
        let n = Self::check(out)?;
        let (bits, _) = Self::parse_hdr(packet, n)?;
        decode_section(&packet[1..], bits, &mut self.last[..n])?;
        self.last_len = n;
        self.conceal_gain = 1.0;
        self.render(n, 1.0, out);
        Ok(out.len())
    }

    fn decode_fec(&mut self, next: &[u8], out: &mut [i16]) -> Result<usize, CodecError> {
        let n = Self::check(out)?;
        let (bits, fec) = Self::parse_hdr(next, n)?;
        if !fec {
            return self.conceal(out);
        }
        let primary = SECTION_HDR + packed_len(n, bits);
        let section = next.get(1 + primary..).ok_or(CodecError::InvalidPacket)?;
        decode_section(section, 2, &mut self.last[..n])?;
        self.last_len = n;
        self.conceal_gain = 1.0;
        self.render(n, 1.0, out);
        Ok(out.len())
    }

    fn conceal(&mut self, out: &mut [i16]) -> Result<usize, CodecError> {
        let n = Self::check(out)?;
        if self.last_len != n {
            self.last[..n].fill(0.0);
            self.last_len = n;
        }
        self.conceal_gain *= 0.5;
        let g = self.conceal_gain;
        self.render(n, g, out);
        Ok(out.len())
    }

    fn reset(&mut self) -> Result<(), CodecError> {
        self.fir.reset();
        self.last_len = 0;
        self.conceal_gain = 1.0;
        Ok(())
    }
}
