//! Playout side of the jitter buffer: one 10 ms block per call.

use super::{
    BURST_IDLE_US, DROP_HEADROOM_US, DROP_SPACING_US, FADE_AFTER_US, FADE_SAMPLES, JitterBuffer,
    MAX_STRETCH_US, PullStatus,
};
use crate::codec::VoiceDecoder;
use crate::vad::FLOOR_RISE_DB_PER_S;
use crate::{BLOCK_SAMPLES, BLOCK_US, Block, rms_dbfs};

/// Sub-blocks quieter than this may be dropped by the adaptation.
const DROP_LEVEL_DBFS: f32 = -40.0;
/// Or quieter than the stream floor plus this margin.
const DROP_FLOOR_MARGIN_DB: f32 = 3.0;

impl JitterBuffer {
    /// Produces the next 10 ms of this stream at `now_us` (silence when idle/buffering).
    pub fn pull(&mut self, now_us: u64, dec: &mut dyn VoiceDecoder, out: &mut Block) -> PullStatus {
        out.fill(0);
        if self.burst.is_none() {
            return PullStatus::Idle;
        }
        if self.decoder_reset {
            let _ = dec.reset();
            self.decoder_reset = false;
        }
        if now_us.saturating_sub(self.last_rx_us) >= BURST_IDLE_US && !self.has_pending() {
            self.end_now(dec);
            return PullStatus::Ended;
        }
        if !self.playing {
            let start_at = self.first_arrival_us + self.target_us.saturating_sub(self.frame.us());
            if now_us < start_at {
                return PullStatus::Buffering;
            }
            match self.lowest_ts() {
                Some(ts) => {
                    self.play_ts = ts;
                    self.playing = true;
                }
                None => return PullStatus::Buffering,
            }
        }
        let mut dropped = false;
        loop {
            if self.pcm_pos >= self.pcm_blocks && !self.fetch(dec) {
                self.end_now(dec);
                return PullStatus::Ended;
            }
            let start = self.pcm_pos * BLOCK_SAMPLES;
            let level = rms_dbfs(&self.pcm[start..start + BLOCK_SAMPLES]);
            if !dropped && self.held != Held::Concealed && self.should_drop(now_us, level) {
                self.pcm_pos += 1;
                self.play_ts += 1;
                self.stats.dropped_subblocks += 1;
                self.stretch_us = self.stretch_us.saturating_sub(BLOCK_US);
                self.last_drop_us = Some(now_us);
                dropped = true;
                continue;
            }
            out.copy_from_slice(&self.pcm[start..start + BLOCK_SAMPLES]);
            self.pcm_pos += 1;
            if !self.expanding {
                self.play_ts += 1;
            }
            if self.held == Held::Concealed {
                self.loss_us += BLOCK_US;
            } else {
                self.update_floor(level);
            }
            break;
        }
        match self.held {
            Held::Decoded => PullStatus::Normal,
            Held::Fec => PullStatus::Fec,
            Held::Concealed => self.apply_fade(out),
        }
    }

    /// Fades concealment out over 20 ms once more than 100 ms of it has played.
    fn apply_fade(&mut self, out: &mut Block) -> PullStatus {
        if self.loss_us <= FADE_AFTER_US {
            return PullStatus::Plc;
        }
        for s in out.iter_mut() {
            let g = 1.0 - (self.fade_pos as f32 / FADE_SAMPLES as f32).min(1.0);
            *s = (f32::from(*s) * g) as i16;
            self.fade_pos = (self.fade_pos + 1).min(FADE_SAMPLES);
        }
        if self.fade_pos >= FADE_SAMPLES && out.iter().all(|&s| s == 0) {
            PullStatus::Faded
        } else {
            PullStatus::Plc
        }
    }

    /// Decodes (or recovers / conceals) the frame at `play_ts`. False when the burst is over.
    fn fetch(&mut self, dec: &mut dyn VoiceDecoder) -> bool {
        self.expanding = false;
        let ts = self.play_ts;
        let ticks = u32::from(self.frame.ticks());
        let n = self.frame.samples();
        let found = self.find(ts);
        let next = if self.lbrr {
            self.find(ts + ticks)
        } else {
            None
        };
        let starved = found.is_none()
            && next.is_none()
            && !self.has_frame_after(ts)
            && self.stretch_us < MAX_STRETCH_US;
        let pcm = &mut self.pcm[..n];
        let kind = if let Some(i) = found {
            self.slots[i].valid = false;
            match dec.decode(self.slots[i].frame.opus(), pcm) {
                Ok(_) => Held::Decoded,
                Err(_) => conceal(dec, pcm),
            }
        } else {
            if self.stop_pending && self.max_end <= ts {
                return false;
            }
            if starved {
                return self.expand(dec);
            }
            match next {
                Some(j) => match dec.decode_fec(self.slots[j].frame.opus(), pcm) {
                    Ok(_) => Held::Fec,
                    Err(_) => conceal(dec, pcm),
                },
                None => conceal(dec, pcm),
            }
        };
        match kind {
            Held::Decoded => {
                self.stats.decoded += 1;
                self.reset_loss();
            }
            Held::Fec => {
                self.stats.fec_recovered += 1;
                self.reset_loss();
            }
            Held::Concealed => self.stats.concealed += 1,
        }
        self.held = kind;
        self.pcm_blocks = self.frame.blocks();
        self.pcm_pos = 0;
        true
    }

    /// Starvation: conceals one 10 ms block that does not consume the timeline, so the
    /// late frame at `play_ts` still plays when it arrives.
    fn expand(&mut self, dec: &mut dyn VoiceDecoder) -> bool {
        conceal(dec, &mut self.pcm[..BLOCK_SAMPLES]);
        self.stats.expanded += 1;
        self.stretch_us += BLOCK_US;
        self.expanding = true;
        self.held = Held::Concealed;
        self.pcm_blocks = 1;
        self.pcm_pos = 0;
        true
    }

    fn reset_loss(&mut self) {
        self.loss_us = 0;
        self.fade_pos = 0;
    }

    fn should_drop(&self, now_us: u64, level_db: f32) -> bool {
        if self.buffered_us() <= self.target_us + DROP_HEADROOM_US {
            return false;
        }
        if self
            .last_drop_us
            .is_some_and(|t| now_us.saturating_sub(t) < DROP_SPACING_US)
        {
            return false;
        }
        let floor = self.floor_db.unwrap_or(f32::NEG_INFINITY);
        level_db < DROP_LEVEL_DBFS || level_db < floor + DROP_FLOOR_MARGIN_DB
    }

    fn update_floor(&mut self, level_db: f32) {
        let rise = FLOOR_RISE_DB_PER_S * (BLOCK_US as f32 / 1e6);
        self.floor_db = Some(match self.floor_db {
            Some(f) if level_db >= f => (f + rise).min(level_db),
            _ => level_db,
        });
    }

    fn end_now(&mut self, dec: &mut dyn VoiceDecoder) {
        self.finish();
        let _ = dec.reset();
        self.decoder_reset = false;
    }
}

fn conceal(dec: &mut dyn VoiceDecoder, pcm: &mut [i16]) -> Held {
    if dec.conceal(pcm).is_err() {
        pcm.fill(0);
    }
    Held::Concealed
}

/// Kind of the PCM currently held by the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Held {
    Decoded,
    Fec,
    Concealed,
}
