//! Per-origin jitter buffer, NetEq-lite (DESIGN §7.5).
//!
//! * Keyed by `(origin, sess)` by the caller ([`crate::ReceivePath`]); a new `burst` id
//!   resets the burst-relative timeline. Frames are placed by `ts` (10 ms ticks).
//! * Target delay `D`: initial 60 ms (40 ms frames, BLE) / 40 ms (20 ms frames, Wi-Fi),
//!   updated per packet from the p95 of `(arrival − expected)` over the last 50 packets,
//!   clamped to `[frame + 20 ms (+ 1 frame with LBRR), 200 ms]`. `D` carries across bursts.
//! * Adaptation: when buffered audio exceeds `D + 30 ms`, one quiet 10 ms sub-block
//!   (< −40 dBFS or < stream floor + 3 dB) is dropped, at most one per 100 ms.
//! * Loss: LBRR (`decode_fec` from the next packet) when the stream has it, otherwise deep
//!   PLC. After 100 ms of consecutive loss the output fades to silence over 20 ms.
//! * Starvation (the frame at the playout point is missing and nothing after it has
//!   arrived): the playout is **stretched** instead: one 10 ms concealment block is played
//!   without consuming the timeline, so the frame still plays when it arrives (BLE
//!   delivers in connection-event bunches; discarding it as late and concealing a whole
//!   frame is what crackled on the first device test). The stretch of one burst is
//!   bounded by [`MAX_STRETCH_US`]; the quiet-block drops above give it back.
//! * A burst ends on TALK STOP (once drained) or 600 ms without frames: flush + decoder reset.
//!
//! Fixed storage, no allocation on `push` / `pull`.

use crate::packetizer::VoiceFrame;
use crate::profile::FrameDuration;
use crate::{BLOCK_SAMPLES, BLOCK_US, MAX_FRAME_SAMPLES};

/// Frame slots (≥ 200 ms max delay + reordering at 20 ms frames).
pub const SLOTS: usize = 32;
/// Packets in the delay statistics window.
pub const STATS_WINDOW: usize = 50;
/// Maximum target delay.
pub const MAX_DELAY_US: u64 = 200_000;
/// Headroom above `D` before a quiet sub-block may be dropped.
pub const DROP_HEADROOM_US: u64 = 30_000;
/// Minimum spacing between two drops.
pub const DROP_SPACING_US: u64 = 100_000;
/// Consecutive loss after which the output fades out.
pub const FADE_AFTER_US: u64 = 100_000;
/// Fade-out length (samples, 20 ms).
pub const FADE_SAMPLES: usize = 320;
/// Silence after which a burst ends without TALK STOP.
pub const BURST_IDLE_US: u64 = 600_000;
/// Most playout stretch (net of drops) one burst may accumulate on starvation; past it a
/// missing frame is concealed and skipped as a loss.
pub const MAX_STRETCH_US: u64 = MAX_DELAY_US;

/// Initial target delay for a stream of `frame`-sized packets.
pub const fn initial_delay_us(frame: FrameDuration) -> u64 {
    match frame {
        FrameDuration::Ms40 | FrameDuration::Ms60 => 60_000,
        _ => 40_000,
    }
}

/// Lower clamp of `D`.
pub const fn min_delay_us(frame: FrameDuration, lbrr: bool) -> u64 {
    frame.us() + 20_000 + if lbrr { frame.us() } else { 0 }
}

/// Outcome of [`JitterBuffer::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    Accepted,
    /// First frame of a new burst (timeline reset).
    NewBurst,
    /// Its playout time already passed.
    Late,
    Duplicate,
    /// Belongs to the burst that just ended, or has a frame size different from its burst.
    Stale,
}

/// What [`JitterBuffer::pull`] produced for this block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullStatus {
    /// No burst: silence.
    Idle,
    /// Burst received, waiting for `D`: silence.
    Buffering,
    /// Decoded audio.
    Normal,
    /// Frame recovered from the next packet's LBRR.
    Fec,
    /// Concealed (deep PLC).
    Plc,
    /// Long loss: faded to silence.
    Faded,
    /// The burst just ended (TALK STOP drained or 600 ms idle); silence.
    Ended,
}

impl PullStatus {
    /// True when the block carries stream audio (to the mixer).
    pub const fn is_audio(self) -> bool {
        matches!(self, PullStatus::Normal | PullStatus::Fec | PullStatus::Plc)
    }
}

/// Counters (diagnostics, Lab).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JitterStats {
    pub received: u32,
    pub late: u32,
    pub duplicates: u32,
    pub decoded: u32,
    pub fec_recovered: u32,
    pub concealed: u32,
    pub dropped_subblocks: u32,
    pub bursts: u32,
    /// 10 ms concealment blocks inserted without consuming the timeline because the
    /// next frame had not arrived yet (playout stretched; the delay grows by 10 ms).
    pub expanded: u32,
}

impl JitterStats {
    /// Field-wise saturating sum (totals across streams).
    pub fn accumulate(&mut self, o: &JitterStats) {
        self.received = self.received.saturating_add(o.received);
        self.late = self.late.saturating_add(o.late);
        self.duplicates = self.duplicates.saturating_add(o.duplicates);
        self.decoded = self.decoded.saturating_add(o.decoded);
        self.fec_recovered = self.fec_recovered.saturating_add(o.fec_recovered);
        self.concealed = self.concealed.saturating_add(o.concealed);
        self.dropped_subblocks = self.dropped_subblocks.saturating_add(o.dropped_subblocks);
        self.bursts = self.bursts.saturating_add(o.bursts);
        self.expanded = self.expanded.saturating_add(o.expanded);
    }
}

#[derive(Clone)]
struct Slot {
    valid: bool,
    frame: VoiceFrame,
}

/// One origin's jitter buffer.
pub struct JitterBuffer {
    slots: [Slot; SLOTS],
    target_us: u64,
    // Burst timeline.
    burst: Option<u8>,
    prev_burst: Option<u8>,
    frame: FrameDuration,
    lbrr: bool,
    first_arrival_us: u64,
    last_rx_us: u64,
    max_end: u32,
    min_offset: Option<i64>,
    playing: bool,
    play_ts: u32,
    stop_pending: bool,
    decoder_reset: bool,
    // Decoded PCM of the current frame.
    pcm: [i16; MAX_FRAME_SAMPLES],
    pcm_blocks: usize,
    pcm_pos: usize,
    held: pull::Held,
    // Loss and adaptation.
    loss_us: u64,
    fade_pos: usize,
    /// The held PCM is a stretch block: playing it does not advance `play_ts`.
    expanding: bool,
    /// Net stretch of this burst (expansions minus drops).
    stretch_us: u64,
    last_drop_us: Option<u64>,
    floor_db: Option<f32>,
    // Delay statistics.
    samples: [u32; STATS_WINDOW],
    n_samples: usize,
    sample_idx: usize,
    stats: JitterStats,
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self {
            slots: core::array::from_fn(|_| Slot {
                valid: false,
                frame: VoiceFrame::empty(),
            }),
            target_us: initial_delay_us(FrameDuration::Ms40),
            burst: None,
            prev_burst: None,
            frame: FrameDuration::Ms40,
            lbrr: false,
            first_arrival_us: 0,
            last_rx_us: 0,
            max_end: 0,
            min_offset: None,
            playing: false,
            play_ts: 0,
            stop_pending: false,
            decoder_reset: false,
            pcm: [0; MAX_FRAME_SAMPLES],
            pcm_blocks: 0,
            pcm_pos: 0,
            held: pull::Held::Decoded,
            loss_us: 0,
            fade_pos: 0,
            expanding: false,
            stretch_us: 0,
            last_drop_us: None,
            floor_db: None,
            samples: [0; STATS_WINDOW],
            n_samples: 0,
            sample_idx: 0,
            stats: JitterStats::default(),
        }
    }

    /// Forgets everything, including `D` and the counters (the slot serves a new stream).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Current target delay `D` in microseconds.
    pub fn target_delay_us(&self) -> u64 {
        self.target_us
    }

    /// Buffered audio ahead of the playout point (received, not yet played).
    pub fn buffered_us(&self) -> u64 {
        if self.burst.is_none() {
            return 0;
        }
        let pos = if self.playing {
            self.play_ts
        } else {
            self.lowest_ts().unwrap_or(self.max_end)
        };
        u64::from(self.max_end.saturating_sub(pos)) * BLOCK_US
    }

    pub fn stats(&self) -> JitterStats {
        self.stats
    }

    /// Burst currently being received or played.
    pub fn current_burst(&self) -> Option<u8> {
        self.burst
    }

    /// True when no burst is active.
    pub fn is_idle(&self) -> bool {
        self.burst.is_none()
    }

    fn slot_index(&self, ts: u32) -> usize {
        (ts / u32::from(self.frame.ticks())) as usize % SLOTS
    }

    /// `ts` of the next frame the playout will fetch (frames before it are late).
    fn next_fetch_ts(&self) -> u32 {
        if self.expanding {
            return self.play_ts;
        }
        self.play_ts + (self.pcm_blocks - self.pcm_pos.min(self.pcm_blocks)) as u32
    }

    /// True when a received frame is still waiting to be played.
    fn has_pending(&self) -> bool {
        let next = if self.playing {
            self.next_fetch_ts()
        } else {
            0
        };
        self.slots
            .iter()
            .any(|s| s.valid && u32::from(s.frame.ts) >= next)
    }

    fn lowest_ts(&self) -> Option<u32> {
        self.slots
            .iter()
            .filter(|s| s.valid)
            .map(|s| u32::from(s.frame.ts))
            .min()
    }

    fn find(&self, ts: u32) -> Option<usize> {
        let i = self.slot_index(ts);
        (self.slots[i].valid && u32::from(self.slots[i].frame.ts) == ts).then_some(i)
    }

    fn clear_slots(&mut self) {
        for s in &mut self.slots {
            s.valid = false;
        }
    }

    fn begin_burst(&mut self, now_us: u64, f: &VoiceFrame) {
        self.clear_slots();
        self.prev_burst = self.burst;
        self.burst = Some(f.burst);
        self.frame = f.cfg.duration();
        self.lbrr = f.cfg.lbrr();
        if self.stats.bursts == 0 {
            self.target_us = initial_delay_us(self.frame);
        }
        self.target_us = self
            .target_us
            .clamp(min_delay_us(self.frame, self.lbrr), MAX_DELAY_US);
        self.first_arrival_us = now_us;
        self.max_end = 0;
        self.min_offset = None;
        self.playing = false;
        self.play_ts = 0;
        self.stop_pending = false;
        self.decoder_reset = true;
        self.pcm_blocks = 0;
        self.pcm_pos = 0;
        self.loss_us = 0;
        self.fade_pos = 0;
        self.expanding = false;
        self.stretch_us = 0;
        self.last_drop_us = None;
        self.floor_db = None;
        self.stats.bursts += 1;
    }

    /// Accepts one VOICE frame received at `now_us` (routing already removed duplicates
    /// across paths; jitter statistics use this first copy's arrival).
    pub fn push(&mut self, now_us: u64, f: &VoiceFrame) -> PushResult {
        let mut result = PushResult::Accepted;
        if self.burst != Some(f.burst) {
            if self.prev_burst == Some(f.burst) {
                return PushResult::Stale;
            }
            self.begin_burst(now_us, f);
            result = PushResult::NewBurst;
        } else if f.cfg.duration() != self.frame {
            return PushResult::Stale;
        }
        self.last_rx_us = now_us;
        let ts = u32::from(f.ts);
        let ticks = u32::from(self.frame.ticks());
        if self.playing && ts < self.next_fetch_ts() {
            self.stats.late += 1;
            return PushResult::Late;
        }
        if self.find(ts).is_some() {
            self.stats.duplicates += 1;
            return PushResult::Duplicate;
        }
        self.stats.received += 1;
        self.record_delay(now_us, ts);
        let i = self.slot_index(ts);
        self.slots[i].frame.clone_from(f);
        self.slots[i].valid = true;
        self.max_end = self.max_end.max(ts + ticks);
        result
    }

    fn record_delay(&mut self, now_us: u64, ts: u32) {
        let offset = now_us as i64 - i64::from(ts) * BLOCK_US as i64;
        let min = self.min_offset.map_or(offset, |m| m.min(offset));
        self.min_offset = Some(min);
        let jitter = (offset - min).clamp(0, i64::from(u32::MAX)) as u32;
        self.samples[self.sample_idx] = jitter;
        self.sample_idx = (self.sample_idx + 1) % STATS_WINDOW;
        self.n_samples = (self.n_samples + 1).min(STATS_WINDOW);
        let mut sorted = [0u32; STATS_WINDOW];
        let n = self.n_samples;
        sorted[..n].copy_from_slice(&self.samples[..n]);
        sorted[..n].sort_unstable();
        let p95 = sorted[((n * 95).div_ceil(100)).saturating_sub(1).min(n - 1)];
        let d = self.frame.us() + u64::from(p95);
        self.target_us = d.clamp(min_delay_us(self.frame, self.lbrr), MAX_DELAY_US);
    }

    /// TALK STOP for `burst`: the burst ends once its buffered frames are played.
    pub fn end_burst(&mut self, burst: u8) {
        if self.burst == Some(burst) {
            self.stop_pending = true;
        }
    }

    fn finish(&mut self) {
        self.clear_slots();
        self.prev_burst = self.burst;
        self.burst = None;
        self.playing = false;
        self.stop_pending = false;
        self.decoder_reset = true;
        self.pcm_blocks = 0;
        self.pcm_pos = 0;
        self.expanding = false;
        self.stretch_us = 0;
    }

    /// True when a frame after `ts` is buffered (a gap at `ts` is then a loss, not lateness).
    fn has_frame_after(&self, ts: u32) -> bool {
        self.slots
            .iter()
            .any(|s| s.valid && u32::from(s.frame.ts) > ts)
    }

    /// Net playout stretch of the current burst (diagnostics).
    pub fn stretch_us(&self) -> u64 {
        self.stretch_us
    }
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for JitterBuffer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JitterBuffer")
            .field("burst", &self.burst)
            .field("target_us", &self.target_us)
            .field("play_ts", &self.play_ts)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

mod pull;

const _: () = assert!(BLOCK_SAMPLES * 6 == MAX_FRAME_SAMPLES);
