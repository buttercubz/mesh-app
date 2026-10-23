//! Originated packets (VOICE, TALK, FLOOR_REJECT): a fixed ring filled on the
//! 10 ms paths without allocating, flushed from the timer path (the engine's
//! `poll_timeout` returns "now" while it is non-empty) into MESH packets handed to the
//! mesh glue ([`Ctx::originate`]): it sends them to every neighbor through my own
//! per-link admission (an own burst occupies a slot, §6.3), the per-neighbor bundle
//! choice (§9.7) and the paced staging queues (§5.6). Every `seq` comes from the
//! engine's shared per-origin counter ([`Ctx::next_mesh_seq`], §5.3).
//!
//! The origin sets `age` to the queueing delay so far (§4.5; the glue adds its staging
//! residence at dequeue); voice older than `VOICE_MAX_AGE` is dropped, and a voice
//! packet carries the same deadline (§5.6: voice is dropped in the core, never in an
//! OS buffer).

use mesh_voice::VoiceFrame;
use mesh_wire::mesh::{
    FloorReject, MeshHeader, MeshPayload, PacketType, Talk, Voice, VoiceCfg, add_age, encode_packet,
};

use super::bus::MediaIdentity;
use crate::Instant;
use crate::engine::{Ctx, OriginPacket};
use crate::links::Prio;

/// Ring capacity: 4 voice frames + their TALK packets with room to spare.
pub const OUTBOX_CAP: usize = 16;

/// What an outbox slot carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutKind {
    Voice,
    Talk(Talk),
    Reject(FloorReject),
}

#[derive(Clone)]
struct Slot {
    used: bool,
    created: Instant,
    kind: OutKind,
    frame: VoiceFrame,
}

/// The ring.
pub(super) struct Outbox {
    slots: Box<[Slot]>,
    head: usize,
    len: usize,
    dropped: u32,
    /// VOICE frames queued (cumulative, diagnostics).
    voice_frames: u64,
}

impl Outbox {
    pub fn new() -> Self {
        Outbox {
            slots: (0..OUTBOX_CAP)
                .map(|_| Slot {
                    used: false,
                    created: Instant::ZERO,
                    kind: OutKind::Voice,
                    frame: VoiceFrame::empty(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            head: 0,
            len: 0,
            dropped: 0,
            voice_frames: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Packets dropped because the ring was full.
    #[cfg(test)]
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    pub fn clear(&mut self) {
        for s in self.slots.iter_mut() {
            s.used = false;
        }
        self.head = 0;
        self.len = 0;
    }

    fn push_slot(&mut self, created: Instant, kind: OutKind, frame: Option<&VoiceFrame>) {
        if self.len == OUTBOX_CAP {
            // Full (the timer path stalled): the oldest packet goes.
            self.slots[self.head].used = false;
            self.head = (self.head + 1) % OUTBOX_CAP;
            self.len -= 1;
            self.dropped += 1;
        }
        let i = (self.head + self.len) % OUTBOX_CAP;
        let s = &mut self.slots[i];
        s.used = true;
        s.created = created;
        s.kind = kind;
        if let Some(f) = frame {
            s.frame.clone_from(f);
        }
        self.len += 1;
    }

    /// Queues a VOICE frame encoded at `created` (no allocation).
    pub fn push_voice(&mut self, created: Instant, f: &VoiceFrame) {
        self.voice_frames = self.voice_frames.wrapping_add(1);
        self.push_slot(created, OutKind::Voice, Some(f));
    }

    /// VOICE frames queued so far (cumulative).
    pub fn voice_frames(&self) -> u64 {
        self.voice_frames
    }

    /// Queues a control packet.
    pub fn push(&mut self, created: Instant, kind: OutKind) {
        self.push_slot(created, kind, None);
    }

    /// Iterates queued packets oldest first (tests).
    #[cfg(test)]
    pub fn kinds(&self) -> impl Iterator<Item = OutKind> + '_ {
        (0..self.len).map(move |k| self.slots[(self.head + k) % OUTBOX_CAP].kind)
    }

    /// Originates every queued packet ([`Ctx::originate`]). Returns the number of
    /// packets handed over.
    pub fn flush(&mut self, ctx: &mut Ctx, me: MediaIdentity) -> usize {
        if self.len == 0 {
            return 0;
        }
        let now = ctx.now;
        let lim = ctx.cfg.limits.routing;
        let mut sent = 0;
        while self.len > 0 {
            let i = self.head;
            self.head = (self.head + 1) % OUTBOX_CAP;
            self.len -= 1;
            let s = &mut self.slots[i];
            s.used = false;
            let waited = now.since(s.created);
            let (ptype, prio, deadline) = match s.kind {
                OutKind::Voice => {
                    if waited > lim.voice_max_age_us {
                        continue;
                    }
                    (
                        PacketType::Voice,
                        Prio::P1,
                        Some(s.created.plus_us(lim.voice_max_age_us)),
                    )
                }
                OutKind::Talk(_) => (PacketType::Talk, Prio::P0, None),
                OutKind::Reject(_) => (PacketType::FloorReject, Prio::P0, None),
            };
            let mut h = MeshHeader::new(ptype, me.node, ctx.sess, ctx.next_mesh_seq());
            h.leader = me.leader;
            h.age = add_age(0, waited);
            match s.kind {
                OutKind::Voice => h.ttl = lim.voice_ttl,
                OutKind::Reject(_) => h.ttl = mesh_routing::consts::NEAR_TALKER_REJECT_TTL,
                _ => {}
            }
            let body = match s.kind {
                OutKind::Voice => encode_packet(
                    &h,
                    &MeshPayload::Voice(Voice {
                        burst: s.frame.burst,
                        ts: s.frame.ts,
                        cfg: VoiceCfg(s.frame.cfg.0),
                        opus: s.frame.opus(),
                    }),
                ),
                OutKind::Talk(t) => encode_packet(&h, &MeshPayload::Talk(t)),
                OutKind::Reject(r) => encode_packet(&h, &MeshPayload::FloorReject(r)),
            };
            let Ok(body) = body else {
                continue;
            };
            ctx.originate(OriginPacket {
                prio,
                body,
                deadline,
            });
            sent += 1;
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_wire::mesh::{EncoderProfile, TalkMode, TalkPrio, TalkState};

    #[test]
    fn ring_drops_oldest_when_full() {
        let mut o = Outbox::new();
        let t = Talk {
            burst: 1,
            state: TalkState::Start,
            prio: TalkPrio::Member,
            mode: TalkMode::Ptt,
            profile: EncoderProfile::Ble16,
            elapsed_10ms: 0,
        };
        for _ in 0..OUTBOX_CAP + 2 {
            o.push(Instant::ZERO, OutKind::Talk(t));
        }
        assert_eq!(o.len(), OUTBOX_CAP);
        assert_eq!(o.dropped(), 2);
        o.push_voice(Instant(5), &VoiceFrame::empty());
        assert_eq!(o.kinds().last(), Some(OutKind::Voice));
        o.clear();
        assert!(o.is_empty());
    }
}
