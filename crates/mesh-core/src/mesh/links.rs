//! Link lifecycle in the glue: neighbor table registration after IDPROOF, pacing state,
//! link quality inputs (PING RTT, PING loss, RSSI, liveness) copied from the link layer,
//! NBR send/ingest, voice-link detection and the burst-dependent link hints (fast PING,
//! Android connection priority).

use mesh_routing::{LinkQueue, LinkRole as RRole, LinkUpError, PacerPath};
use mesh_wire::record::{Nbr, Record};

use super::state::{LinkMeta, Mesh};
use super::{cl, rl};
use crate::LinkId;
use crate::config::Platform;
use crate::effects::{LinkPriority, RadioCommand};
use crate::engine::Ctx;
use crate::links::{LinkDownReason, LinkKind, LinkRole, LinkState, Prio};

fn pacer_path(kind: LinkKind) -> PacerPath {
    match kind {
        LinkKind::BleL2cap => PacerPath::L2cap,
        LinkKind::BleGatt => PacerPath::Gatt,
        LinkKind::WifiAware => PacerPath::Aware,
        LinkKind::Tunnel => PacerPath::Tunnel,
    }
}

impl Mesh {
    /// A group link is Up (IDPROOF verified, peer known).
    pub fn link_up(&mut self, ctx: &mut Ctx, link: LinkId) {
        let Some(l) = ctx
            .links
            .get(link)
            .filter(|l| l.state == LinkState::Up && !l.is_lab())
        else {
            return;
        };
        let Some(peer) = l.peer_node.map(|p| p.0) else {
            return;
        };
        let (info, max_frame) = (l.info, l.info.max_frame);
        let now = ctx.now;
        let role = match info.role {
            LinkRole::Initiator => RRole::Initiator,
            LinkRole::Responder => RRole::Responder,
        };
        let kind = info.kind.routing();
        let r = self
            .router
            .neighbors
            .link_up(now.routing(), peer, rl(link), kind, role, max_frame);
        match r {
            Ok(()) => {}
            Err(LinkUpError::Banned(_)) => {
                // A banned identity (penalties, §5.2) is disconnected again.
                ctx.links
                    .close_now(link, None, LinkDownReason::LocalClose, &mut ctx.fx);
                return;
            }
            Err(LinkUpError::Duplicate(old)) => {
                // The secure module resolves duplicates before Up; if one slips through,
                // the bundle follows the link the §5.7 rule keeps.
                let old_role = self
                    .links
                    .get(&cl(old))
                    .map_or(RRole::Responder, |m| rrole(m.role));
                let keep_new = mesh_routing::resolve_duplicate(
                    self.id.node,
                    peer,
                    (old, old_role),
                    (rl(link), role),
                ) == old;
                if !keep_new {
                    return;
                }
                self.forget_link(cl(old));
                if table_link_up(self, now, peer, link, kind, role, max_frame).is_err() {
                    return;
                }
            }
            Err(LinkUpError::KnownLink) => return,
        }
        if let Some(ls) = self.router.neighbors.link_mut(rl(link)) {
            ls.quality.rtt = ctx
                .links
                .get(link)
                .map(|l| l.rtt.clone())
                .unwrap_or_default();
        }
        self.pacing
            .add_link(now.routing(), rl(link), pacer_path(info.kind), max_frame);
        let counters = ctx.links.get(link).map(|l| l.counters).unwrap_or_default();
        self.links.insert(
            link,
            LinkMeta {
                kind: info.kind,
                role: info.role,
                disc_id: info.disc_id,
                peer,
                queue: LinkQueue::new(self.router.cfg.limits.voice_max_age_us),
                applied_rate: None,
                pings_sent: counters.pings_sent,
                pongs_received: counters.pongs_received,
                pings_owed: 0,
                high_priority: None,
                staged_bursts: Default::default(),
                peer_caps: ctx.links.get(link).and_then(|l| l.peer_caps),
            },
        );
        self.refresh_pacing(ctx);
        // A new link changes our NBR: send it at once (it is also the first heartbeat).
        self.send_nbr(ctx, true);
    }

    /// Removes a link from every glue table.
    pub fn forget_link(&mut self, link: LinkId) {
        self.router.on_link_down(rl(link));
        self.pacing.remove_link(rl(link));
        self.links.remove(&link);
    }

    /// A group link went down.
    pub fn link_down(&mut self, ctx: &mut Ctx, link: LinkId, reason: LinkDownReason) {
        let meta = self.links.get(&link).cloned();
        let peer = meta.as_ref().map(|m| m.peer);
        self.forget_link(link);
        self.nbr.on_link_down();
        if let Some(p) = peer
            && self.router.neighbors.neighbor(p).is_none()
        {
            self.seg.forget(p);
            self.bundles.forget(p);
        }
        if reason == LinkDownReason::LimitReached {
            self.selector.on_limit_reached(ctx.now.routing());
        }
        if let Some(m) = meta {
            self.upgrades.on_link_down(ctx, m.peer, m.kind);
            self.arm_reconnect(ctx, &m, reason);
        }
        self.refresh_pacing(ctx);
        self.send_nbr(ctx, false);
    }

    /// Copies the link layer's quality inputs into the neighbor table: RTT estimator,
    /// RSSI, last inbound frame, PING loss once per second.
    pub fn sync_quality(&mut self, ctx: &Ctx) {
        let now = ctx.now;
        let loss_due = now >= self.next_loss;
        if loss_due {
            self.next_loss = now.plus_us(super::LOSS_PERIOD_US);
        }
        for (id, meta) in self.links.iter_mut() {
            let Some(l) = ctx.links.get(*id) else {
                continue;
            };
            if let Some(ls) = self.router.neighbors.link_mut(rl(*id)) {
                ls.quality.rtt = l.rtt.clone();
                ls.quality.rssi = l.rssi;
                ls.last_rx = ls.last_rx.max(l.last_rx.routing());
            }
            if let Some(n) = self.router.neighbors.neighbor_mut(meta.peer) {
                n.last_rx = n.last_rx.max(l.last_rx.routing());
            }
            if loss_due {
                let sent = l.counters.pings_sent.saturating_sub(meta.pings_sent);
                let got = l
                    .counters
                    .pongs_received
                    .saturating_sub(meta.pongs_received);
                meta.pings_sent = l.counters.pings_sent;
                meta.pongs_received = l.counters.pongs_received;
                let (expected, lost) = settle_pings(&mut meta.pings_owed, sent, got);
                if expected > 0 {
                    self.router.neighbors.on_loss(rl(*id), expected, lost);
                }
            }
        }
        if loss_due {
            self.router.neighbors.tick(now.routing());
        }
    }

    /// Our NBR on every Up link when due (on change ≤ 4 Hz, heartbeat 1 s, at once after
    /// a link change).
    pub fn send_nbr(&mut self, ctx: &mut Ctx, force: bool) {
        let now = ctx.now.routing();
        let own = self.router.neighbors.own_entries(now);
        if force {
            self.nbr.on_link_down();
        }
        let Some(nbr) = self.nbr.poll(now, &own) else {
            return;
        };
        self.nbr_last = Some(ctx.now);
        for id in self.links.keys().copied().collect::<Vec<_>>() {
            if ctx.send_record(id, Prio::P0, &Record::Nbr(nbr)) {
                self.stats.nbr_sent += 1;
            }
        }
    }

    /// An NBR record from the neighbor on `link`.
    pub fn on_nbr(&mut self, ctx: &Ctx, link: LinkId, nbr: &Nbr) {
        if !self.links.contains_key(&link) {
            return;
        }
        self.stats.nbr_received += 1;
        self.router
            .neighbors
            .on_nbr(ctx.now.routing(), rl(link), nbr);
    }

    /// Voice links are those that carried voice in the last 2 s (relayed or my own:
    /// both are marked when admitted on a link). Returns `true` when a link started
    /// carrying voice since the last check. Also drives the BLE PING period (500 ms
    /// while a burst is active, §5.1; Aware links keep 500 ms in the link layer, §9.6)
    /// and the Android GATT-client connection priority.
    pub fn detect_voice(&mut self, ctx: &mut Ctx) -> bool {
        let now = ctx.now.routing();
        let voice: std::collections::BTreeSet<LinkId> = self
            .links
            .keys()
            .copied()
            .filter(|id| {
                self.router
                    .neighbors
                    .link(rl(*id))
                    .is_some_and(|s| s.is_voice_link(now))
            })
            .collect();
        let started = voice.iter().any(|l| !self.voice_links.contains(l));
        self.voice_links = voice;
        let active =
            self.router.floor.entries().iter().any(|e| e.live()) || !self.voice_links.is_empty();
        if active != self.fast_ping {
            self.fast_ping = active;
            ctx.links.set_fast_ping(active);
        }
        if ctx.radio.set_voice_active(active) {
            ctx.refresh_radio();
        }
        if ctx.cfg.platform == Platform::Android {
            for (id, m) in self.links.iter_mut() {
                if m.kind == LinkKind::BleGatt
                    && m.role == LinkRole::Initiator
                    && m.high_priority != Some(active)
                {
                    m.high_priority = Some(active);
                    ctx.fx.diag(mesh_lab::ridelog::DiagEvent::LinkParams {
                        link: id.0,
                        prio: if active { "high" } else { "balanced" },
                        grant_kbps: m.applied_rate.map_or(0.0, |r| r as f32 / 1_000.0),
                    });
                    ctx.fx.radio(RadioCommand::SetLinkParams {
                        link: *id,
                        priority: if active {
                            LinkPriority::High
                        } else {
                            LinkPriority::Balanced
                        },
                    });
                }
            }
        }
        started
    }
}

fn rrole(r: LinkRole) -> RRole {
    match r {
        LinkRole::Initiator => RRole::Initiator,
        LinkRole::Responder => RRole::Responder,
    }
}

fn table_link_up(
    m: &mut Mesh,
    now: crate::Instant,
    peer: u16,
    link: LinkId,
    kind: mesh_routing::LinkKind,
    role: RRole,
    max_frame: u16,
) -> Result<(), LinkUpError> {
    m.router
        .neighbors
        .link_up(now.routing(), peer, rl(link), kind, role, max_frame)
}

/// One loss period of PING accounting: `sent` PINGs and `got` PONGs since the last
/// one, `owed` PINGs of the previous period still unanswered. PONGs pay the oldest
/// PINGs first; a PING still unanswered a whole period after the one it was sent in is
/// lost. Returns `(settled, lost)`. Counting `sent − got` per period instead reads a
/// PONG that crosses a period boundary as a loss, and with the 2 s PING period phase-
/// locked to the 1 s accounting that is every PING: `q` of a perfect link decayed to 0
/// and its neighbors stopped pruning their relays to it (§5.4, first field test).
pub(crate) fn settle_pings(owed: &mut u64, sent: u64, got: u64) -> (u32, u32) {
    let old = *owed;
    let pay_old = got.min(old);
    let pay_new = (got - pay_old).min(sent);
    *owed = sent - pay_new;
    let settled = (old + pay_new).min(u64::from(u32::MAX)) as u32;
    let lost = (old - pay_old).min(u64::from(u32::MAX)) as u32;
    (settled, lost)
}

#[cfg(test)]
mod ping_loss_tests {
    use super::settle_pings;

    #[test]
    fn a_pong_in_the_next_period_is_not_a_loss() {
        let mut owed = 0;
        // PING at the end of a period, its PONG early in the next, every 2 s.
        for _ in 0..10 {
            assert_eq!(settle_pings(&mut owed, 1, 0), (0, 0));
            assert_eq!(settle_pings(&mut owed, 0, 1), (1, 0));
        }
        // Answered within the period.
        assert_eq!(settle_pings(&mut owed, 1, 1), (1, 0));
        // Never answered: lost one period later.
        assert_eq!(settle_pings(&mut owed, 1, 0), (0, 0));
        assert_eq!(settle_pings(&mut owed, 0, 0), (1, 1));
        assert_eq!(owed, 0);
        // A lost PING then an answered one.
        assert_eq!(settle_pings(&mut owed, 1, 0), (0, 0));
        assert_eq!(settle_pings(&mut owed, 1, 1), (1, 0));
        assert_eq!(settle_pings(&mut owed, 0, 0), (1, 1));
    }
}
