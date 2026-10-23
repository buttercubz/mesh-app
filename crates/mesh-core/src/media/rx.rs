//! Receive side (DESIGN §6.2-§6.3, §7.5): authenticated MESH records carrying VOICE,
//! TALK, FLOOR_REJECT and PATH_HINT. Duplicates are dropped by `(origin, sess, seq)`;
//! the floor table ranks every burst from the first packet this node saw (age +
//! burst-relative time, no wall clock); VOICE goes to the jitter buffer of its
//! `(origin, sess)` stream, which resets its timeline on a new burst; the mixer takes
//! the ≤ 4 best-ranked streams. A burst outside the global top `group_cap` heard within
//! two hops gets one near-talker FLOOR_REJECT{OUTRANKED} and is not played.

use mesh_api::TalkDeniedReason;
use mesh_routing::{BurstKey, DedupVerdict, FloorEvent};
use mesh_voice::fsm::RejectReason;
use mesh_voice::{StreamKey, TalkInput, TalkState, VoiceFrame};
use mesh_wire::mesh::{
    EncoderProfile, FloorReject, FloorRejectReason, MeshPacket, MeshPayload, PacketType,
    TalkState as WireTalkState,
};

use super::PATH_HINT_HOLD_US;
use super::outbox::OutKind;
use super::state::{Arrival, Media, StreamTrack};
use crate::LinkId;
use crate::engine::Ctx;

fn reject_reason(r: FloorRejectReason) -> RejectReason {
    match r {
        FloorRejectReason::Cap => RejectReason::Cap,
        FloorRejectReason::Preempted => RejectReason::Preempted,
        FloorRejectReason::Congestion => RejectReason::Congestion,
        FloorRejectReason::Outranked => RejectReason::Outranked,
    }
}

fn denied_reason(r: FloorRejectReason) -> TalkDeniedReason {
    match r {
        FloorRejectReason::Preempted => TalkDeniedReason::Preempted,
        FloorRejectReason::Congestion => TalkDeniedReason::Congestion,
        FloorRejectReason::Cap | FloorRejectReason::Outranked => TalkDeniedReason::Cap,
    }
}

impl Media {
    /// A MESH record body from group link `link` (authenticated by the link layer).
    pub(super) fn on_mesh(&mut self, ctx: &mut Ctx, link: LinkId, body: &[u8]) {
        if !self.in_session {
            return;
        }
        let Ok(pkt) = MeshPacket::decode(body) else {
            return;
        };
        let h = pkt.header;
        if !matches!(
            h.ptype,
            PacketType::Voice | PacketType::Talk | PacketType::FloorReject | PacketType::PathHint
        ) {
            return;
        }
        let me = self.identity(ctx);
        if h.origin == me.node && h.sess == ctx.sess {
            return;
        }
        let now = ctx.now;
        if self
            .dedup
            .check_and_mark(now.routing(), h.origin, h.sess, h.seq)
            != DedupVerdict::New
        {
            return;
        }
        let Ok(payload) = MeshPayload::decode(&pkt) else {
            return;
        };
        // Age at arrival: the header age plus this link's one-way estimate (RTT/2).
        let owd = ctx.links.get(link).map_or(0, |l| l.owd_us());
        let age_us = h.age_us() + owd;
        let hops = h.hops.saturating_add(1);
        match payload {
            MeshPayload::Voice(v) => {
                let key = BurstKey::new(h.origin, h.sess, v.burst);
                let ev = self
                    .floor
                    .on_voice(now.routing(), key, h.leader, v.ts, age_us);
                self.floor_events(ctx, &ev);
                if !self.floor.get(key).is_some_and(|e| e.live()) {
                    return;
                }
                let cap = self.group_cap(ctx);
                if self.floor.near_talker_reject(key, h.hops, cap) {
                    self.outbox.push(
                        now,
                        OutKind::Reject(FloorReject {
                            target: h.origin,
                            target_sess: h.sess,
                            burst: v.burst,
                            reason: FloorRejectReason::Outranked,
                            at: me.node,
                        }),
                    );
                    self.diag_reject(ctx, h.origin, FloorRejectReason::Outranked);
                    let ev: Vec<FloorEvent> = self.floor.on_reject(key).into_iter().collect();
                    if let Some(r) = self.recv.as_mut() {
                        r.talk_stop(
                            StreamKey {
                                origin: h.origin,
                                sess: h.sess,
                            },
                            v.burst,
                        );
                    }
                    self.floor_events(ctx, &ev);
                    self.update_ranks();
                    self.ui_dirty = true;
                    return;
                }
                let Some(frame) =
                    VoiceFrame::from_parts(v.burst, v.ts, mesh_voice::VoiceCfg(v.cfg.0), v.opus)
                else {
                    return;
                };
                let skey = StreamKey {
                    origin: h.origin,
                    sess: h.sess,
                };
                if let Some(r) = self.recv.as_mut() {
                    r.push(now.us(), skey, &frame);
                }
                self.track_arrival(
                    skey,
                    Arrival {
                        valid: true,
                        burst: v.burst,
                        ts: v.ts,
                        at: now,
                        age_us,
                        dur_us: u64::from(v.cfg.dur().ms()) * 1_000,
                        hops,
                    },
                );
                self.update_ranks();
                self.ui_dirty = true;
            }
            MeshPayload::Talk(t) => {
                let key = BurstKey::new(h.origin, h.sess, t.burst);
                let ev = self.floor.on_talk(now.routing(), key, &t, age_us);
                if t.state == WireTalkState::Stop
                    && let Some(r) = self.recv.as_mut()
                {
                    r.talk_stop(
                        StreamKey {
                            origin: h.origin,
                            sess: h.sess,
                        },
                        t.burst,
                    );
                }
                self.floor_events(ctx, &ev);
                self.update_ranks();
                self.ui_dirty = true;
            }
            MeshPayload::FloorReject(r) if r.target == me.node && r.target_sess == ctx.sess => {
                self.reject_me(ctx, r.burst, r.reason);
            }
            MeshPayload::FloorReject(r) => {
                let key = BurstKey::new(r.target, r.target_sess, r.burst);
                let ev: Vec<FloorEvent> = self.floor.on_reject(key).into_iter().collect();
                if let Some(rp) = self.recv.as_mut() {
                    rp.talk_stop(
                        StreamKey {
                            origin: r.target,
                            sess: r.target_sess,
                        },
                        r.burst,
                    );
                }
                self.floor_events(ctx, &ev);
                self.update_ranks();
                self.ui_dirty = true;
            }
            MeshPayload::PathHint(p)
                if p.profile == EncoderProfile::Ble12 && (p.target == 0 || p.target == me.node) =>
            {
                self.path_hint_until = Some(now.plus_us(PATH_HINT_HOLD_US));
            }
            _ => {}
        }
    }

    /// `floor_reject(me, burst)` (§6.4): a FLOOR_REJECT that targets my burst, or my own
    /// link refusing it (the mesh glue's relay admission, §6.3,
    /// [`super::MediaRequest::FloorRejected`]). The burst is tombstoned and the talk FSM
    /// stops TX and goes BLOCKED.
    pub(super) fn reject_me(&mut self, ctx: &mut Ctx, burst: u8, reason: FloorRejectReason) {
        let me = self.identity(ctx);
        let key = BurstKey::new(me.node, ctx.sess, burst);
        self.diag_reject(ctx, me.node, reason);
        let ev: Vec<FloorEvent> = self.floor.on_reject(key).into_iter().collect();
        self.last_reject = Some(denied_reason(reason));
        self.talk_input(
            ctx,
            TalkInput::FloorReject {
                burst,
                reason: reject_reason(reason),
            },
        );
        self.floor_events(ctx, &ev);
        self.update_ranks();
        self.ui_dirty = true;
    }

    /// Floor table changes: a freed slot lets a blocked talker go (§6.4).
    pub(super) fn floor_events(&mut self, ctx: &mut Ctx, ev: &[FloorEvent]) {
        self.diag_floor(ctx, ev);
        if !ev.iter().any(|e| matches!(e, FloorEvent::Ended(..))) {
            return;
        }
        if self.fsm.state() != TalkState::Blocked || !self.live {
            return;
        }
        let me = self.identity(ctx);
        let prio = if me.leader && me.leader_priority {
            2
        } else {
            1
        };
        let key = BurstKey::new(me.node, ctx.sess, self.next_burst);
        let cap = self.cap_local(ctx);
        if self.floor.local_admit(ctx.now.routing(), key, prio, cap) {
            self.talk_input(ctx, TalkInput::SlotFreed);
        }
    }

    /// Floor idle release (timer path).
    pub(super) fn floor_timer(&mut self, ctx: &mut Ctx) {
        let ev = self.floor.expire(ctx.now.routing());
        if !ev.is_empty() {
            self.floor_events(ctx, &ev);
            self.update_ranks();
            self.ui_dirty = true;
        }
        self.dedup.expire(ctx.now.routing());
    }

    /// Mix order follows the floor rank (0 = best).
    pub(super) fn update_ranks(&mut self) {
        let Some(r) = self.recv.as_mut() else {
            return;
        };
        for (i, e) in self.floor.ranked().into_iter().enumerate() {
            r.set_rank(
                StreamKey {
                    origin: e.key.origin,
                    sess: e.key.sess,
                },
                i as u32,
            );
        }
    }

    fn track_arrival(&mut self, key: StreamKey, a: Arrival) {
        let recv = self.recv.as_ref();
        let idle = |k: Option<StreamKey>| {
            k.is_none_or(|k| recv.and_then(|r| r.jitter(k)).is_none_or(|j| j.is_idle()))
        };
        let i = match self.streams.iter().position(|s| s.key == Some(key)) {
            Some(i) => i,
            None => match self.streams.iter().position(|s| idle(s.key)) {
                Some(i) => {
                    self.streams[i] = StreamTrack::EMPTY;
                    self.streams[i].key = Some(key);
                    i
                }
                None => return,
            },
        };
        let drops = recv
            .and_then(|r| r.jitter(key))
            .map_or(0, |j| j.stats().dropped_subblocks);
        let s = &mut self.streams[i];
        if s.burst != Some(a.burst) {
            s.burst = Some(a.burst);
            s.arrivals = StreamTrack::EMPTY.arrivals;
            s.next = 0;
            s.play_ts = None;
            s.drops = drops;
        }
        s.hops = a.hops;
        s.arrivals[s.next] = a;
        s.next = (s.next + 1) % s.arrivals.len();
    }
}
