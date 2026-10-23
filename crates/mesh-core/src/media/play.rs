//! Playout (DESIGN §7.5-§7.6, §12.6): every 10 ms the jitter buffers are pulled, the
//! ≤ 4 best-ranked streams are mixed (−3 dB each when ≥ 2) with the earcons, the spoken
//! prompts and the audio-check signal *before* the limiter (so every cue sits inside the
//! AEC reference), and each first-block playout of a received frame closes one latency
//! estimate. No allocation.

use mesh_routing::BurstKey;
use mesh_voice::{BLOCK_SAMPLES, Block, rms_linear};

/// Streams mixed at most (§6.3, §7.6).
const MIX: usize = mesh_routing::consts::MIX_STREAMS;

use super::latency::{LatencySample, estimate_us};
use super::state::{BLOCK, Media};
use crate::Instant;
use crate::engine::Ctx;

impl Media {
    /// `true` when this module has something to play (or must keep pulling streams).
    fn playout_active(&self) -> bool {
        self.in_session
            || self.check.is_some()
            || !self.earcons.is_idle()
            || self.player.is_playing()
    }

    /// 10 ms of playout mixed into `out` (whatever earlier modules wrote is kept, as a cue).
    pub(super) fn on_playout(&mut self, ctx: &mut Ctx, out: &mut Block) {
        if ctx.is_lab() || !self.playout_active() {
            return;
        }
        let now = ctx.now;
        let base = *out;
        let base_on = base.iter().any(|&s| s != 0);
        // A prompt starts once the cue's earcon has finished (earcon, then words).
        let may_speak =
            self.prompts.enabled && (self.player.is_playing() || self.earcons.is_idle());
        let mut ear = [0i16; BLOCK_SAMPLES];
        let ear_on = self.earcons.render(&mut ear);
        let mut spoken = [0i16; BLOCK_SAMPLES];
        let spoken_on = may_speak && self.player.render(now.us(), &self.bank, &mut spoken);
        let mut check = [0i16; BLOCK_SAMPLES];
        let check_on = self
            .check
            .as_mut()
            .is_some_and(|c| c.pop_output(&mut check));
        let mut cues: [&Block; 4] = [&base; 4];
        let mut n = 0;
        for (on, b) in [
            (base_on, &base),
            (ear_on, &ear),
            (spoken_on, &spoken),
            (check_on, &check),
        ] {
            if on {
                cues[n] = b;
                n += 1;
            }
        }
        match self.recv.as_mut() {
            Some(r) if self.in_session => {
                r.pull(now.us());
                for s in self.streams.iter_mut() {
                    s.level = 0.0;
                }
                for (key, pcm) in r.selected() {
                    if let Some(s) = self.streams.iter_mut().find(|s| s.key == Some(key)) {
                        s.level = rms_linear(pcm);
                    }
                }
                // A burst rejected after its first frames were buffered (near-talker
                // or received FLOOR_REJECT) is never mixed again (§6.2).
                let floor = &self.floor;
                let mut streams: [&Block; MIX] = [&base; MIX];
                let mut k = 0;
                for (key, pcm) in r.selected() {
                    let burst = r.jitter(key).and_then(|j| j.current_burst());
                    let rejected = burst.is_some_and(|b| {
                        floor
                            .get(BurstKey::new(key.origin, key.sess, b))
                            .is_some_and(|e| !e.live())
                    });
                    if !rejected && k < MIX {
                        streams[k] = pcm;
                        k += 1;
                    }
                }
                self.mixer.mix(&streams[..k], &cues[..n], out);
            }
            _ => self.mixer.mix(&[], &cues[..n], out),
        }
        if self.in_session {
            self.track_playout(now);
        }
    }

    /// Advances each stream's burst timeline by the block just played and turns the
    /// frame whose first block it was into a latency sample.
    fn track_playout(&mut self, now: Instant) {
        let Some(r) = self.recv.as_ref() else {
            return;
        };
        let lookahead = self.lookahead_us;
        for s in self.streams.iter_mut() {
            let Some(key) = s.key else {
                continue;
            };
            let Some(status) = r.status(key) else {
                continue;
            };
            if !status.is_audio() {
                if !matches!(status, mesh_voice::PullStatus::Buffering) {
                    s.play_ts = None;
                }
                continue;
            }
            let Some(burst) = s.burst else {
                continue;
            };
            let dropped = r
                .jitter(key)
                .map_or(s.drops, |j| j.stats().dropped_subblocks);
            let skipped = dropped.saturating_sub(s.drops);
            s.drops = dropped;
            let mut ts = match s.play_ts {
                Some(t) => t,
                None => s
                    .arrivals
                    .iter()
                    .filter(|a| a.valid && a.burst == burst)
                    .map(|a| u32::from(a.ts))
                    .min()
                    .unwrap_or(0),
            };
            ts += skipped;
            for a in s.arrivals.iter_mut() {
                if !a.valid || a.burst != burst {
                    continue;
                }
                let ats = u32::from(a.ts);
                if ats == ts {
                    let wait = now.since(a.at);
                    self.latency.record(LatencySample {
                        origin: key.origin,
                        hops: a.hops,
                        m2e_us: estimate_us(a.dur_us, lookahead, a.age_us, wait),
                    });
                    a.valid = false;
                } else if ats + (a.dur_us / BLOCK) as u32 <= ts {
                    // Its whole frame went by (late or replaced by concealment).
                    a.valid = false;
                }
            }
            s.play_ts = Some(ts + 1);
        }
    }
}
