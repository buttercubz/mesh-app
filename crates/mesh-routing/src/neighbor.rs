//! Neighbor table (DESIGN §5.1): keyed by `node_id` after IDPROOF, one link bundle per
//! neighbor (≤ 1 link per kind), link quality, congested flag, the latest NBR report,
//! liveness (any inbound frame), penalties with a 60 s ban and per-neighbor ingress caps.

use std::collections::{BTreeMap, VecDeque};

use mesh_wire::record::Nbr;

use crate::consts::{
    BAN_US, CONGESTED_HOLD_US, INGRESS_BYTES_PER_S, INGRESS_RECORDS_PER_S, LINK_DEATH_US,
    PENALTY_LIMIT_PER_MIN, PENALTY_WINDOW_US, Q_GOOD, VOICE_LINK_WINDOW_US,
};
use crate::nbr::{NbrReport, OwnLinkEntry};
use crate::quality::LinkQuality;
use crate::types::{Instant, LinkId, LinkKind, LinkRole, NodeId};

/// One link of a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkState {
    pub id: LinkId,
    pub kind: LinkKind,
    pub role: LinkRole,
    pub max_frame: u16,
    pub quality: LinkQuality,
    pub congested_until: Option<Instant>,
    pub last_rx: Instant,
    pub up_at: Instant,
    /// Last time a voice frame was sent on this link (voice-link classification, §5.6).
    pub last_voice_tx: Option<Instant>,
    /// Published per-link stream cap (`cap(L)`, advertised as `vcap`).
    pub vcap: u8,
}

impl LinkState {
    /// `q` now.
    pub fn q(&self, now: Instant) -> u8 {
        self.quality.q(now)
    }

    /// Congested flag (2 s hold).
    pub fn congested(&self, now: Instant) -> bool {
        self.congested_until.is_some_and(|t| now < t)
    }

    /// Carried voice in the last 2 s.
    pub fn is_voice_link(&self, now: Instant) -> bool {
        self.last_voice_tx
            .is_some_and(|t| now.since(t) < VOICE_LINK_WINDOW_US)
    }
}

/// Up to one link per kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkBundle {
    slots: [Option<LinkState>; 3],
}

impl LinkBundle {
    pub fn get(&self, kind: LinkKind) -> Option<&LinkState> {
        self.slots[kind.index()].as_ref()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LinkState> {
        self.slots.iter().flatten()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bucket {
    tokens: u64,
    rate: u64,
    last: Instant,
}

impl Bucket {
    fn new(rate: u64, now: Instant) -> Self {
        Bucket {
            tokens: rate,
            rate,
            last: now,
        }
    }

    fn take(&mut self, now: Instant, n: u64) -> bool {
        let refill = self.rate * now.since(self.last) / 1_000_000;
        self.tokens = (self.tokens + refill).min(self.rate);
        self.last = now;
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

/// A neighbor (one node, after IDPROOF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbor {
    pub node_id: NodeId,
    pub links: LinkBundle,
    pub nbr: Option<NbrReport>,
    /// CAPS facts (opaque here): platform, role.
    pub platform: Option<u8>,
    pub role: Option<u8>,
    pub last_rx: Instant,
    penalties: VecDeque<(Instant, u32)>,
    ingress_records: Bucket,
    ingress_bytes: Bucket,
}

impl Neighbor {
    fn new(node_id: NodeId, now: Instant) -> Self {
        Neighbor {
            node_id,
            links: LinkBundle::default(),
            nbr: None,
            platform: None,
            role: None,
            last_rx: now,
            penalties: VecDeque::new(),
            ingress_records: Bucket::new(u64::from(INGRESS_RECORDS_PER_S), now),
            ingress_bytes: Bucket::new(u64::from(INGRESS_BYTES_PER_S), now),
        }
    }

    /// The fresh NBR report, if any.
    pub fn fresh_nbr(&self, now: Instant) -> Option<&NbrReport> {
        self.nbr.as_ref().filter(|r| r.fresh(now))
    }

    /// Penalty points within the last minute.
    pub fn penalty(&self, now: Instant) -> u32 {
        self.penalties
            .iter()
            .filter(|(t, _)| now.since(*t) < PENALTY_WINDOW_US)
            .map(|(_, p)| *p)
            .sum()
    }

    /// Best link for traffic: uncongested first, then good, then higher-capacity kind,
    /// then higher `q`, then lower link id.
    pub fn best_link(&self, now: Instant) -> Option<&LinkState> {
        self.links.iter().max_by(|a, b| {
            let ka = (
                !a.congested(now),
                a.q(now) >= Q_GOOD,
                a.kind.preference(),
                a.q(now),
            );
            let kb = (
                !b.congested(now),
                b.q(now) >= Q_GOOD,
                b.kind.preference(),
                b.q(now),
            );
            ka.cmp(&kb).then(b.id.cmp(&a.id))
        })
    }
}

/// Why `link_up` refused a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkUpError {
    /// The neighbor already has a link of this kind (duplicate, resolve with
    /// [`crate::select::resolve_duplicate`]).
    Duplicate(LinkId),
    /// The link id is already registered.
    KnownLink,
    /// The identity is banned until the given instant.
    Banned(Instant),
}

/// The neighbor table.
#[derive(Debug, Clone, Default)]
pub struct NeighborTable {
    neighbors: BTreeMap<NodeId, Neighbor>,
    by_link: BTreeMap<LinkId, (NodeId, LinkKind)>,
    bans: BTreeMap<NodeId, Instant>,
}

impl NeighborTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an authenticated link to `node` (after IDPROOF).
    #[allow(clippy::too_many_arguments)]
    pub fn link_up(
        &mut self,
        now: Instant,
        node: NodeId,
        link: LinkId,
        kind: LinkKind,
        role: LinkRole,
        max_frame: u16,
    ) -> Result<(), LinkUpError> {
        if let Some(until) = self.banned_until(now, node) {
            return Err(LinkUpError::Banned(until));
        }
        if self.by_link.contains_key(&link) {
            return Err(LinkUpError::KnownLink);
        }
        let n = self
            .neighbors
            .entry(node)
            .or_insert_with(|| Neighbor::new(node, now));
        if let Some(existing) = &n.links.slots[kind.index()] {
            return Err(LinkUpError::Duplicate(existing.id));
        }
        n.links.slots[kind.index()] = Some(LinkState {
            id: link,
            kind,
            role,
            max_frame,
            quality: LinkQuality::new(kind),
            congested_until: None,
            last_rx: now,
            up_at: now,
            last_voice_tx: None,
            vcap: kind.s_max(),
        });
        n.last_rx = now;
        self.by_link.insert(link, (node, kind));
        Ok(())
    }

    /// Removes a link; the neighbor goes away with its last link.
    pub fn link_down(&mut self, link: LinkId) -> Option<(NodeId, LinkKind)> {
        let (node, kind) = self.by_link.remove(&link)?;
        if let Some(n) = self.neighbors.get_mut(&node) {
            n.links.slots[kind.index()] = None;
            if n.links.is_empty() {
                self.neighbors.remove(&node);
            }
        }
        Some((node, kind))
    }

    /// Node at the other end of `link`.
    pub fn node_of(&self, link: LinkId) -> Option<NodeId> {
        self.by_link.get(&link).map(|(n, _)| *n)
    }

    pub fn neighbor(&self, node: NodeId) -> Option<&Neighbor> {
        self.neighbors.get(&node)
    }

    pub fn neighbor_mut(&mut self, node: NodeId) -> Option<&mut Neighbor> {
        self.neighbors.get_mut(&node)
    }

    pub fn neighbors(&self) -> impl Iterator<Item = &Neighbor> {
        self.neighbors.values()
    }

    pub fn len(&self) -> usize {
        self.neighbors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.neighbors.is_empty()
    }

    /// The state of `link`.
    pub fn link(&self, link: LinkId) -> Option<&LinkState> {
        let (node, kind) = self.by_link.get(&link)?;
        self.neighbors.get(node)?.links.get(*kind)
    }

    /// Mutable state of `link`.
    pub fn link_mut(&mut self, link: LinkId) -> Option<&mut LinkState> {
        let (node, kind) = self.by_link.get(&link)?;
        self.neighbors.get_mut(node)?.links.slots[kind.index()].as_mut()
    }

    /// Every link (ascending link id).
    pub fn link_ids(&self) -> impl Iterator<Item = LinkId> + '_ {
        self.by_link.keys().copied()
    }

    /// Liveness and ingress caps for one inbound frame carrying `records` records and
    /// `bytes` bytes. Returns `false` when the frame exceeds the neighbor's ingress caps
    /// (300 records/s, 64 KB/s): the caller drops it.
    pub fn on_rx(&mut self, now: Instant, link: LinkId, records: u32, bytes: u32) -> bool {
        let Some(&(node, kind)) = self.by_link.get(&link) else {
            return false;
        };
        let Some(n) = self.neighbors.get_mut(&node) else {
            return false;
        };
        n.last_rx = now;
        if let Some(l) = n.links.slots[kind.index()].as_mut() {
            l.last_rx = now;
        }
        let ok_r = n.ingress_records.take(now, u64::from(records));
        let ok_b = n.ingress_bytes.take(now, u64::from(bytes));
        ok_r && ok_b
    }

    /// RTT sample from a PONG on `link`.
    pub fn on_rtt(&mut self, now: Instant, link: LinkId, rtt_us: u64) {
        if let Some(l) = self.link_mut(link) {
            l.quality.rtt.sample(now, rtt_us);
        }
    }

    /// Platform metrics.
    pub fn on_rssi(&mut self, link: LinkId, rssi: Option<i16>) {
        if let Some(l) = self.link_mut(link) {
            l.quality.rssi = rssi;
        }
    }

    /// SEC counter gaps / PING loss observations.
    pub fn on_loss(&mut self, link: LinkId, expected: u32, lost: u32) {
        if let Some(l) = self.link_mut(link) {
            l.quality.loss.observe(expected, lost);
        }
    }

    /// A voice frame was dropped on `link` or a `WouldBlock` was returned: congested
    /// for 2 s.
    pub fn mark_congested(&mut self, now: Instant, link: LinkId) {
        if let Some(l) = self.link_mut(link) {
            l.congested_until = Some(now.plus_us(CONGESTED_HOLD_US));
        }
    }

    /// A voice frame left on `link` (voice-link classification).
    pub fn mark_voice_tx(&mut self, now: Instant, link: LinkId) {
        if let Some(l) = self.link_mut(link) {
            l.last_voice_tx = Some(now);
        }
    }

    /// Stores the published cap of `link` (`vcap` in our NBR).
    pub fn set_vcap(&mut self, link: LinkId, vcap: u8) {
        if let Some(l) = self.link_mut(link) {
            l.vcap = vcap;
        }
    }

    /// Ingests an NBR record received on `link`.
    pub fn on_nbr(&mut self, now: Instant, link: LinkId, nbr: &Nbr) {
        let Some(node) = self.node_of(link) else {
            return;
        };
        if let Some(n) = self.neighbors.get_mut(&node) {
            n.nbr = Some(NbrReport::from_record(now, nbr));
        }
    }

    /// Adds penalty points; returns `true` (and bans the identity for 60 s) when the
    /// neighbor reaches 20 points within a minute. The caller disconnects it.
    pub fn penalize(&mut self, now: Instant, node: NodeId, points: u32) -> bool {
        let Some(n) = self.neighbors.get_mut(&node) else {
            return false;
        };
        n.penalties.push_back((now, points));
        while n
            .penalties
            .front()
            .is_some_and(|(t, _)| now.since(*t) >= PENALTY_WINDOW_US)
        {
            n.penalties.pop_front();
        }
        if n.penalty(now) >= PENALTY_LIMIT_PER_MIN {
            self.bans.insert(node, now.plus_us(BAN_US));
            true
        } else {
            false
        }
    }

    /// The ban expiry of `node`, if banned now.
    pub fn banned_until(&self, now: Instant, node: NodeId) -> Option<Instant> {
        self.bans.get(&node).copied().filter(|t| now < *t)
    }

    /// Links with no inbound frame for 4 s (the caller closes them).
    pub fn dead_links(&self, now: Instant) -> Vec<LinkId> {
        self.neighbors
            .values()
            .flat_map(|n| n.links.iter())
            .filter(|l| now.since(l.last_rx) >= LINK_DEATH_US)
            .map(|l| l.id)
            .collect()
    }

    /// Per-second housekeeping: loss EWMA ticks, expired bans.
    pub fn tick(&mut self, now: Instant) {
        for n in self.neighbors.values_mut() {
            for l in n.links.slots.iter_mut().flatten() {
                l.quality.loss.tick(now);
            }
        }
        self.bans.retain(|_, t| now < *t);
    }

    /// Our own link entries for the NBR builder.
    pub fn own_entries(&self, now: Instant) -> Vec<OwnLinkEntry> {
        self.neighbors
            .values()
            .flat_map(|n| n.links.iter().map(move |l| (n.node_id, l)))
            .map(|(node, l)| {
                let q = l.q(now);
                OwnLinkEntry {
                    node_id: node,
                    kind: l.kind,
                    q,
                    good: q >= Q_GOOD,
                    congested: l.congested(now),
                    vcap: l.vcap,
                }
            })
            .collect()
    }

    /// Number of links of `kind`.
    pub fn count_kind(&self, kind: LinkKind) -> usize {
        self.by_link.values().filter(|(_, k)| *k == kind).count()
    }
}

#[cfg(test)]
#[path = "neighbor_tests.rs"]
mod tests;
