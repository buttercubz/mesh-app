//! Per-link scheduling of relayed packets and my own (DESIGN §5.6):
//! bundle choice, staging in the per-link strict-priority [`LinkQueue`] (≤ 2 voice
//! frames per origin, ≤ 60 ms of voice backlog at the drain rate, stream-level shedding
//! of the lowest-ranked burst), and the just-in-time hand-off to the link layer.
//!
//! The hand-off moves one frame's worth of records when the link layer has no P0/P1
//! record waiting and its token bucket is not in debt, so the `age` rewrite at dequeue
//! (§4.5) sees almost all of the local residence and voice is dropped in the core,
//! never in an OS buffer. Each voice record carries its remaining age budget as the
//! link-layer deadline.

use mesh_routing::{Outgoing, Priority, QueueItem, RouteOutcome, RouterAction, ShedEvent};
use mesh_wire::mesh::{MeshHeader, PacketType, TrafficClass};
use mesh_wire::record::Record;

use super::state::Mesh;
use super::{cl, rl};
use crate::engine::Ctx;
use crate::links::Prio;
use crate::{Instant, LinkId};

/// A FLOOR_REJECT{outranked} the router originated for near-talker rejection (TTL 4).
fn is_near_talker_reject(me: u16, o: &Outgoing) -> bool {
    let h = &o.packet.header;
    h.origin == me
        && h.ptype == PacketType::FloorReject
        && h.ttl == mesh_routing::consts::NEAR_TALKER_REJECT_TTL
        && mesh_wire::mesh::FloorReject::decode(&o.packet.payload)
            .is_ok_and(|r| r.reason == mesh_wire::mesh::FloorRejectReason::Outranked)
}

fn prio(p: Priority) -> Prio {
    match p {
        Priority::P0 => Prio::P0,
        Priority::P1 => Prio::P1,
        Priority::P2 => Prio::P2,
        Priority::P3 => Prio::P3,
    }
}

impl Mesh {
    /// Applies a router outcome: actions, then the outgoing packets. Packets the
    /// router originated take the shared per-origin `seq` ([`Mesh::adopt_seq`]).
    pub fn apply(&mut self, ctx: &mut Ctx, out: RouteOutcome) {
        for a in out.actions {
            self.action(ctx, a);
        }
        let me = self.id.node;
        let mut seqs = Vec::new();
        for mut o in out.out {
            if is_near_talker_reject(me, &o) {
                // Near-talker rejection is flooded by the media module, which owns the
                // receive-side floor table; the router's copy would be a duplicate
                // (its local tombstone still applies to relay admission).
                continue;
            }
            self.adopt_seq(ctx, &mut o, &mut seqs);
            self.schedule(ctx, o);
        }
        self.drop_unadmitted();
    }

    fn action(&mut self, ctx: &mut Ctx, a: RouterAction) {
        match a {
            RouterAction::Disconnect { node, .. } => {
                // ≥ 20 penalty points in a minute: disconnect; the router keeps the
                // identity banned for 60 s (new links are refused in `link_up`).
                let ids: Vec<LinkId> = self
                    .links
                    .iter()
                    .filter(|(_, m)| m.peer == node)
                    .map(|(id, _)| *id)
                    .collect();
                for id in ids {
                    ctx.links.close_now(
                        id,
                        None,
                        crate::links::LinkDownReason::LocalClose,
                        &mut ctx.fx,
                    );
                }
            }
            RouterAction::PathHint { profile, .. } => {
                if profile == mesh_wire::mesh::EncoderProfile::Ble12 {
                    self.ble12_until =
                        Some(ctx.now.plus_us(mesh_routing::consts::PATH_HINT_HOLD_US));
                    self.sync_profile(ctx.now);
                }
            }
            // The media module keeps the talkers and the origin talk FSM (it reads the
            // FLOOR_REJECTs that target me itself; my own link refusing my burst is
            // reported by `own_voice`); the relay floor table only drives admission.
            RouterAction::Floor(_) | RouterAction::RejectedMe { .. } => {}
        }
    }

    /// Stages one outgoing packet on the link(s) the bundle policy picks.
    pub fn schedule(&mut self, ctx: &mut Ctx, o: Outgoing) {
        let now = ctx.now.routing();
        let voice = o.packet.header.class == TrafficClass::Voice;
        let Some(peer) = self.router.neighbors.node_of(o.link) else {
            return;
        };
        let (sel, switch) = match self.router.neighbors.neighbor(peer) {
            Some(n) => self.bundles.select(now, n),
            None => (None, None),
        };
        if let Some(sw) = switch
            && sw.downgrade
        {
            self.on_downgrade(ctx, sw.from, sw.to);
        }
        let Some(sel) = sel else { return };
        let mut targets = vec![sel.primary];
        if voice && let Some(ov) = sel.overlap {
            targets.push(ov);
            self.stats.overlap_copies += 1;
        }
        for link in targets {
            if voice && link != o.link && !self.admit_on(ctx, link, &o) {
                continue;
            }
            self.stage(ctx, link, o.prio, o.packet.clone());
        }
    }

    /// Voice moved by the bundle to another link than the router admitted it on: that
    /// link's admission applies too (hard enforcement per link, §6.3).
    fn admit_on(&mut self, ctx: &Ctx, link: mesh_routing::LinkId, o: &Outgoing) -> bool {
        let Some(b) = o.packet.burst else {
            return false;
        };
        let cap = self.router.link_cap(link);
        let congestion = self
            .router
            .neighbors
            .link(link)
            .is_some_and(|l| cap < self.router.cfg.limits.s_max(l.kind));
        let edge = self.router.is_edge_link(link);
        let d = self.router.admission.admits_at(
            ctx.now.routing(),
            link,
            b,
            &mut self.router.floor,
            cap,
            congestion,
            edge,
        );
        if d.admitted() {
            self.router.neighbors.mark_voice_tx(ctx.now.routing(), link);
        }
        d.admitted()
    }

    fn stage(
        &mut self,
        ctx: &mut Ctx,
        link: mesh_routing::LinkId,
        p: Priority,
        packet: mesh_routing::QueuedPacket,
    ) {
        // A link that starts carrying voice leaves the 8 kbps idle floor at once, before
        // its voice backlog bound (60 ms of drain) is computed.
        if packet.header.class == TrafficClass::Voice
            && self.links.get(&cl(link)).and_then(|m| m.applied_rate)
                < self.pacing.link(link).map(|l| l.pacer.rate_bps())
        {
            self.refresh_pacing(ctx);
        }
        let rate = self.drain_rate(link);
        let Some(meta) = self.links.get_mut(&cl(link)) else {
            return;
        };
        if let Some(k) = packet.burst {
            meta.staged_bursts.insert(k);
        }
        let shed = meta
            .queue
            .push(p, QueueItem::Mesh(packet), rate, &self.router.floor);
        self.on_shed(ctx, link, &shed);
        if let Some(pl) = self.pacing.link_mut(link) {
            pl.pacer.note_queued();
        }
    }

    /// Hard enforcement per link (§6.3): frames still staged for a burst that is no
    /// longer admitted on that link (evicted by a higher-ranked burst, shed, ended) are
    /// dropped, so the link never carries more bursts than its cap.
    pub fn drop_unadmitted(&mut self) {
        for (id, meta) in self.links.iter_mut() {
            if meta.staged_bursts.is_empty() {
                continue;
            }
            let admitted = self.router.admission.admitted(rl(*id));
            let gone: Vec<_> = meta
                .staged_bursts
                .iter()
                .filter(|k| !admitted.contains(k))
                .copied()
                .collect();
            for k in gone {
                meta.queue.drop_burst(k);
                meta.staged_bursts.remove(&k);
            }
        }
    }

    /// The rate the link drains at now (what the link layer's token bucket runs at).
    pub fn drain_rate(&self, link: mesh_routing::LinkId) -> u32 {
        self.links
            .get(&cl(link))
            .and_then(|m| m.applied_rate)
            .or_else(|| self.pacing.link(link).map(|l| l.pacer.send_rate_bps()))
            .unwrap_or(mesh_routing::consts::R_INIT_L2CAP_BPS)
    }

    /// Stream-level shedding and voice drops (§5.6): a shed burst is not forwarded on
    /// that link again until it ends; any voice drop marks the link congested for 2 s
    /// (the next NBR reports it).
    pub fn on_shed(&mut self, ctx: &Ctx, link: mesh_routing::LinkId, ev: &[ShedEvent]) {
        let now = ctx.now.routing();
        for e in ev {
            match e {
                ShedEvent::Burst(k) => {
                    self.stats.bursts_shed += 1;
                    self.router.on_shed(now, link, *k);
                }
                ShedEvent::Frame(_) => {
                    self.stats.voice_dropped += 1;
                    self.router.neighbors.mark_congested(now, link);
                }
            }
        }
    }

    /// Moves staged records into the link layer where there is room now: the link layer
    /// holds at most about one frame of records, and its token bucket is not in debt.
    pub fn pump(&mut self, ctx: &mut Ctx) {
        let now = ctx.now;
        let ids: Vec<LinkId> = self
            .links
            .iter()
            .filter(|(_, m)| !m.queue.is_empty())
            .map(|(id, _)| *id)
            .collect();
        let mut next: Option<Instant> = None;
        let mut wake = |t: Instant| next = Some(next.map_or(t, |n: Instant| n.min(t)));
        for id in ids {
            let Some(l) = ctx.links.get_mut(id) else {
                continue;
            };
            if !l.established() {
                continue;
            }
            let budget = l.plaintext_budget();
            let backlog = l.backlog_bytes();
            if backlog >= budget {
                if let Some(pl) = self.pacing.link_mut(rl(id)) {
                    pl.pacer.note_backlogged();
                }
                wake(now.plus_us(super::PUMP_RETRY_US));
                continue;
            }
            if !l.pace.allows(now) {
                if let Some(pl) = self.pacing.link_mut(rl(id)) {
                    pl.pacer.note_backlogged();
                }
                let at = l
                    .pace
                    .ready_at()
                    .unwrap_or(now.plus_us(super::PUMP_RETRY_US));
                wake(at.max(now.plus_us(1)));
                continue;
            }
            let room = budget - backlog;
            let rate = self.drain_rate(rl(id));
            let Some(meta) = self.links.get_mut(&id) else {
                continue;
            };
            let mut dropped = Vec::new();
            let recs = meta
                .queue
                .pop_frame(now.routing(), room, rate, &mut dropped);
            let more = !meta.queue.is_empty();
            self.on_shed(ctx, rl(id), &dropped);
            let max_age = self.router.cfg.limits.voice_max_age_us;
            for r in recs {
                if r.rtype != mesh_wire::record::rtype::MESH {
                    continue;
                }
                let (pr, deadline) = match r.mesh {
                    Some(h) if h.class == TrafficClass::Voice => {
                        let age = MeshHeader::decode(&r.body).map_or(0, |h| h.age_us());
                        (Prio::P1, Some(now.plus_us(max_age.saturating_sub(age))))
                    }
                    Some(h) => (prio(Priority::of_mesh(&h)), None),
                    None => (Prio::P2, None),
                };
                if ctx
                    .links
                    .send_record(id, pr, &Record::Mesh(&r.body), deadline)
                {
                    self.stats.handed_off += 1;
                }
            }
            if more {
                wake(now.plus_us(super::PUMP_RETRY_US));
            }
        }
        self.pump_at = next;
    }
}
