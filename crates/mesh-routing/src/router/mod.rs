//! Stateless managed flooding with immediate voice relay (DESIGN §5.2, §5.5).
//!
//! ```text
//! on authenticated MESH record P from neighbor N over link L:
//!   if P.version != 1 or malformed            → drop; N.penalty += 1
//!   if P.hops == 0 and P.origin != N.node_id  → drop; N.penalty += 5
//!   if dedup.seen(P.origin, P.sess, P.seq)    → stats.dup += 1; return
//!   dedup.mark(P.origin, P.sess, P.seq)
//!   age_now = P.age*2 + owd(L) + residence
//!   deliver_locally(P)                         # unless ptype unknown
//!   if P.ttl_remaining <= 1 → return
//!   if P.class == voice and age_now > VOICE_MAX_AGE → stats.stale += 1; return
//!   for each neighbor M ∉ {N, P.origin}, using M's best link for P.class:
//!       if P.class != voice and pruned(N, M, P) → continue
//!       if P.class == voice and !floor.admits(link(M), burst(P)) → maybe FLOOR_REJECT; continue
//!       enqueue(link(M), P with ttl−1, hops+1; age rewritten at dequeue)
//! ```
//!
//! Origination goes to **all** neighbors (no pruning). Voice relays at once; control
//! and bulk wait a random 5-25 ms (records coalesce) with counter suppression: if the
//! same key arrived from ≥ 3 distinct neighbors before the jittered send, the pending
//! copy is dropped for the neighbors that already sent it to us. Pruning is evaluated
//! at release time against the then-current NBR.

mod egress;
mod ingress;

use std::collections::{BTreeMap, BTreeSet};

use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

use crate::dedup::Dedup;
use crate::floor::{Admission, BurstKey, FloorEvent, FloorTable};
use crate::limits::Limits;
use crate::neighbor::NeighborTable;
use crate::prune::SegTimeouts;
use crate::queue::{Priority, QueuedPacket};
use crate::types::{Instant, LinkId, LinkKind, NodeId};

use mesh_wire::mesh::{EncoderProfile, FloorRejectReason, TrafficClass};

/// A MESH record body taken from a frame whose AEAD tag verified (SEC) or a LAB frame on
/// a Lab link. Routing state never changes for anything else (§4.1, §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authenticated<'a> {
    bytes: &'a [u8],
}

impl<'a> Authenticated<'a> {
    /// The caller asserts that `bytes` came out of a verified SEC frame (or a LAB frame
    /// on a Lab link).
    pub fn after_verified_frame(bytes: &'a [u8]) -> Self {
        Authenticated { bytes }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// Router configuration.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub me: NodeId,
    /// Origin session tag (random non-zero per engine start).
    pub sess: u8,
    /// Random initial `seq`.
    pub initial_seq: u16,
    /// Current group epoch (floor hash input).
    pub epoch: u32,
    pub limits: Limits,
    /// Phase 1 "direct delivery" sets this to `false` (no relaying).
    pub multi_hop: bool,
    /// Seed of the injected ChaCha20 RNG (relay jitter).
    pub rng_seed: [u8; 32],
}

/// A packet to enqueue on a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub link: LinkId,
    pub prio: Priority,
    pub packet: QueuedPacket,
}

/// Something the core must act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterAction {
    /// The neighbor reached 20 penalty points in a minute: close its links; it is banned
    /// until `until`.
    Disconnect { node: NodeId, until: Instant },
    /// Floor table change (snapshot `talkers[]`, mixer selection).
    Floor(FloorEvent),
    /// One of my bursts was refused (remotely or by my own link admission): the origin
    /// talk FSM goes BLOCKED (§6.4).
    RejectedMe {
        burst: u8,
        reason: FloorRejectReason,
    },
    /// A PATH_HINT for me (or all): use `profile` for 30 s (§5.6, §7.4).
    PathHint {
        profile: EncoderProfile,
        from: NodeId,
    },
}

/// Result of one inbound mesh packet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteOutcome {
    /// Deliver locally (header of the packet as received; the core parses the payload).
    pub deliver: Option<mesh_wire::mesh::MeshHeader>,
    /// Immediate forwards and originations (voice, my FLOOR_REJECTs).
    pub out: Vec<Outgoing>,
    pub actions: Vec<RouterAction>,
}

/// Counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RouterStats {
    pub delivered: u64,
    pub forwarded: u64,
    pub originated: u64,
    pub dup: u64,
    pub too_old: u64,
    pub stale: u64,
    pub malformed: u64,
    pub spoofed: u64,
    pub pruned: u64,
    pub suppressed: u64,
    pub floor_refused: u64,
    pub rejects_sent: u64,
    pub unknown_link: u64,
}

#[derive(Debug, Clone)]
struct PendingCtrl {
    due: Instant,
    key: (NodeId, u8, u16),
    from: NodeId,
    class: TrafficClass,
    seg_msg: Option<u16>,
    packet: QueuedPacket,
    heard_from: BTreeSet<NodeId>,
}

/// The router of one node.
#[derive(Debug, Clone)]
pub struct Router {
    pub cfg: RouterConfig,
    pub neighbors: NeighborTable,
    pub dedup: Dedup,
    pub floor: FloorTable,
    pub admission: Admission,
    pub seg_timeouts: SegTimeouts,
    /// Published `cap_pacing` per link (from the cap estimator).
    link_caps: BTreeMap<LinkId, u8>,
    pending: Vec<PendingCtrl>,
    seq: u16,
    rng: ChaCha20Rng,
    pub stats: RouterStats,
}

impl Router {
    pub fn new(cfg: RouterConfig) -> Self {
        let floor = FloorTable::new(cfg.epoch, cfg.limits.t_tie_us, cfg.limits.leader_priority);
        Router {
            admission: Admission::new(cfg.limits.t_sticky_us),
            seq: cfg.initial_seq,
            rng: ChaCha20Rng::from_seed(cfg.rng_seed),
            floor,
            neighbors: NeighborTable::new(),
            dedup: Dedup::new(),
            seg_timeouts: SegTimeouts::new(),
            link_caps: BTreeMap::new(),
            pending: Vec::new(),
            stats: RouterStats::default(),
            cfg,
        }
    }

    pub fn me(&self) -> NodeId {
        self.cfg.me
    }

    /// Sets the published `cap_pacing` of a link.
    pub fn set_link_cap(&mut self, link: LinkId, cap_pacing: u8) {
        self.link_caps.insert(link, cap_pacing);
        let cap = self.link_cap(link);
        self.neighbors.set_vcap(link, cap);
    }

    /// `cap(L) = min(S_MAX[kind], cap_pacing, override)`.
    pub fn link_cap(&self, link: LinkId) -> u8 {
        let Some(l) = self.neighbors.link(link) else {
            return 0;
        };
        let pacing = self.link_caps.get(&link).copied().unwrap_or(l.kind.s_max());
        self.cfg.limits.link_cap(l.kind, pacing)
    }

    /// `cap_pacing(L) < S_MAX[kind]` (the CONGESTION reject reason).
    fn congestion(&self, link: LinkId) -> bool {
        match (self.neighbors.link(link), self.link_caps.get(&link)) {
            (Some(l), Some(&c)) => c < self.cfg.limits.s_max(l.kind),
            _ => false,
        }
    }

    /// `cap_local`: the cap of my most constrained voice link (S_MAX BLE without links).
    /// BLE edge links ([`Router::is_edge_link`]) do not count: a full edge sheds my burst
    /// on that link only, it never blocks me on the Wi-Fi backbone. BLE fallback links to
    /// a neighbor I also hold an Aware link to ([`Router::is_ble_fallback_link`]) do not
    /// count either: the bundle sends voice to that neighbor over Aware only.
    pub fn cap_local(&self) -> u8 {
        self.neighbors
            .link_ids()
            .filter(|l| !self.is_edge_link(*l) && !self.is_ble_fallback_link(*l))
            .map(|l| self.link_cap(l))
            .min()
            .unwrap_or(self.cfg.limits.s_max_ble)
    }

    /// `true` when `link` is a BLE link to a neighbor I also hold an Aware link to (the
    /// fallback of an upgraded link; it carries no voice while the Aware link is up).
    pub fn is_ble_fallback_link(&self, link: LinkId) -> bool {
        self.neighbors
            .link(link)
            .is_some_and(|l| l.kind == LinkKind::Ble)
            && self
                .neighbors
                .node_of(link)
                .and_then(|n| self.neighbors.neighbor(n))
                .is_some_and(|n| n.links.get(LinkKind::Aware).is_some())
    }

    /// `true` when `link` is a BLE *edge* of a Wi-Fi backbone node: a BLE link while I
    /// hold an Aware link to another neighbor (hybrid groups: Aware is the backbone,
    /// BLE-only members hang off it). Admission on an edge is local to it (shed, never
    /// a group-wide FLOOR_REJECT, see [`crate::floor::Admission::admits_at`]).
    pub fn is_edge_link(&self, link: LinkId) -> bool {
        let Some(l) = self.neighbors.link(link) else {
            return false;
        };
        if l.kind != LinkKind::Ble {
            return false;
        }
        let peer = self.neighbors.node_of(link);
        self.neighbors
            .neighbors()
            .any(|m| Some(m.node_id) != peer && m.links.get(LinkKind::Aware).is_some())
    }

    /// Whether the neighbor behind `link` belongs to the Wi-Fi backbone: the link itself
    /// is Aware, I hold an Aware link to that neighbor, or its latest NBR lists an Aware
    /// link of its own.
    pub fn backbone_peer(&self, link: LinkId) -> bool {
        if self
            .neighbors
            .link(link)
            .is_some_and(|l| l.kind == LinkKind::Aware)
        {
            return true;
        }
        self.neighbors
            .node_of(link)
            .and_then(|n| self.neighbors.neighbor(n))
            .is_some_and(|n| {
                n.links.get(LinkKind::Aware).is_some()
                    || n.nbr
                        .as_ref()
                        .is_some_and(|r| r.entries.iter().any(|e| e.kind == Some(LinkKind::Aware)))
            })
    }

    /// The near-talker `group_cap` for VOICE received on `link`, per segment (§6.3): the
    /// Aware cap when the voice comes from the Wi-Fi backbone, the BLE cap inside a BLE
    /// segment. A group-wide "any BLE link" cap would throttle an Aware island to 2.
    pub fn near_talker_cap(&self, link: LinkId) -> u8 {
        self.cfg.limits.group_cap(!self.backbone_peer(link))
    }

    /// The origin's `local_admit` for burst `burst` with priority `prio` (§6.4).
    pub fn local_admit(&self, now: Instant, burst: u8, prio: u8) -> bool {
        let key = BurstKey::new(self.cfg.me, self.cfg.sess, burst);
        self.floor.local_admit(now, key, prio, self.cap_local())
    }

    /// A link went down: forget its admission state and caps.
    pub fn on_link_down(&mut self, link: LinkId) {
        self.neighbors.link_down(link);
        self.admission.on_link_down(link);
        self.link_caps.remove(&link);
    }

    /// The queue of `link` shed `burst` (stream-level shedding, §5.6): no further frames
    /// of it on that link until it ends; the link is congested for 2 s.
    pub fn on_shed(&mut self, now: Instant, link: LinkId, burst: BurstKey) {
        self.admission.shed(link, burst);
        self.neighbors.mark_congested(now, link);
    }

    /// Housekeeping: floor idle release, dedup and timeout expiry, loss ticks.
    pub fn tick(&mut self, now: Instant) -> Vec<RouterAction> {
        let ev = self.floor.expire(now);
        let mut out = Vec::new();
        for e in ev {
            if let FloorEvent::Ended(k, _) = e {
                self.admission.on_burst_ended(k);
            }
            out.push(RouterAction::Floor(e));
        }
        self.dedup.expire(now);
        self.seg_timeouts.expire(now);
        self.neighbors.tick(now);
        out
    }

    /// Earliest pending control release.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.iter().map(|p| p.due).min()
    }

    fn next_seq(&mut self) -> u16 {
        let s = self.seq;
        self.seq = self.seq.wrapping_add(1);
        s
    }
}

#[cfg(test)]
mod tests;
