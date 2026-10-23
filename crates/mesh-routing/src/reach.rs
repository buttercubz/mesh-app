//! Presence, reachability, "who hears me", one-way members, islands and voice reach
//! (DESIGN §5.8).
//!
//! - A member is **reachable** if its PRESENCE was received within 6 s. Hops and path
//!   age come from the latest copy (`traversals = hops + 1`).
//! - `heard` bitmaps give **who hears me** (members whose bitmap has my bit), **one-way**
//!   members (I hear them, they do not hear me), and **islands** (connected components of
//!   the "hears" graph over the roster).
//! - **Voice reach**: `kpi` (≤ 2 BLE link traversals, or an all-Aware path, within the age
//!   cap), `bestEffort` (3-4 traversals within the voice TTL and age cap), `no`.
//! - Events: `memberLost` after 6 s of silence and `memberBack` on the first PRESENCE
//!   after a loss, aggregated over 5 s; `alone` when no member has been reachable for
//!   10 s; `partition` when the island count changes and stays changed for 6 s. A
//!   PRESENCE from a direct neighbor (hops 0) whose `roster_ver16`/`epoch16` is behind
//!   mine triggers the anti-entropy push (§8.9).
//! - Unconfirmed members (not in the roster) are tracked by `node_id` only.

use std::collections::{BTreeMap, BTreeSet};

use mesh_wire::mesh::Presence;

use crate::consts::KPI_MAX_TRAVERSALS;
use crate::types::{Instant, NodeId};

/// Voice reach class of a member (snapshot `voice`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VoiceReach {
    Kpi,
    BestEffort,
    No,
}

impl VoiceReach {
    /// Classifies a path of `traversals` link traversals with path age `age_us`.
    pub fn classify(
        traversals: u8,
        age_us: u64,
        all_aware: bool,
        voice_ttl: u8,
        max_age_us: u64,
    ) -> Self {
        if age_us > max_age_us || traversals == 0 {
            return VoiceReach::No;
        }
        if traversals <= KPI_MAX_TRAVERSALS || (all_aware && traversals <= voice_ttl) {
            VoiceReach::Kpi
        } else if traversals <= voice_ttl {
            VoiceReach::BestEffort
        } else {
            VoiceReach::No
        }
    }

    /// i18n/snapshot key.
    pub const fn key(self) -> &'static str {
        match self {
            VoiceReach::Kpi => "kpi",
            VoiceReach::BestEffort => "bestEffort",
            VoiceReach::No => "no",
        }
    }
}

/// Discrete events with i18n keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReachEvent {
    /// `memberLost` (aggregated list).
    MemberLost(Vec<NodeId>),
    /// `memberBack` (aggregated list).
    MemberBack(Vec<NodeId>),
    /// `alone`.
    Alone,
    /// `partition` with the new island count.
    Partition(usize),
    /// A direct neighbor is behind on roster or epoch: push anti-entropy (§8.9).
    PeerBehind(NodeId),
}

impl ReachEvent {
    pub const fn key(&self) -> &'static str {
        match self {
            ReachEvent::MemberLost(_) => "memberLost",
            ReachEvent::MemberBack(_) => "memberBack",
            ReachEvent::Alone => "alone",
            ReachEvent::Partition(_) => "partition",
            ReachEvent::PeerBehind(_) => "peerBehind",
        }
    }
}

/// Latest knowledge about one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberReach {
    pub node_id: NodeId,
    pub roster_index: Option<u8>,
    pub last_heard: Instant,
    pub sess: u8,
    /// Link traversals of the latest copy (`hops + 1`).
    pub traversals: u8,
    pub age_us: u64,
    pub all_aware: bool,
    pub presence: Presence,
    lost: bool,
}

/// Connected components of the "hears" graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslandView {
    /// Each island's members (ascending), mine first.
    pub islands: Vec<Vec<NodeId>>,
}

impl IslandView {
    pub fn count(&self) -> usize {
        self.islands.len()
    }
}

/// Reachability state of one node.
#[derive(Debug, Clone)]
pub struct Reachability {
    me: NodeId,
    roster: BTreeMap<NodeId, u8>,
    members: BTreeMap<NodeId, MemberReach>,
    my_roster_version: u32,
    my_epoch: u32,
    voice_ttl: u8,
    voice_max_age_us: u64,
    pending_lost: BTreeSet<NodeId>,
    pending_back: BTreeSet<NodeId>,
    agg_since: Option<Instant>,
    last_reachable: Instant,
    alone_sent: bool,
    islands_published: Option<usize>,
    islands_changed: Option<(Instant, usize)>,
}

#[path = "reach_impl.rs"]
mod imp;

#[cfg(test)]
#[path = "reach_tests.rs"]
mod tests;
