//! My own MESH packets (DESIGN §5.2 origination, §6.3, §9.7).
//!
//! The group and media modules hand what they originate to the glue through
//! `Ctx::originate`; it leaves exactly like a relayed packet: to **every** neighbor (the
//! origin never prunes, §5.2), on the link that neighbor's bundle picks (Aware or BLE,
//! both only during the 1 s overlap), through that link's paced staging queue (strict
//! priority, voice backlog bound, stream-level shedding, the node airtime budget). Two
//! origin rules on top:
//!
//! - my TALK, VOICE and FLOOR_REJECT update the relay floor table like anyone else's,
//!   and my VOICE goes through my own per-link admission: an own burst occupies a slot
//!   on every link that carries it, so a talking relay admits at most `cap − 1` relayed
//!   bursts there (§6.3), and a lower-ranked relayed burst admitted within `T_STICKY`
//!   is evicted with a FLOOR_REJECT. If one of my links refuses my burst (it already
//!   carries `cap` bursts that outrank it), the frame goes nowhere and the media module
//!   is told (`MediaRequest::FloorRejected`): the talk FSM goes BLOCKED (§6.4). This is
//!   the router's own-origination rule (`Router::originate` for VOICE);
//! - packets the router builds itself (relay FLOOR_REJECT, PATH_HINT) carry its private
//!   counter; they take the engine's shared per-origin `seq` when they are staged
//!   ([`Mesh::adopt_seq`]), so every packet of `(me, sess)` lives in one dedup window at
//!   every receiver (§5.3).

use mesh_routing::{AdmitDecision, BurstKey, FloorEvent, Outgoing, Priority, QueuedPacket};
use mesh_wire::mesh::{FloorReject, FloorRejectReason, MeshPacket, MeshPayload, PacketType};

use super::state::Mesh;
use crate::engine::{Ctx, OriginPacket};
use crate::links::Prio;
use crate::media::MediaRequest;

fn priority(p: Prio) -> Priority {
    match p {
        Prio::P0 => Priority::P0,
        Prio::P1 => Priority::P1,
        Prio::P2 => Priority::P2,
        Prio::P3 => Priority::P3,
    }
}

impl Mesh {
    /// Sends one packet I originated to every neighbor.
    pub fn originate_own(&mut self, ctx: &mut Ctx, op: OriginPacket) {
        let now = ctx.now.routing();
        let Ok(p) = MeshPacket::decode(&op.body) else {
            return;
        };
        let Ok(payload) = MeshPayload::decode(&p) else {
            return;
        };
        let h = p.header;
        if h.origin != self.id.node || h.hops != 0 {
            // Neighbors would refuse it (first-hop origin check, §5.2) and penalize me.
            self.stats.foreign_origin += 1;
            return;
        }
        self.stats.originated += 1;
        let mut burst = None;
        let events: Vec<FloorEvent> = match payload {
            MeshPayload::Voice(v) => {
                let key = BurstKey::new(h.origin, h.sess, v.burst);
                burst = Some(key);
                self.router.floor.on_voice(now, key, h.leader, v.ts, 0)
            }
            MeshPayload::Talk(t) => {
                let key = BurstKey::new(h.origin, h.sess, t.burst);
                self.router.floor.on_talk(now, key, &t, 0)
            }
            MeshPayload::FloorReject(r) => {
                let key = BurstKey::new(r.target, r.target_sess, r.burst);
                self.router.floor.on_reject(key).into_iter().collect()
            }
            _ => Vec::new(),
        };
        self.floor_ended(&events);
        let packet = QueuedPacket {
            header: h,
            payload: p.payload.to_vec(),
            enqueued_at: now,
            // The origin's queueing delay so far; the staging residence is added at
            // dequeue (§4.5).
            base_age_us: h.age_us(),
            burst,
        };
        let prio = priority(op.prio);
        let targets: Vec<mesh_routing::LinkId> = self
            .router
            .neighbors
            .neighbors()
            .filter_map(|n| n.best_link(now).map(|l| l.id))
            .collect();
        match burst {
            Some(b) => self.own_voice(ctx, b, &targets, prio, packet),
            None => {
                for link in targets {
                    let o = Outgoing {
                        link,
                        prio,
                        packet: packet.clone(),
                    };
                    self.schedule(ctx, o);
                }
            }
        }
        self.drop_unadmitted();
    }

    /// My own VOICE on each neighbor's link through my per-link admission.
    fn own_voice(
        &mut self,
        ctx: &mut Ctx,
        b: BurstKey,
        targets: &[mesh_routing::LinkId],
        prio: Priority,
        packet: QueuedPacket,
    ) {
        let now = ctx.now.routing();
        let mut admitted = Vec::new();
        let mut evicted: Vec<(BurstKey, FloorRejectReason)> = Vec::new();
        let mut refused: Option<FloorRejectReason> = None;
        for &link in targets {
            let cap = self.router.link_cap(link);
            let congestion = self
                .router
                .neighbors
                .link(link)
                .is_some_and(|s| cap < self.router.cfg.limits.s_max(s.kind));
            let d =
                self.router
                    .admission
                    .admits(now, link, b, &mut self.router.floor, cap, congestion);
            match d {
                AdmitDecision::Admitted => admitted.push(link),
                AdmitDecision::AdmittedEvicting { victim, reason } => {
                    evicted.push((victim, reason));
                    admitted.push(link);
                }
                AdmitDecision::Refused { reject } => {
                    if let Some(r) = reject {
                        refused.get_or_insert(r);
                    }
                }
            }
        }
        for (victim, reason) in evicted {
            self.reject_burst(ctx, victim, reason);
        }
        if let Some(reason) = refused {
            // My own link refused my burst: stop locally, nothing to flood.
            self.stats.own_refused += 1;
            let ev: Vec<FloorEvent> = self.router.floor.on_reject(b).into_iter().collect();
            self.floor_ended(&ev);
            ctx.post_media(MediaRequest::FloorRejected {
                burst: b.burst,
                reason,
            });
            return;
        }
        for link in admitted {
            self.router.neighbors.mark_voice_tx(now, link);
            let o = Outgoing {
                link,
                prio,
                packet: packet.clone(),
            };
            self.schedule(ctx, o);
        }
    }

    /// Floods `FLOOR_REJECT{victim, reason, at = me}`: my own burst took its slot.
    fn reject_burst(&mut self, ctx: &mut Ctx, victim: BurstKey, reason: FloorRejectReason) {
        let r = FloorReject {
            target: victim.origin,
            target_sess: victim.sess,
            burst: victim.burst,
            reason,
            at: self.id.node,
        };
        let out = self.router.originate(
            ctx.now.routing(),
            PacketType::FloorReject,
            false,
            r.encode(),
        );
        self.apply(ctx, out);
    }

    /// Ended bursts free their admission slots.
    fn floor_ended(&mut self, ev: &[FloorEvent]) {
        for e in ev {
            if let FloorEvent::Ended(k, _) = e {
                self.router.admission.on_burst_ended(*k);
            }
        }
    }

    /// A packet the router originated (hops 0, my origin) takes the next `seq` of the
    /// engine's shared per-origin counter; its copies for several links (same router
    /// `seq` within one outcome, listed in `seen`) keep one `seq`.
    pub fn adopt_seq(&self, ctx: &mut Ctx, o: &mut Outgoing, seen: &mut Vec<(u16, u16)>) {
        let h = &mut o.packet.header;
        if h.origin != self.id.node || h.hops != 0 {
            return;
        }
        let shared = match seen.iter().find(|(r, _)| *r == h.seq) {
            Some((_, s)) => *s,
            None => {
                let s = ctx.next_mesh_seq();
                seen.push((h.seq, s));
                s
            }
        };
        h.seq = shared;
    }
}
