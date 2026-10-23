//! Inbound side of the router: validation, penalties, dedup, floor updates, delivery and
//! the forwarding decision (DESIGN §5.2).

use mesh_wire::mesh::{
    FloorReject, FloorRejectReason, MeshHeader, MeshPacket, MeshPayload, PacketType, PathHint,
    TrafficClass,
};

use super::{Authenticated, RouteOutcome, Router, RouterAction};
use crate::consts::{PENALTY_MALFORMED, PENALTY_ORIGIN_SPOOF};
use crate::dedup::DedupVerdict;
use crate::floor::{BurstKey, FloorEvent};
use crate::types::{Instant, LinkId, NodeId};

impl Router {
    /// Routes one authenticated MESH record received on `link`.
    pub fn on_mesh_packet(
        &mut self,
        now: Instant,
        link: LinkId,
        pkt: Authenticated<'_>,
    ) -> RouteOutcome {
        let mut out = RouteOutcome::default();
        let Some(n) = self.neighbors.node_of(link) else {
            self.stats.unknown_link += 1;
            return out;
        };
        let decoded =
            MeshPacket::decode(pkt.bytes()).and_then(|p| MeshPayload::decode(&p).map(|pl| (p, pl)));
        let (p, payload) = match decoded {
            Ok(v) => v,
            Err(_) => {
                self.stats.malformed += 1;
                self.penalize(now, n, PENALTY_MALFORMED, &mut out);
                return out;
            }
        };
        let h = p.header;
        if h.hops == 0 && h.origin != n {
            self.stats.spoofed += 1;
            self.penalize(now, n, PENALTY_ORIGIN_SPOOF, &mut out);
            return out;
        }
        match self.dedup.check_and_mark(now, h.origin, h.sess, h.seq) {
            DedupVerdict::New => {}
            DedupVerdict::Duplicate => {
                self.stats.dup += 1;
                let key = h.dedup_key();
                if let Some(pc) = self.pending.iter_mut().find(|pc| pc.key == key) {
                    pc.heard_from.insert(n);
                }
                return out;
            }
            DedupVerdict::TooOld => {
                self.stats.too_old += 1;
                return out;
            }
        }
        let owd = self
            .neighbors
            .link(link)
            .map_or(0, |l| l.quality.rtt.owd_us());
        let age_now = h.age_us() + owd;
        self.local_effects(now, &h, &payload, age_now, &mut out);
        if !matches!(h.ptype, PacketType::Unknown(_) | PacketType::Alert) {
            self.stats.delivered += 1;
            out.deliver = Some(h);
        }
        if !self.cfg.multi_hop || h.ttl <= 1 {
            return out;
        }
        if h.class == TrafficClass::Voice && age_now > self.cfg.limits.voice_max_age_us {
            self.stats.stale += 1;
            return out;
        }
        let mut relayed_payload = p.payload.to_vec();
        if let MeshPayload::MeshProbe(mut probe) = payload {
            // Relays append their node_id to the trace (n ≤ 8; full traces pass as-is).
            if probe.append(self.cfg.me).is_ok()
                && let Ok(b) = probe.encode()
            {
                relayed_payload = b;
            }
        }
        let Some(rh) = h.relayed(0) else {
            return out;
        };
        let burst = match payload {
            MeshPayload::Voice(v) => Some(BurstKey::new(h.origin, h.sess, v.burst)),
            _ => None,
        };
        self.forward(now, n, rh, relayed_payload, age_now, burst, &mut out);
        out
    }

    fn penalize(&mut self, now: Instant, n: NodeId, points: u32, out: &mut RouteOutcome) {
        if self.neighbors.penalize(now, n, points)
            && let Some(until) = self.neighbors.banned_until(now, n)
        {
            out.actions
                .push(RouterAction::Disconnect { node: n, until });
        }
    }

    /// Floor table, presence and PATH_HINT effects of a new packet.
    fn local_effects(
        &mut self,
        now: Instant,
        h: &MeshHeader,
        payload: &MeshPayload<'_>,
        age_now: u64,
        out: &mut RouteOutcome,
    ) {
        match payload {
            MeshPayload::Voice(v) => {
                let key = BurstKey::new(h.origin, h.sess, v.burst);
                let ev = self.floor.on_voice(now, key, h.leader, v.ts, age_now);
                self.push_floor_events(ev, out);
                let group_cap = self.cfg.limits.group_cap(self.group_has_ble);
                if h.origin != self.cfg.me && self.floor.near_talker_reject(key, h.hops, group_cap)
                {
                    self.originate_reject(now, key, FloorRejectReason::Outranked, true, out);
                }
            }
            MeshPayload::Talk(t) => {
                let key = BurstKey::new(h.origin, h.sess, t.burst);
                let ev = self.floor.on_talk(now, key, t, age_now);
                self.push_floor_events(ev, out);
            }
            MeshPayload::FloorReject(r) => self.apply_reject(r, out),
            MeshPayload::Presence(_) => self.dedup.on_presence(h.origin, h.sess),
            MeshPayload::PathHint(PathHint {
                profile, target, ..
            }) if (*target == PathHint::ALL || *target == self.cfg.me) => {
                out.actions.push(RouterAction::PathHint {
                    profile: *profile,
                    from: h.origin,
                });
            }
            _ => {}
        }
    }

    /// A FLOOR_REJECT (received, or flooded by me): tombstone the burst everywhere.
    pub(super) fn apply_reject(&mut self, r: &FloorReject, out: &mut RouteOutcome) {
        let key = BurstKey::new(r.target, r.target_sess, r.burst);
        if let Some(ev) = self.floor.on_reject(key) {
            self.push_floor_events(vec![ev], out);
        }
        if r.target == self.cfg.me && r.target_sess == self.cfg.sess {
            out.actions.push(RouterAction::RejectedMe {
                burst: r.burst,
                reason: r.reason,
            });
        }
    }

    pub(super) fn push_floor_events(&mut self, ev: Vec<FloorEvent>, out: &mut RouteOutcome) {
        for e in ev {
            if let FloorEvent::Ended(k, _) = e {
                self.admission.on_burst_ended(k);
            }
            out.actions.push(RouterAction::Floor(e));
        }
    }
}
