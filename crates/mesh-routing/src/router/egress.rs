//! Outbound side of the router: relay fan-out with voice admission, jittered control
//! relays with counter suppression and pruning, and origination (DESIGN §5.2, §6.3).

use rand_core::RngCore;

use mesh_wire::mesh::{
    FloorReject, FloorRejectReason, MeshHeader, MeshPacket, MeshPayload, PacketType, Segment,
    TrafficClass, segment_body,
};

use super::{Outgoing, PendingCtrl, RouteOutcome, Router, RouterAction};
use crate::consts::{
    COUNTER_SUPPRESSION, CTRL_JITTER_MAX_US, CTRL_JITTER_MIN_US, NEAR_TALKER_REJECT_TTL,
};
use crate::floor::{AdmitDecision, BurstKey};
use crate::prune::pruned;
use crate::queue::{Priority, QueuedPacket};
use crate::types::{Instant, LinkId, NodeId};

impl Router {
    /// Best link of every neighbor except `skip`.
    fn targets(&self, now: Instant, skip: &[NodeId]) -> Vec<(NodeId, LinkId)> {
        self.neighbors
            .neighbors()
            .filter(|m| !skip.contains(&m.node_id))
            .filter_map(|m| m.best_link(now).map(|l| (m.node_id, l.id)))
            .collect()
    }

    /// Relay fan-out of a new packet received from `from`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn forward(
        &mut self,
        now: Instant,
        from: NodeId,
        rh: MeshHeader,
        payload: Vec<u8>,
        age_now: u64,
        burst: Option<BurstKey>,
        out: &mut RouteOutcome,
    ) {
        let packet = QueuedPacket {
            header: rh,
            payload,
            enqueued_at: now,
            base_age_us: age_now,
            burst,
        };
        match rh.class {
            TrafficClass::Voice => {
                let targets = self.targets(now, &[from, rh.origin]);
                self.send_voice(now, &targets, packet, false, out);
            }
            TrafficClass::Lab => {
                for (_, link) in self.targets(now, &[from, rh.origin]) {
                    self.stats.forwarded += 1;
                    out.out.push(Outgoing {
                        link,
                        prio: Priority::P3,
                        packet: packet.clone(),
                    });
                }
            }
            TrafficClass::Ctrl | TrafficClass::Bulk => {
                let jitter = CTRL_JITTER_MIN_US
                    + self.rng.next_u64() % (CTRL_JITTER_MAX_US - CTRL_JITTER_MIN_US + 1);
                let seg_msg = if rh.seg {
                    Segment::decode(&packet.payload).ok().map(|s| s.msg_id)
                } else {
                    None
                };
                self.pending.push(PendingCtrl {
                    due: now.plus_us(jitter),
                    key: rh.dedup_key(),
                    from,
                    class: rh.class,
                    seg_msg,
                    packet,
                    heard_from: [from].into_iter().collect(),
                });
            }
        }
    }

    /// Voice on each target link through per-link admission. When any link refuses the
    /// burst with a reason, the burst is rejected (FLOOR_REJECT, tombstone) and this frame
    /// goes nowhere: hard enforcement never lets a doomed frame overfill another link.
    /// BLE edge links of a backbone node refuse without a reason (the burst is shed on
    /// that edge only and keeps flowing on the other links).
    fn send_voice(
        &mut self,
        now: Instant,
        targets: &[(NodeId, LinkId)],
        packet: QueuedPacket,
        own: bool,
        out: &mut RouteOutcome,
    ) {
        let Some(b) = packet.burst else {
            return;
        };
        let mut rejects: Vec<(BurstKey, FloorRejectReason)> = Vec::new();
        let mut admitted: Vec<LinkId> = Vec::new();
        let mut refused_b = None;
        for &(_, link) in targets {
            let cap = self.link_cap(link);
            let congestion = self.congestion(link);
            let edge = self.is_edge_link(link);
            let d = self
                .admission
                .admits_at(now, link, b, &mut self.floor, cap, congestion, edge);
            match d {
                AdmitDecision::Admitted => admitted.push(link),
                AdmitDecision::AdmittedEvicting { victim, reason } => {
                    rejects.push((victim, reason));
                    admitted.push(link);
                }
                AdmitDecision::Refused { reject } => {
                    self.stats.floor_refused += 1;
                    if let Some(r) = reject {
                        refused_b.get_or_insert(r);
                    }
                }
            }
        }
        if let Some(r) = refused_b {
            rejects.push((b, r));
        } else {
            for link in admitted {
                self.neighbors.mark_voice_tx(now, link);
                if own {
                    self.stats.originated += 1;
                } else {
                    self.stats.forwarded += 1;
                }
                out.out.push(Outgoing {
                    link,
                    prio: Priority::P1,
                    packet: packet.clone(),
                });
            }
        }
        for (key, reason) in rejects {
            if key.origin == self.cfg.me && key.sess == self.cfg.sess {
                // My own link refused my burst: stop locally, nothing to flood.
                if let Some(ev) = self.floor.on_reject(key) {
                    self.push_floor_events(vec![ev], out);
                }
                out.actions.push(RouterAction::RejectedMe {
                    burst: key.burst,
                    reason,
                });
            } else {
                self.originate_reject(now, key, reason, false, out);
            }
        }
    }

    /// Floods FLOOR_REJECT{key, reason, at = me} (TTL 6, or 4 for near-talker) and
    /// applies it locally.
    pub(super) fn originate_reject(
        &mut self,
        now: Instant,
        key: BurstKey,
        reason: FloorRejectReason,
        near: bool,
        out: &mut RouteOutcome,
    ) {
        let r = FloorReject {
            target: key.origin,
            target_sess: key.sess,
            burst: key.burst,
            reason,
            at: self.cfg.me,
        };
        self.apply_reject(&r, out);
        let mut h = MeshHeader::new(
            PacketType::FloorReject,
            self.cfg.me,
            self.cfg.sess,
            self.next_seq(),
        );
        if near {
            h.ttl = NEAR_TALKER_REJECT_TTL;
        }
        self.dedup.check_and_mark(now, h.origin, h.sess, h.seq);
        self.stats.rejects_sent += 1;
        let packet = QueuedPacket {
            header: h,
            payload: r.encode(),
            enqueued_at: now,
            base_age_us: 0,
            burst: None,
        };
        for (_, link) in self.targets(now, &[]) {
            out.out.push(Outgoing {
                link,
                prio: Priority::P0,
                packet: packet.clone(),
            });
        }
    }

    /// Releases jittered control relays that are due.
    pub fn poll(&mut self, now: Instant) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let (due, keep): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|p| p.due <= now);
        self.pending = keep;
        for pc in due {
            let suppress = pc.heard_from.len() >= COUNTER_SUPPRESSION;
            let seg_timed_out = pc.seg_msg.is_some_and(|m| {
                self.seg_timeouts
                    .timed_out(now, pc.from, pc.key.0, pc.key.1, m)
            });
            let from = self.neighbors.neighbor(pc.from).cloned();
            for (m, link) in self.targets(now, &[pc.from, pc.key.0]) {
                if suppress && pc.heard_from.contains(&m) {
                    self.stats.suppressed += 1;
                    continue;
                }
                if let Some(n) = &from
                    && pruned(now, pc.class, n, m, seg_timed_out)
                {
                    self.stats.pruned += 1;
                    continue;
                }
                self.stats.forwarded += 1;
                out.push(Outgoing {
                    link,
                    prio: Priority::of_mesh(&pc.packet.header),
                    packet: pc.packet.clone(),
                });
            }
        }
        out
    }

    /// Originates a non-segmented packet to **all** neighbors (no pruning, no jitter).
    /// VOICE goes through my own per-link admission. `payload` is the encoded payload.
    pub fn originate(
        &mut self,
        now: Instant,
        ptype: PacketType,
        leader: bool,
        payload: Vec<u8>,
    ) -> RouteOutcome {
        let mut out = RouteOutcome::default();
        let mut h = MeshHeader::new(ptype, self.cfg.me, self.cfg.sess, self.next_seq());
        h.leader = leader;
        if ptype == PacketType::Voice {
            h.ttl = self.cfg.limits.voice_ttl;
        }
        self.dedup.check_and_mark(now, h.origin, h.sess, h.seq);
        let mut burst = None;
        if let Ok(hb) = h.encode() {
            let mut bytes = hb.to_vec();
            bytes.extend_from_slice(&payload);
            if let Ok(p) = MeshPacket::decode(&bytes)
                && let Ok(pl) = MeshPayload::decode(&p)
            {
                if let MeshPayload::Voice(v) = pl {
                    burst = Some(BurstKey::new(h.origin, h.sess, v.burst));
                }
                self.own_effects(now, &h, &pl, &mut out);
            }
        }
        let packet = QueuedPacket {
            header: h,
            payload,
            enqueued_at: now,
            base_age_us: 0,
            burst,
        };
        let targets = self.targets(now, &[]);
        if ptype == PacketType::Voice {
            self.send_voice(now, &targets, packet, true, &mut out);
        } else {
            for (_, link) in targets {
                self.stats.originated += 1;
                out.out.push(Outgoing {
                    link,
                    prio: Priority::of_mesh(&h),
                    packet: packet.clone(),
                });
            }
        }
        out
    }

    /// Originates a segmented ROSTER/JOIN_REQ/REKEY body: one packet per segment, each
    /// with its own `seq`.
    pub fn originate_segmented(
        &mut self,
        now: Instant,
        ptype: PacketType,
        leader: bool,
        msg_id: u16,
        body: &[u8],
    ) -> RouteOutcome {
        let mut out = RouteOutcome::default();
        let Ok(segs) = segment_body(msg_id, body) else {
            return out;
        };
        for s in segs {
            if let Ok(pl) = s.encode() {
                let r = self.originate(now, ptype, leader, pl);
                out.out.extend(r.out);
                out.actions.extend(r.actions);
            }
        }
        out
    }

    /// My own TALK/VOICE/FLOOR_REJECT update my floor table like anyone else's.
    fn own_effects(
        &mut self,
        now: Instant,
        h: &MeshHeader,
        pl: &MeshPayload<'_>,
        out: &mut RouteOutcome,
    ) {
        match pl {
            MeshPayload::Voice(v) => {
                let key = BurstKey::new(h.origin, h.sess, v.burst);
                let ev = self.floor.on_voice(now, key, h.leader, v.ts, 0);
                self.push_floor_events(ev, out);
            }
            MeshPayload::Talk(t) => {
                let key = BurstKey::new(h.origin, h.sess, t.burst);
                let ev = self.floor.on_talk(now, key, t, 0);
                self.push_floor_events(ev, out);
            }
            MeshPayload::FloorReject(r) => self.apply_reject(r, out),
            _ => {}
        }
    }

    /// Floods `PATH_HINT{BLE12, at = me, target = all}` (TTL 3): one of my links' grant
    /// stayed below `cost(1)` for 3 s (§5.6).
    pub fn originate_path_hint(&mut self, now: Instant) -> RouteOutcome {
        let hint = mesh_wire::mesh::PathHint {
            profile: mesh_wire::mesh::EncoderProfile::Ble12,
            at: self.cfg.me,
            target: mesh_wire::mesh::PathHint::ALL,
        };
        match hint.encode() {
            Ok(p) => self.originate(now, PacketType::PathHint, false, p),
            Err(_) => RouteOutcome::default(),
        }
    }
}
