//! `ridelog/1` emission of the media module (diagnostics spec §2.4): floor grants,
//! rejects and releases, PTT and VAD edges, jitter-buffer retargets, and the module's
//! part of the 1 Hz core sample (audio state, per-stream jitter buffers and totals).
//!
//! The 10 ms capture path only writes plain fields here (no allocation); the edges are
//! emitted from the timer path. Nothing in this file changes a media decision.

use std::collections::BTreeMap;

use mesh_lab::ridelog::{CoreJb, CoreSample, DiagEvent, FloorKind, JbCounters};
use mesh_routing::floor::{EndReason, FloorEvent};
use mesh_voice::jitter::JitterStats;
use mesh_wire::mesh::FloorRejectReason;

use super::state::Media;
use crate::engine::Ctx;

/// What the media module remembers for diagnostics.
#[derive(Debug, Clone, Default)]
pub(super) struct MediaDiag {
    /// VAD state from the capture path (last `VadEvent`) and the last one reported.
    pub vad: bool,
    pub vad_p: f32,
    vad_reported: bool,
    /// PTT held (hold style) or latched.
    pub ptt: bool,
    /// Jitter-buffer target (ms) per origin at the previous sample.
    jb_target: BTreeMap<u16, u16>,
}

/// Static code of a floor end reason.
pub(super) const fn end_str(r: EndReason) -> &'static str {
    match r {
        EndReason::Stop => "stop",
        EndReason::Rejected => "rejected",
        EndReason::Idle => "idle",
        EndReason::Replaced => "replaced",
        EndReason::Evicted => "evicted",
    }
}

/// Static code of a FLOOR_REJECT reason.
pub(super) const fn reject_str(r: FloorRejectReason) -> &'static str {
    match r {
        FloorRejectReason::Cap => "cap",
        FloorRejectReason::Preempted => "preempted",
        FloorRejectReason::Congestion => "congestion",
        FloorRejectReason::Outranked => "outranked",
    }
}

fn jb_counters(s: &JitterStats) -> JbCounters {
    JbCounters {
        received: u64::from(s.received),
        late: u64::from(s.late),
        duplicates: u64::from(s.duplicates),
        fec_recovered: u64::from(s.fec_recovered),
        concealed: u64::from(s.concealed),
        expanded: u64::from(s.expanded),
        dropped: u64::from(s.dropped_subblocks),
    }
}

fn ms16(us: u64) -> u16 {
    u16::try_from(us / 1_000).unwrap_or(u16::MAX)
}

impl Media {
    /// Live floor entries (the `streams` field of floor events).
    fn live_streams(&self) -> u8 {
        let n = self.floor.entries().iter().filter(|e| e.live()).count();
        u8::try_from(n).unwrap_or(u8::MAX)
    }

    /// Floor table changes: grants and releases.
    pub(super) fn diag_floor(&self, ctx: &mut Ctx, ev: &[FloorEvent]) {
        if !ctx.fx.diag_on() {
            return;
        }
        let streams = self.live_streams();
        for e in ev {
            let (kind, key, reason) = match *e {
                FloorEvent::Started(k) => (FloorKind::Grant, k, None),
                FloorEvent::Ended(k, EndReason::Rejected) => {
                    (FloorKind::Reject, k, Some(end_str(EndReason::Rejected)))
                }
                FloorEvent::Ended(k, r) => (FloorKind::Release, k, Some(end_str(r))),
            };
            ctx.fx.diag(DiagEvent::Floor {
                kind,
                origin: key.origin,
                reason,
                streams,
            });
        }
    }

    /// A FLOOR_REJECT decided here (near-talker) or aimed at my burst.
    pub(super) fn diag_reject(&self, ctx: &mut Ctx, origin: u16, reason: FloorRejectReason) {
        let streams = self.live_streams();
        ctx.fx.diag(DiagEvent::Floor {
            kind: FloorKind::Reject,
            origin,
            reason: Some(reject_str(reason)),
            streams,
        });
    }

    /// Edges recorded by the capture path (timer path, may allocate).
    pub(super) fn diag_timer(&mut self, ctx: &mut Ctx) {
        if !ctx.fx.diag_on() {
            return;
        }
        if self.diag.vad != self.diag.vad_reported {
            self.diag.vad_reported = self.diag.vad;
            ctx.fx.diag(DiagEvent::Vad {
                on: self.diag.vad,
                p: self.diag.vad_p,
            });
        }
    }

    /// The module's part of the 1 Hz core sample, and jitter-buffer retargets.
    pub(super) fn fill_diag(&mut self, ctx: &mut Ctx, s: &mut CoreSample) {
        if let Some(a) = s.audio.as_mut() {
            a.vad = self.diag.vad;
            a.ptt = self.diag.ptt;
            a.tx_frames = self.outbox.voice_frames();
        }
        let Some(recv) = self.recv.as_ref() else {
            self.diag.jb_target.clear();
            return;
        };
        s.jb_total = Some(jb_counters(&recv.totals()));
        let mut targets = BTreeMap::new();
        for t in self.streams.iter() {
            let Some(key) = t.key else { continue };
            let Some(jb) = recv.jitter(key) else { continue };
            let target_ms = ms16(jb.target_delay_us());
            targets.insert(key.origin, target_ms);
            s.jb.push(CoreJb {
                origin: key.origin,
                target_ms,
                depth_ms: ms16(jb.buffered_us()),
                owd_p95_ms: None,
                counters: jb_counters(&jb.stats()),
            });
        }
        for (origin, to_ms) in &targets {
            if let Some(from_ms) = self.diag.jb_target.get(origin)
                && from_ms != to_ms
            {
                ctx.fx.diag(DiagEvent::JbRetarget {
                    origin: *origin,
                    from_ms: *from_ms,
                    to_ms: *to_ms,
                    why: if to_ms > from_ms { "grow" } else { "shrink" },
                });
            }
        }
        self.diag.jb_target = targets;
    }
}
