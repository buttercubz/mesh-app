//! Wi-Fi Aware upgrade over the existing BLE link (DESIGN §9.6-§9.7; Phase 4 hooks).
//!
//! `UPGRADE{offer, transport, nonce}` → `UPGRADE{accept}` → the native layer (WP-24)
//! builds the Aware data path (`RadioCommand::AwareUpgrade`) → a fresh Noise HS + IDPROOF
//! on the Aware link → the link joins the neighbor's bundle → `UPGRADE{ready}`. The BLE
//! link always stays as the fallback and control path. An upgrade needs both CAPS to
//! carry the matching transport bit (iPhone↔iPhone or Android↔Android only in v1), the
//! Aware radio on, the group's Aware pairing and the policy switch. The lower `node_id`
//! offers, so two neighbors never offer at once.
//!
//! `RadioCommand::AwareUpgrade{peer, op}` mirrors the UPGRADE ops to the native layer:
//! 1 offer sent (prepare the session), 2 accepted (build the data path), 3 rejected
//! (abandon), 4 ready, 5 retire (tear the Aware path down).

use std::collections::BTreeMap;

use mesh_wire::mesh::{FloorReject, FloorRejectReason, PacketType};
use mesh_wire::record::{Caps, Record, Upgrade, UpgradeOp, UpgradeTransport};

use super::state::Mesh;
use crate::config::Platform;
use crate::effects::RadioCommand;
use crate::engine::Ctx;
use crate::links::{LinkKind, Prio, RadioAvail};
use crate::{Instant, LinkId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Offered,
    Accepted,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Attempt {
    step: Option<Step>,
    nonce: u32,
    transport: UpgradeTransport,
    last: Instant,
}

/// Upgrade state per neighbor.
#[derive(Debug, Clone, Default)]
pub struct Upgrades {
    per: BTreeMap<u16, Attempt>,
}

impl Upgrades {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` while an upgrade with `peer` is in progress or done.
    pub fn active(&self, peer: u16) -> bool {
        self.per.get(&peer).is_some_and(|a| a.step.is_some())
    }

    /// The Aware link of `peer` went down: retire (and allow a new offer later).
    pub fn on_link_down(&mut self, ctx: &mut Ctx, peer: u16, kind: LinkKind) {
        if kind != LinkKind::WifiAware {
            return;
        }
        if let Some(a) = self.per.get_mut(&peer) {
            a.step = None;
            a.last = ctx.now;
        }
        ctx.fx.radio(RadioCommand::AwareUpgrade { peer, op: 5 });
    }
}

/// The Aware transport both sides support, if any.
pub fn common_transport(mine: Platform, my_bits: u8, peer: &Caps) -> Option<UpgradeTransport> {
    match mine {
        Platform::Ios
            if my_bits & Caps::T_AWARE_IOS != 0
                && peer.transports & Caps::T_AWARE_IOS != 0
                && peer.platform == Caps::PLATFORM_IOS =>
        {
            Some(UpgradeTransport::AwareIos)
        }
        Platform::Android
            if my_bits & Caps::T_AWARE_ANDROID != 0
                && peer.transports & Caps::T_AWARE_ANDROID != 0
                && peer.platform == Caps::PLATFORM_ANDROID =>
        {
            Some(UpgradeTransport::AwareAndroid)
        }
        _ => None,
    }
}

fn send(ctx: &mut Ctx, link: LinkId, op: UpgradeOp, transport: UpgradeTransport, nonce: u32) {
    let u = Upgrade {
        op,
        transport,
        nonce,
        params: &[],
    };
    ctx.send_record(link, Prio::P0, &Record::Upgrade(u));
}

impl Mesh {
    fn upgrade_allowed(&self, ctx: &Ctx) -> bool {
        self.policy.aware_upgrade && self.id.aware_pairing && ctx.radio.aware == RadioAvail::On
    }

    fn ble_link_of(&self, peer: u16) -> Option<LinkId> {
        self.links
            .iter()
            .find(|(_, m)| m.peer == peer && m.kind.is_ble())
            .map(|(id, _)| *id)
    }

    /// Offers upgrades to eligible neighbors (lower `node_id` offers).
    pub fn upgrade_tick(&mut self, ctx: &mut Ctx) {
        if !self.upgrade_allowed(ctx) {
            return;
        }
        let aware_links = self
            .links
            .values()
            .filter(|m| m.kind == LinkKind::WifiAware)
            .count();
        if aware_links >= usize::from(self.router.cfg.limits.aware_links_max) {
            return;
        }
        let candidates: Vec<(LinkId, u16, UpgradeTransport)> = self
            .links
            .iter()
            .filter(|(_, m)| m.kind.is_ble() && m.peer > self.id.node)
            .filter_map(|(id, m)| {
                let t = common_transport(
                    ctx.cfg.platform,
                    ctx.cfg.limits.transports,
                    m.peer_caps.as_ref()?,
                )?;
                Some((*id, m.peer, t))
            })
            .filter(|(_, p, _)| {
                !self
                    .links
                    .values()
                    .any(|m| m.peer == *p && m.kind == LinkKind::WifiAware)
            })
            .collect();
        for (link, peer, transport) in candidates {
            let retry = self.policy.upgrade_retry_us;
            let due = self
                .upgrades
                .per
                .get(&peer)
                .is_none_or(|a| a.step.is_none() && ctx.now.since(a.last) >= retry);
            if !due {
                continue;
            }
            let nonce = ctx.rand_u32();
            self.upgrades.per.insert(
                peer,
                Attempt {
                    step: Some(Step::Offered),
                    nonce,
                    transport,
                    last: ctx.now,
                },
            );
            send(ctx, link, UpgradeOp::Offer, transport, nonce);
            ctx.fx.radio(RadioCommand::AwareUpgrade { peer, op: 1 });
            // One new attempt per tick.
            break;
        }
    }

    /// An UPGRADE record from the neighbor on `link`.
    pub fn on_upgrade(&mut self, ctx: &mut Ctx, link: LinkId, u: &Upgrade<'_>) {
        let Some(peer) = self.links.get(&link).map(|m| m.peer) else {
            return;
        };
        let mine = self.links.get(&link).and_then(|m| {
            common_transport(
                ctx.cfg.platform,
                ctx.cfg.limits.transports,
                m.peer_caps.as_ref()?,
            )
        });
        let now = ctx.now;
        match u.op {
            UpgradeOp::Offer => {
                let ok = self.upgrade_allowed(ctx) && mine == Some(u.transport);
                if ok {
                    self.upgrades.per.insert(
                        peer,
                        Attempt {
                            step: Some(Step::Accepted),
                            nonce: u.nonce,
                            transport: u.transport,
                            last: now,
                        },
                    );
                    send(ctx, link, UpgradeOp::Accept, u.transport, u.nonce);
                    ctx.fx.radio(RadioCommand::AwareUpgrade { peer, op: 2 });
                } else {
                    send(ctx, link, UpgradeOp::Reject, u.transport, u.nonce);
                }
            }
            UpgradeOp::Accept => {
                if let Some(a) = self.upgrades.per.get_mut(&peer)
                    && a.step == Some(Step::Offered)
                    && a.nonce == u.nonce
                {
                    a.step = Some(Step::Accepted);
                    ctx.fx.radio(RadioCommand::AwareUpgrade { peer, op: 2 });
                }
            }
            UpgradeOp::Reject => {
                if let Some(a) = self.upgrades.per.get_mut(&peer)
                    && a.nonce == u.nonce
                {
                    a.step = None;
                    a.last = now;
                    ctx.fx.radio(RadioCommand::AwareUpgrade { peer, op: 3 });
                }
            }
            UpgradeOp::Ready => {
                if let Some(a) = self.upgrades.per.get_mut(&peer) {
                    a.step = Some(Step::Ready);
                }
            }
            UpgradeOp::Retire => {
                if let Some(a) = self.upgrades.per.get_mut(&peer) {
                    a.step = None;
                    a.last = now;
                }
                ctx.fx.radio(RadioCommand::AwareUpgrade { peer, op: 5 });
            }
        }
    }

    /// An Aware link of `peer` joined the bundle: tell the peer over BLE.
    pub fn upgrade_link_up(&mut self, ctx: &mut Ctx, peer: u16) {
        let Some(ble) = self.ble_link_of(peer) else {
            return;
        };
        let (transport, nonce) = match self.upgrades.per.get_mut(&peer) {
            Some(a) => {
                a.step = Some(Step::Ready);
                (a.transport, a.nonce)
            }
            None => (
                match ctx.cfg.platform {
                    Platform::Android => UpgradeTransport::AwareAndroid,
                    _ => UpgradeTransport::AwareIos,
                },
                0,
            ),
        };
        send(ctx, ble, UpgradeOp::Ready, transport, nonce);
        ctx.fx.radio(RadioCommand::AwareUpgrade { peer, op: 4 });
    }

    /// Aware → BLE for one neighbor: re-evaluate the bursts admitted on the Aware link
    /// with the BLE link's cap; the excess, lowest rank first, gets
    /// `FLOOR_REJECT{congestion}` (§9.7).
    pub fn on_downgrade(
        &mut self,
        ctx: &mut Ctx,
        from: mesh_routing::LinkId,
        to: mesh_routing::LinkId,
    ) {
        let now = ctx.now.routing();
        let cap = usize::from(self.router.link_cap(to));
        let mut admitted = self.router.admission.admitted(from);
        admitted.sort_by(|a, b| {
            let ra = self.router.floor.get(*a).map(|e| e.rank);
            let rb = self.router.floor.get(*b).map(|e| e.rank);
            ra.cmp(&rb)
        });
        for k in admitted.into_iter().skip(cap) {
            self.router.admission.shed(to, k);
            let r = FloorReject {
                target: k.origin,
                target_sess: k.sess,
                burst: k.burst,
                reason: FloorRejectReason::Congestion,
                at: self.id.node,
            };
            let out = self
                .router
                .originate(now, PacketType::FloorReject, false, r.encode());
            self.apply(ctx, out);
        }
    }
}
