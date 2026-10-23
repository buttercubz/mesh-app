//! Burst packetizer (DESIGN §4.6, §6.4, §7.4): accumulates 10 ms blocks into 20/40 ms
//! frames, encodes them and stamps the VOICE header fields.
//!
//! `ts` is burst-relative in 10 ms ticks since the burst's first VOICE frame: frame *n* of
//! a 40 ms profile has `ts = 4n` (pre-roll frames included, frame 0 carries FIRST).

use crate::capture::PreRoll;
use crate::codec::{CodecError, VoiceEncoder};
use crate::profile::{AudioBandwidth, EncoderConfig, FrameDuration};
use crate::{BLOCK_SAMPLES, Block, MAX_FRAME_SAMPLES, MAX_PAYLOAD};

/// VOICE `cfg` byte: b7-6 dur, b5-4 bw, b3 LBRR, b2 DRED, b1 FIRST, b0 PREROLL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VoiceCfg(pub u8);

impl VoiceCfg {
    pub const LBRR: u8 = 1 << 3;
    pub const DRED: u8 = 1 << 2;
    pub const FIRST: u8 = 1 << 1;
    pub const PREROLL: u8 = 1;

    pub const fn new(
        dur: FrameDuration,
        bw: AudioBandwidth,
        lbrr: bool,
        dred: bool,
        first: bool,
        preroll: bool,
    ) -> Self {
        VoiceCfg(
            (dur.cfg_bits() << 6)
                | (bw.cfg_bits() << 4)
                | if lbrr { Self::LBRR } else { 0 }
                | if dred { Self::DRED } else { 0 }
                | if first { Self::FIRST } else { 0 }
                | if preroll { Self::PREROLL } else { 0 },
        )
    }

    pub const fn duration(self) -> FrameDuration {
        FrameDuration::from_cfg_bits(self.0 >> 6)
    }

    /// `None` for the reserved value 3.
    pub const fn bandwidth(self) -> Option<AudioBandwidth> {
        AudioBandwidth::from_cfg_bits(self.0 >> 4)
    }

    pub const fn lbrr(self) -> bool {
        self.0 & Self::LBRR != 0
    }

    pub const fn dred(self) -> bool {
        self.0 & Self::DRED != 0
    }

    pub const fn first(self) -> bool {
        self.0 & Self::FIRST != 0
    }

    pub const fn preroll(self) -> bool {
        self.0 & Self::PREROLL != 0
    }
}

/// One encoded VOICE payload with its header fields (fixed size: no allocation).
#[derive(Clone)]
pub struct VoiceFrame {
    pub burst: u8,
    pub ts: u16,
    pub cfg: VoiceCfg,
    len: u16,
    payload: [u8; MAX_PAYLOAD],
}

impl VoiceFrame {
    pub const fn empty() -> Self {
        Self {
            burst: 0,
            ts: 0,
            cfg: VoiceCfg(0),
            len: 0,
            payload: [0; MAX_PAYLOAD],
        }
    }

    /// Builds a frame from received fields (`opus` must be 1..=400 bytes).
    pub fn from_parts(burst: u8, ts: u16, cfg: VoiceCfg, opus: &[u8]) -> Option<Self> {
        if opus.is_empty() || opus.len() > MAX_PAYLOAD {
            return None;
        }
        let mut f = Self::empty();
        f.burst = burst;
        f.ts = ts;
        f.cfg = cfg;
        f.set_payload(opus);
        Some(f)
    }

    pub(crate) fn set_payload(&mut self, opus: &[u8]) {
        let n = opus.len().min(MAX_PAYLOAD);
        self.payload[..n].copy_from_slice(&opus[..n]);
        self.len = n as u16;
    }

    /// Codec payload (`opus[1..=400]`).
    pub fn opus(&self) -> &[u8] {
        &self.payload[..usize::from(self.len)]
    }
}

impl Default for VoiceFrame {
    fn default() -> Self {
        Self::empty()
    }
}

impl core::fmt::Debug for VoiceFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VoiceFrame")
            .field("burst", &self.burst)
            .field("ts", &self.ts)
            .field("cfg", &self.cfg)
            .field("len", &self.len)
            .finish()
    }
}

/// Origin-side burst encoder.
pub struct Packetizer {
    enc: Box<dyn VoiceEncoder>,
    burst: u8,
    active: bool,
    buf: [i16; MAX_FRAME_SAMPLES],
    fill: usize,
    /// Pre-roll samples still inside `buf` (for the PREROLL bit).
    preroll_in_buf: bool,
    next_ts: u16,
    frames: u32,
    out: VoiceFrame,
}

impl Packetizer {
    /// Wraps the session's encoder (created once by the engine).
    pub fn new(encoder: Box<dyn VoiceEncoder>) -> Self {
        Self {
            enc: encoder,
            burst: 0,
            active: false,
            buf: [0; MAX_FRAME_SAMPLES],
            fill: 0,
            preroll_in_buf: false,
            next_ts: 0,
            frames: 0,
            out: VoiceFrame::empty(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn burst(&self) -> u8 {
        self.burst
    }

    /// Ticks (10 ms) elapsed since the burst's first frame (`TALK.elapsed_10ms`).
    pub fn elapsed_ticks(&self) -> u16 {
        self.next_ts
            .wrapping_add((self.fill / BLOCK_SAMPLES) as u16)
    }

    /// Frames emitted in the current burst.
    pub fn frames_sent(&self) -> u32 {
        self.frames
    }

    pub fn config(&self) -> &EncoderConfig {
        self.enc.config()
    }

    /// Starts burst `burst`: resets and configures the encoder, then encodes the pre-roll
    /// (40 ms) ahead of the live blocks. Emits 1 (40 ms) or 2 (20 ms) frames.
    pub fn start_burst(
        &mut self,
        burst: u8,
        cfg: &EncoderConfig,
        preroll: &PreRoll,
        emit: &mut impl FnMut(&VoiceFrame),
    ) -> Result<(), CodecError> {
        self.enc.reset()?;
        self.enc.configure(cfg)?;
        self.burst = burst;
        self.active = true;
        self.fill = 0;
        self.next_ts = 0;
        self.frames = 0;
        for i in 0..preroll.len() {
            if let Some(b) = preroll.get(i) {
                self.preroll_in_buf = true;
                self.push_inner(b, emit)?;
            }
        }
        Ok(())
    }

    /// Adds one live block; emits a frame whenever one is complete.
    pub fn push_block(
        &mut self,
        block: &Block,
        emit: &mut impl FnMut(&VoiceFrame),
    ) -> Result<(), CodecError> {
        if !self.active {
            return Ok(());
        }
        self.push_inner(block, emit)
    }

    fn push_inner(
        &mut self,
        block: &Block,
        emit: &mut impl FnMut(&VoiceFrame),
    ) -> Result<(), CodecError> {
        let frame = self.enc.config().frame.samples();
        self.buf[self.fill..self.fill + BLOCK_SAMPLES].copy_from_slice(block);
        self.fill += BLOCK_SAMPLES;
        if self.fill >= frame {
            self.encode_frame(emit)?;
        }
        Ok(())
    }

    fn encode_frame(&mut self, emit: &mut impl FnMut(&VoiceFrame)) -> Result<(), CodecError> {
        let cfg = *self.enc.config();
        let frame = cfg.frame.samples();
        let n = self.enc.encode(&self.buf[..frame], &mut self.out.payload)?;
        self.out.len = n as u16;
        self.out.burst = self.burst;
        self.out.ts = self.next_ts;
        self.out.cfg = VoiceCfg::new(
            cfg.frame,
            cfg.max_bandwidth,
            cfg.inband_fec,
            cfg.dred_duration_10ms > 0,
            self.frames == 0,
            self.preroll_in_buf,
        );
        self.preroll_in_buf = false;
        self.next_ts = self.next_ts.wrapping_add(cfg.frame.ticks());
        self.frames += 1;
        self.fill = 0;
        emit(&self.out);
        Ok(())
    }

    /// Ends the burst (PTT up, MAX_BURST, VOX release): a partial frame is completed with
    /// silence and flushed.
    pub fn finish_burst(&mut self, emit: &mut impl FnMut(&VoiceFrame)) -> Result<(), CodecError> {
        if !self.active {
            return Ok(());
        }
        if self.fill > 0 {
            let frame = self.enc.config().frame.samples();
            self.buf[self.fill..frame].fill(0);
            self.fill = frame;
            self.encode_frame(emit)?;
        }
        self.active = false;
        Ok(())
    }

    /// Stops at once without flushing (FLOOR_REJECT: stop TX immediately).
    pub fn abort_burst(&mut self) {
        self.active = false;
        self.fill = 0;
        self.preroll_in_buf = false;
    }
}

impl core::fmt::Debug for Packetizer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Packetizer")
            .field("burst", &self.burst)
            .field("active", &self.active)
            .field("next_ts", &self.next_ts)
            .finish_non_exhaustive()
    }
}
