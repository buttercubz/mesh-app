//! The mesh glue state of one group session (created once the identity is known).

use std::collections::BTreeMap;

use mesh_routing::{LinkQueue, LinkSelector, NbrBuilder, NodePacing, Router, RouterConfig};
use mesh_wire::mesh::EncoderProfile;

use super::bundle::Bundles;
use super::identity::MeshIdentity;
use super::policy::MeshPolicy;
use super::seg::SegTracker;
use super::upgrade::Upgrades;
use crate::engine::Ctx;
use crate::{Instant, LinkId};

/// Counters of the glue (diagnostics and tests).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshStats {
    /// MESH records handed to the router.
    pub ingress: u64,
    /// My own packets echoed back by relays.
    pub own_echo: u64,
    /// Records over the per-neighbor ingress caps (not relayed).
    pub over_ingress_cap: u64,
    /// Relayed or originated records handed to the link layer.
    pub handed_off: u64,
    /// My own packets taken from the origination channel (`Ctx::originate`).
    pub originated: u64,
    /// My own voice frames one of my links refused (the burst was rejected locally).
    pub own_refused: u64,
    /// Originated packets dropped because their origin is not my group `node_id`.
    pub foreign_origin: u64,
    /// Voice frames dropped in the staging queues (per-origin limit, deadline).
    pub voice_dropped: u64,
    /// Bursts shed entirely on a link (stream-level shedding).
    pub bursts_shed: u64,
    /// NBR records sent / received.
    pub nbr_sent: u64,
    pub nbr_received: u64,
    /// Redundant links closed by the selection policy.
    pub closed_redundant: u64,
    /// PATH_HINTs originated.
    pub path_hints: u64,
    /// Voice copies sent on the old link during a bundle switch overlap.
    pub overlap_copies: u64,
}

/// `(slot, epoch, unreachable members, reachable members, BLE neighbors, lost
/// members)`.
pub(crate) type RadioView = (u32, u32, Vec<u16>, Vec<u16>, Vec<u16>, Vec<u16>);

/// Per-link bookkeeping of the glue.
#[derive(Debug, Clone)]
pub(crate) struct LinkMeta {
    pub kind: crate::links::LinkKind,
    pub role: crate::links::LinkRole,
    pub disc_id: Option<u32>,
    pub peer: u16,
    /// Staging queue (relayed packets and my own).
    pub queue: LinkQueue,
    /// Rate last applied to the link layer token bucket.
    pub applied_rate: Option<u32>,
    /// PING counters at the last loss observation.
    pub pings_sent: u64,
    pub pongs_received: u64,
    /// PINGs of the previous loss period still unanswered: a PONG may land in the
    /// period after its PING, so a PING is only counted lost one period later.
    pub pings_owed: u64,
    /// Last connection priority hint sent (Android GATT client links).
    pub high_priority: Option<bool>,
    /// Voice bursts staged on this link since they were last checked against the
    /// link's admitted set (evicted or shed bursts lose their queued frames).
    pub staged_bursts: std::collections::BTreeSet<mesh_routing::BurstKey>,
    /// The peer's CAPS (platform, transports), once received.
    pub peer_caps: Option<mesh_wire::record::Caps>,
}

/// The state of the glue while a session runs with a known identity.
pub(crate) struct Mesh {
    pub id: MeshIdentity,
    pub policy: MeshPolicy,
    pub router: Router,
    pub pacing: NodePacing,
    pub nbr: NbrBuilder,
    pub selector: LinkSelector,
    pub bundles: Bundles,
    pub upgrades: Upgrades,
    pub seg: SegTracker,
    pub links: BTreeMap<LinkId, LinkMeta>,
    /// Last PRESENCE heard per origin (reachability for the selection policy).
    pub heard: BTreeMap<u16, Instant>,
    /// PATH_HINT{BLE12} heard: BLE caps use the 12 kbps cost model until then.
    pub ble12_until: Option<Instant>,
    pub next_loss: Instant,
    /// Next link selection evaluation (every 2 s).
    pub next_select: Instant,
    /// The BLE PING period is the burst-time one (§5.1; Aware links always PING at
    /// 500 ms in the link layer, §9.6).
    pub fast_ping: bool,
    /// Links that carried voice in the last 2 s, as of the last check.
    pub voice_links: std::collections::BTreeSet<LinkId>,
    /// What the radio policy's §5.7 candidate classes were last built from:
    /// `(slot, epoch, unreachable members, reachable members, BLE neighbors, lost
    /// members)`.
    pub radio_view: Option<RadioView>,
    /// Last NBR sent (1 s heartbeat deadline).
    pub nbr_last: Option<Instant>,
    /// Next hand-off attempt of the staging queues.
    pub pump_at: Option<Instant>,
    pub stats: MeshStats,
}

impl Mesh {
    pub fn new(ctx: &mut Ctx, id: MeshIdentity) -> Mesh {
        let mut limits = ctx.cfg.limits.routing;
        limits.voice_ttl = id.voice_ttl;
        limits.leader_priority = id.leader_priority;
        let mut seed = [0u8; 32];
        ctx.fill(&mut seed);
        // The router's own counter is never seen on air: what it originates (relay
        // FLOOR_REJECT, PATH_HINT) takes its `seq` from the engine's shared per-origin
        // counter when the glue stages it (`Mesh::apply`), so every packet of
        // `(me, ctx.sess)` lives in one dedup window at every receiver (§5.3).
        let router = Router::new(RouterConfig {
            me: id.node,
            sess: ctx.sess,
            initial_seq: 0,
            epoch: id.epoch,
            limits,
            multi_hop: true,
            rng_seed: seed,
        });
        let now = ctx.now;
        Mesh {
            policy: MeshPolicy::for_platform(ctx.cfg.platform),
            router,
            pacing: NodePacing::new(limits),
            nbr: NbrBuilder::new(),
            selector: LinkSelector::new(id.node, limits, now.routing()),
            bundles: Bundles::new(),
            upgrades: Upgrades::new(),
            seg: SegTracker::new(),
            links: BTreeMap::new(),
            heard: BTreeMap::new(),
            ble12_until: None,
            next_loss: now.plus_us(super::LOSS_PERIOD_US),
            next_select: now,
            fast_ping: false,
            voice_links: Default::default(),
            radio_view: None,
            nbr_last: None,
            pump_at: None,
            stats: MeshStats::default(),
            id,
        }
    }

    /// Applies an identity change that keeps the node id (epoch, leader, policy,
    /// roster): floor hash epoch and leader priority follow the group.
    pub fn update_identity(&mut self, id: MeshIdentity) {
        self.router.cfg.epoch = id.epoch;
        self.router.floor.epoch = id.epoch;
        self.router.floor.leader_priority = id.leader_priority;
        self.router.cfg.limits.leader_priority = id.leader_priority;
        self.router.cfg.limits.voice_ttl = id.voice_ttl;
        self.id = id;
    }

    /// The BLE cost model follows PATH_HINT{BLE12} for 30 s.
    pub fn sync_profile(&mut self, now: Instant) {
        let ble12 = self.ble12_until.is_some_and(|t| now < t);
        self.pacing.ble_profile = if ble12 {
            EncoderProfile::Ble12
        } else {
            EncoderProfile::Ble16
        };
        if !ble12 {
            self.ble12_until = None;
        }
    }

    /// Roster members (other than me) reachable now / unreachable now.
    pub fn reachability(&self, now: Instant) -> (Vec<u16>, Vec<u16>) {
        let mut reach = Vec::new();
        let mut unreach = Vec::new();
        for &m in self.id.roster.iter().filter(|m| **m != self.id.node) {
            let heard = self
                .heard
                .get(&m)
                .is_some_and(|t| now.since(*t) < super::REACHABLE_US);
            let neighbor = self.router.neighbors.neighbor(m).is_some();
            if heard || neighbor {
                reach.push(m);
            } else {
                unreach.push(m);
            }
        }
        (reach, unreach)
    }
}
