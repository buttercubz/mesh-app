//! Session lifecycle, link hooks, records and timers of the group module.

use mesh_api::{NoticeKey, Params};
use mesh_wire::frame::RejectReason;
use mesh_wire::mesh::{LeaveReason, PacketType};
use mesh_wire::record::{Caps, Record, SyncKind};

use super::active::{Active, JOIN_RETX_US};
use super::core::GroupCore;
use super::flood::{self, Delivered};
use super::sync;
use crate::engine::Ctx;
use crate::links::{LinkDownReason, LinkState, Prio};
use crate::session::presence::{self, GroupView};
use crate::session::{self, Out, Session, view};
use crate::{LinkId, NodeId};

impl GroupCore {
    /// Starts the runtime of `id` (engine entered `Session{group_id}`).
    pub(crate) fn start_session(&mut self, ctx: &mut Ctx, id: [u8; 16]) {
        let Some(g) = self.groups.get(&id) else {
            ctx.fx.log(
                crate::effects::LogLevel::Warn,
                "session for an unknown group",
            );
            return;
        };
        let now = ctx.now;
        let mut s = Session::new(g.my_node_id(), g.vehicle, now, g.policy.voice_ttl());
        s.known = g.roster_indices().into_iter().map(|(n, _)| n).collect();
        s.ble_on = ctx.radio.ble == crate::links::RadioAvail::On;
        // `seq` comes from the engine's shared per-origin counter (`Ctx::next_mesh_seq`).
        let msg = (ctx.rand_u32() & 0xFFFF) as u16;
        let mut act = Active::new(id, s, msg, now);
        if let Some(r) = g.roster() {
            let me = g.my_id_hash();
            for e in r.entries.iter().filter(|e| e.id_hash != me) {
                act.registry.observe(&e.id_hash, &me);
            }
        }
        view::on_roster(&mut act.session, ctx, g);
        self.act = Some(act);
        if self.groups.get(&id).is_some_and(|g| !g.session_runs()) {
            return;
        }
        // The session's media state (identity, names) is posted at once, before any
        // Go live (`sync_media` at the end of this hook).
        self.retarget_radio(ctx);
        self.refresh_caps(ctx);
    }

    /// F8 Leave (`Engine::stop`, before the links close): floods my signed LEAVE{user}
    /// on every Up link. It is P0 and queued on the links at once, so it leaves ahead of
    /// the engine's `CLOSE{leaving}`.
    pub(crate) fn leave(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        if !super::core::runs(g, act) || act.up_links().is_empty() {
            return;
        }
        let out = session::ride::leave_out(&g.identity, &g.id, ctx.sess, LeaveReason::User);
        self.originate_before_close(ctx, &out);
    }

    /// Stops the runtime (engine stop or mode change). Links are closing already.
    pub(crate) fn stop_session(&mut self, ctx: &mut Ctx) {
        if let Some(mut act) = self.act.take() {
            session::audio::end_live(&mut act.session, ctx);
            if act.dirty_store {
                self.persist(ctx, act.group_id);
            }
        }
    }

    /// Floods `out` as this node, through the mesh glue (§5.2 origination).
    pub(crate) fn originate(&mut self, ctx: &mut Ctx, out: &Out) {
        self.originate_via(ctx, out, flood::Via::Mesh);
    }

    /// Floods `out` as this node straight onto every Up link's queue: the caller closes
    /// links right behind it in this call (LEAVE, GROUP_CTL{close}, the ROSTER/REKEY
    /// that deny a member), and only the link queue's order makes it leave first.
    pub(crate) fn originate_before_close(&mut self, ctx: &mut Ctx, out: &Out) {
        self.originate_via(ctx, out, flood::Via::BeforeClose);
    }

    fn originate_via(&mut self, ctx: &mut Ctx, out: &Out, via: flood::Via) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        let me = g.my_node_id();
        let leader = g.is_leader();
        flood::originate(ctx, act, me, leader, out, via);
    }

    /// My PRESENCE inputs from the group.
    pub(crate) fn group_view(&self) -> Option<GroupView> {
        let act = self.act.as_ref()?;
        let g = self.groups.get(&act.group_id)?;
        Some(GroupView {
            leader: g.is_leader(),
            roster_version: g.control.roster_version(),
            epoch: g.epoch(),
        })
    }

    pub(crate) fn link_up(&mut self, ctx: &mut Ctx, link: LinkId) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        let kind = ctx.links.get(link).map(|l| l.info.kind);
        if let Some(k) = kind {
            act.session.link_kinds.insert(link, k);
        }
        if let Some(rt) = act.links.get(&link)
            && let Some(tok) = rt.speculative
        {
            act.spec_backoff.remove(&tok);
        }
        // §8.9 rule 5: JOIN_REQ on every new link until a verified roster lists me.
        if !g.in_roster() {
            let body = join_body(g, ctx.cfg.platform.caps_byte());
            sync::send(ctx, act, link, SyncKind::JoinReq, &body);
        }
        // Presence goes out promptly so the peer sees my state and versions.
        act.session.next_presence = act.session.next_presence.min(ctx.now);
        ctx.mark_dirty();
    }

    pub(crate) fn link_down(
        &mut self,
        ctx: &mut Ctx,
        link: LinkId,
        reason: LinkDownReason,
        _peer: Option<NodeId>,
    ) {
        let Some(act) = self.act.as_mut() else { return };
        let rt = act.links.remove(&link);
        act.session.link_kinds.remove(&link);
        ctx.mark_dirty();
        let Some(rt) = rt else { return };
        // What the peer told me when it refused this link (REJECT frame / CLOSE record).
        match reason.peer_reason() {
            Some(RejectReason::NodeIdCollision) if !rt.collided => {
                self.refused_as_collision(ctx);
            }
            Some(RejectReason::MemberRemovedOrDenied) => self.refused_as_removed(ctx),
            _ => {}
        }
    }

    /// A peer refused my identity with `REJECT{8}` / `CLOSE{8}` although its own
    /// `node_id` does not collide with mine: it holds another identity with my `node_id`
    /// (§8.2, a third node's report). I cannot compare `id_hash`es with an identity I
    /// never saw, and the peer will refuse me until one of us changes, so I regenerate
    /// and rejoin with the same invite, unless I lead (the trust anchor never changes).
    fn refused_as_collision(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        if g.is_leader() || !super::core::runs(g, act) {
            return;
        }
        ctx.fx.log(
            crate::effects::LogLevel::Info,
            "group: node_id collision reported by a peer, regenerating my identity",
        );
        self.regenerate_identity(ctx);
    }

    /// A member refused me with `REJECT{5}` / `CLOSE{5}`: its verified roster denies me,
    /// and I missed that roster (removed while away, §1.3 F9). The rider sees "You were
    /// removed" once; the session's radio and audio stop. The stored group keeps its
    /// state: only a verified roster can deny me there.
    fn refused_as_removed(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        if !super::core::runs(g, act) {
            return;
        }
        act.refused = true;
        let mut p = Params::new();
        p.insert("group".into(), g.name.clone().into());
        ctx.notice_and_notify(NoticeKey::Removed, p);
        session::audio::end_session(&mut act.session, ctx);
        ctx.radio.stop(&mut ctx.fx);
        self.close_all(ctx, RejectReason::Leaving, Prio::P0);
        ctx.mark_dirty();
    }

    /// The peer's CAPS: anti-entropy push if it is behind (§8.9 rule 2).
    pub(crate) fn peer_caps(&mut self, ctx: &mut Ctx, link: LinkId, caps: &Caps) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        let summary = (caps.epoch, caps.roster_version, caps.ctl_version);
        let Some(rt) = act.links.get_mut(&link) else {
            return;
        };
        rt.peer_summary = Some(summary);
        if caps.epoch >= g.epoch()
            && let Some(p) = rt.peer
        {
            act.newest_holders.insert(p.id_hash());
        }
        let kinds = g.control.to_push(summary);
        sync::push(ctx, act, &g.control, link, &kinds);
    }

    /// A record of a group link. Returns `true` when claimed.
    pub(crate) fn record(&mut self, ctx: &mut Ctx, link: LinkId, rec: &Record<'_>) -> bool {
        if self
            .act
            .as_ref()
            .is_none_or(|a| !a.links.contains_key(&link))
        {
            return false;
        }
        let state = ctx.links.get(link).map(|l| l.state);
        if let Record::IdProof(p) = rec {
            crate::secure::admit::on_idproof(self, ctx, link, p);
            return true;
        }
        if state != Some(LinkState::Up) {
            // Nothing but IDPROOF flows before admission (§8.4).
            return true;
        }
        match rec {
            Record::Sync(s) => {
                let got = self
                    .act
                    .as_mut()
                    .and_then(|a| sync::ingest(ctx, a, link, s));
                if let Some((kind, body)) = got {
                    self.on_sync_body(ctx, link, kind, &body);
                }
                true
            }
            Record::Mesh(bytes) => {
                let delivered = self.active_mut().and_then(|(g, act)| {
                    let peer = act.links.get(&link)?.peer?.node_id();
                    flood::ingest(ctx, act, g.my_node_id(), peer, bytes)
                });
                if let Some(d) = delivered {
                    self.deliver(ctx, link, d);
                }
                false
            }
            _ => false,
        }
    }

    /// Delivery of an authenticated, deduplicated packet.
    fn deliver(&mut self, ctx: &mut Ctx, link: LinkId, d: Delivered) {
        match d {
            Delivered::Presence {
                origin,
                sess,
                hops,
                age_us,
                p,
            } => {
                let Some((g, act)) = self.active_mut() else {
                    return;
                };
                let owd = ctx.links.get(link).map_or(0, |l| l.owd_us());
                let _events =
                    presence::ingest(&mut act.session, ctx, origin, sess, hops, age_us, owd, &p);
                if hops == 0 {
                    if let Some(id) = act.links.get(&link).and_then(|l| l.peer)
                        && mesh_wire::serial::truncate16(g.epoch()) == p.epoch16
                    {
                        act.newest_holders.insert(id.id_hash());
                    }
                    let kinds = g.control.to_push_presence(p.epoch16, p.roster_ver16);
                    // §8.9 rule 3: push on mismatch (rate limited per link and kind);
                    // a caught-up kind may be pushed again at once later.
                    sync::caught_up(act, link, &kinds);
                    if !kinds.is_empty() {
                        sync::push(ctx, act, &g.control, link, &kinds);
                    }
                }
                ctx.mark_dirty();
            }
            Delivered::Body {
                ptype,
                origin,
                body,
            } => {
                let kind = match ptype {
                    PacketType::Roster => SyncKind::Roster,
                    PacketType::Rekey => SyncKind::Rekey,
                    _ => SyncKind::JoinReq,
                };
                let _ = origin;
                self.apply_body(ctx, kind, &body);
            }
            Delivered::GroupCtl { body } => self.apply_body(ctx, SyncKind::GroupCtl, &body),
            Delivered::Leave {
                origin,
                sess,
                leave,
            } => {
                let Some((g, act)) = self.active_mut() else {
                    return;
                };
                let name = g.name_of(origin);
                session::ride::on_leave(
                    &mut act.session,
                    ctx,
                    origin,
                    sess,
                    leave,
                    g.roster(),
                    name,
                );
            }
            Delivered::RideCtl { origin, ctl } => {
                let Some((g, act)) = self.active_mut() else {
                    return;
                };
                session::ride::on_ride_ctl(
                    &mut act.session,
                    ctx,
                    origin,
                    ctl,
                    g.roster(),
                    g.epoch(),
                );
            }
            Delivered::Position { origin, p } => {
                let Some(act) = self.act.as_mut() else { return };
                session::ride::on_position(&mut act.session, ctx, origin, &p);
            }
        }
    }

    fn on_sync_body(&mut self, ctx: &mut Ctx, _link: LinkId, kind: SyncKind, body: &[u8]) {
        self.apply_body(ctx, kind, body);
    }

    /// Timers: PRESENCE, POSITION, JOIN_REQ re-flood, token rotation, reachability.
    /// Every action is due by its own deadline, so the outcome does not depend on how
    /// often the runtime calls (≤ 5 ms on device).
    pub(crate) fn timer(&mut self, ctx: &mut Ctx) {
        let Some(view) = self.group_view() else {
            return;
        };
        let mut outs: Vec<Out> = Vec::new();
        let rotate;
        {
            let Some((g, act)) = self.active_mut() else {
                return;
            };
            if !super::core::runs(g, act) {
                return;
            }
            let s = &mut act.session;
            if let Some(o) = presence::tick(s, ctx, view) {
                outs.push(o);
            }
            if let Some(o) = session::ride::position_tick(s, ctx) {
                outs.push(o);
            }
            session::audio::radio_tick(s, ctx);
            view::tick(s, ctx, g);
            if !g.in_roster() && ctx.now >= act.join_next {
                act.join_next = ctx.now.plus_us(JOIN_RETX_US);
                outs.push(Out {
                    ptype: PacketType::JoinReq,
                    payload: join_body(g, ctx.cfg.platform.caps_byte()),
                });
            }
            rotate = super::radio::slot_now(ctx) != act.t10 || act.radio_epoch != g.epoch();
        }
        for o in &outs {
            self.originate(ctx, o);
        }
        if rotate {
            self.retarget_radio(ctx);
        }
    }

    /// My per-group identity collides with a peer's and mine has the larger `id_hash`
    /// (§8.2): regenerate it, close the group links and rejoin with the same invite. The
    /// rider's ride state (live capture, talk mode, ride, sharing) carries over: only
    /// the identity and everything keyed by the old `node_id` start afresh.
    pub(crate) fn regenerate_identity(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        g.regenerate_identity(&mut ctx.rng);
        g.remember(
            *g.identity.public(),
            ctx.cfg.platform.caps_byte(),
            &g.display_name.clone(),
        );
        act.dirty_store = true;
        let id = act.group_id;
        // `CLOSE{leaving}`, not `{8}`: a peer that reads `{8}` concludes that *its*
        // identity collides with a third node's (see `refused_as_collision`).
        let ids: Vec<LinkId> = act.links.keys().copied().collect();
        for l in ids {
            let now = ctx.now;
            ctx.links.close(now, l, RejectReason::Leaving, &mut ctx.fx);
        }
        // Keep the ride state; a non-live copy is torn down so no audio command leaves.
        let old = core::mem::replace(
            &mut act.session,
            Session::new(0, g.vehicle, ctx.now, g.policy.voice_ttl()),
        );
        self.flush(ctx);
        self.stop_session(ctx);
        self.start_session(ctx, id);
        if let Some(act) = self.act.as_mut() {
            act.session.carry_over(&old);
        }
        ctx.mark_dirty();
    }

    /// I learned that I am denied: flood a signed LEAVE{removed}, close every link,
    /// tell the rider (§1.3 F9).
    pub(crate) fn on_removed(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        let out = session::ride::leave_out(&g.identity, &g.id, ctx.sess, LeaveReason::Removed);
        let mut p = Params::new();
        p.insert("group".into(), g.name.clone().into());
        ctx.notice_and_notify(NoticeKey::Removed, p);
        session::audio::end_session(&mut act.session, ctx);
        self.originate_before_close(ctx, &out);
        // No new links: every member refuses my identity from now on.
        ctx.radio.stop(&mut ctx.fx);
        // The LEAVE (P0) leaves first. `CLOSE{leaving}`, not `{5}`: a peer that reads
        // `{5}` concludes that *it* was removed (see `refused_as_removed`).
        self.close_all(ctx, RejectReason::Leaving, Prio::P0);
    }

    /// Closes every group link with `reason` once what is queued on it up to priority
    /// `keep` has left (the LEAVE or GROUP_CTL just flooded): ordered by the link's own
    /// queue, never by a timer.
    pub(crate) fn close_all(&mut self, ctx: &mut Ctx, reason: RejectReason, keep: Prio) {
        let Some(act) = self.act.as_mut() else { return };
        let ids: Vec<LinkId> = act.links.keys().copied().collect();
        for l in ids {
            if let Some(rt) = act.links.get_mut(&l) {
                rt.up = false;
            }
            let now = ctx.now;
            ctx.links.close_graceful(now, l, reason, keep, &mut ctx.fx);
        }
    }
}

/// My signed JOIN_REQ body.
pub(crate) fn join_body(g: &super::state::Group, platform: u8) -> Vec<u8> {
    mesh_crypto::join::create(&g.identity, &g.id, platform, &g.display_name).encode()
}
