//! Link selection policy, evaluated every 2 s (DESIGN §5.7, §9.3, §9.5).
//!
//! - **Targets:** BLE links target 2, max 3 per node (from [`Limits`]). The max is lowered
//!   by 1 for 10 min after each `LimitReached` link failure (floor 1). Aware max
//!   `min(4, available data paths)`. Distinct neighbors max 6.
//! - **Under target, or some roster member unreachable:** discovery `Forming`; connect to
//!   candidates ordered by (a) an advertising token that maps to an unreachable member,
//!   (b) an unknown token, (c) RSSI. One outstanding connect per transport, 8 s attempt
//!   timeout, per-peer backoff 5 s → 60 s (doubling), reset on a new adv token.
//!   A **speculative** link to a peer whose HELLO `epoch_lo` is 1-4 epochs ahead (tag not
//!   verifiable) has the lowest priority, at most one per 30 s and a 60 s per-peer backoff
//!   on failure.
//! - **Over max:** close the link to a neighbor that appears `good` in another neighbor's
//!   fresh NBR (redundant) with the worst `q`, never one that is the only path to a
//!   reachable member (articulation check on the 2-hop graph).
//! - **Duplicate links** (same node and kind): keep the link whose initiator has the lower
//!   `node_id` ([`resolve_duplicate`]).

use std::collections::BTreeMap;

use crate::consts::{
    BACKOFF_INITIAL_US, BACKOFF_MAX_US, CONNECT_TIMEOUT_US, LIMIT_BACKOFF_US, SELECT_PERIOD_US,
    SPECULATIVE_BACKOFF_US, SPECULATIVE_INTERVAL_US,
};
use crate::limits::Limits;
use crate::neighbor::NeighborTable;
use crate::types::{Instant, LinkId, LinkKind, LinkRole, NodeId};

mod graph;

pub use graph::{reachable_without, two_hop_graph};

/// How a discovered peer relates to the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateClass {
    /// Its advertising token maps to a roster member that is currently unreachable.
    UnreachableMember,
    /// Unknown token (e.g. a backgrounded iPhone).
    Unknown,
    /// A reachable member we are not linked to (extra path).
    ReachableMember,
    /// HELLO `epoch_lo` 1-4 epochs ahead and the tag did not verify (§9.3).
    Speculative,
}

/// A discovered, connectable peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub disc_id: u32,
    /// Stable reference for backoff: the advertising token (changes on rotation).
    pub peer_ref: u64,
    /// Roster member the token maps to, if any.
    pub node: Option<NodeId>,
    pub class: CandidateClass,
    pub rssi: i16,
    pub transport: LinkKind,
}

/// Inputs of one evaluation.
#[derive(Debug, Clone, Default)]
pub struct SelectInput {
    pub candidates: Vec<Candidate>,
    /// Roster members (other than me) currently unreachable.
    pub unreachable_members: Vec<NodeId>,
    /// Roster members currently reachable (articulation check targets).
    pub reachable_members: Vec<NodeId>,
    /// Available Wi-Fi Aware data paths (0 without Aware).
    pub aware_data_paths: u8,
}

/// Discovery mode (`RadioCommand::SetDiscovery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryMode {
    Forming,
    Stable,
}

/// What the core should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectAction {
    SetDiscovery(DiscoveryMode),
    Connect {
        disc_id: u32,
        speculative: bool,
    },
    /// Abort an attempt that exceeded the 8 s timeout.
    Abort {
        disc_id: u32,
    },
    /// Close a redundant link (`CLOSE{9}`).
    CloseRedundant {
        link: LinkId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Backoff {
    until: Instant,
    current_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Attempt {
    disc_id: u32,
    peer_ref: u64,
    speculative: bool,
    started: Instant,
}

/// The selector of one node.
#[derive(Debug, Clone)]
pub struct LinkSelector {
    pub me: NodeId,
    pub limits: Limits,
    limit_penalties: Vec<Instant>,
    backoff: BTreeMap<u64, Backoff>,
    outstanding: BTreeMap<LinkKind, Attempt>,
    last_speculative: Option<Instant>,
    next_eval: Instant,
    mode: Option<DiscoveryMode>,
}

/// Which of two links to the same node and kind to close: keep the one whose initiator
/// has the lower `node_id` (equal initiators: keep the lower link id).
pub fn resolve_duplicate(
    me: NodeId,
    peer: NodeId,
    a: (LinkId, LinkRole),
    b: (LinkId, LinkRole),
) -> LinkId {
    let initiator = |r: LinkRole| match r {
        LinkRole::Initiator => me,
        LinkRole::Responder => peer,
    };
    let ka = (initiator(a.1), a.0);
    let kb = (initiator(b.1), b.0);
    if ka <= kb { b.0 } else { a.0 }
}

#[path = "select_impl.rs"]
mod imp;

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;

impl LinkSelector {
    pub fn new(me: NodeId, limits: Limits, now: Instant) -> Self {
        LinkSelector {
            me,
            limits,
            limit_penalties: Vec::new(),
            backoff: BTreeMap::new(),
            outstanding: BTreeMap::new(),
            last_speculative: None,
            next_eval: now,
            mode: None,
        }
    }

    /// Current BLE link max: `ble_links_max − active LimitReached penalties`, floor 1.
    pub fn ble_max(&self, now: Instant) -> u8 {
        let active = self.limit_penalties.iter().filter(|t| now < **t).count();
        let m = usize::from(self.limits.ble_links_max).saturating_sub(active);
        m.max(1) as u8
    }

    /// A BLE link failed with `LimitReached`: max −1 for 10 min.
    pub fn on_limit_reached(&mut self, now: Instant) {
        self.limit_penalties.retain(|t| now < *t);
        self.limit_penalties.push(now.plus_us(LIMIT_BACKOFF_US));
    }

    /// The peer advertised a new token: its backoff resets.
    pub fn on_new_token(&mut self, peer_ref: u64) {
        self.backoff.remove(&peer_ref);
    }

    /// Outcome of an attempt started with [`SelectAction::Connect`].
    pub fn on_connect_result(&mut self, now: Instant, disc_id: u32, ok: bool) {
        let Some((&kind, &a)) = self.outstanding.iter().find(|(_, a)| a.disc_id == disc_id) else {
            return;
        };
        self.outstanding.remove(&kind);
        if ok {
            self.backoff.remove(&a.peer_ref);
        } else {
            self.fail(now, a);
        }
    }

    fn fail(&mut self, now: Instant, a: Attempt) {
        let next = if a.speculative {
            SPECULATIVE_BACKOFF_US
        } else {
            match self.backoff.get(&a.peer_ref) {
                Some(b) => (b.current_us * 2).min(BACKOFF_MAX_US),
                None => BACKOFF_INITIAL_US,
            }
        };
        self.backoff.insert(
            a.peer_ref,
            Backoff {
                until: now.plus_us(next),
                current_us: next,
            },
        );
    }

    fn in_backoff(&self, now: Instant, peer_ref: u64) -> bool {
        self.backoff.get(&peer_ref).is_some_and(|b| now < b.until)
    }

    /// `true` when an evaluation is due (every 2 s).
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next_eval
    }

    fn schedule(&mut self, now: Instant) {
        self.next_eval = now.plus_us(SELECT_PERIOD_US);
    }

    fn timeouts(&mut self, now: Instant, out: &mut Vec<SelectAction>) {
        let expired: Vec<(LinkKind, Attempt)> = self
            .outstanding
            .iter()
            .filter(|(_, a)| now.since(a.started) >= CONNECT_TIMEOUT_US)
            .map(|(k, a)| (*k, *a))
            .collect();
        for (k, a) in expired {
            self.outstanding.remove(&k);
            self.fail(now, a);
            out.push(SelectAction::Abort { disc_id: a.disc_id });
        }
    }

    /// Links of `kind` I have now.
    fn count(table: &NeighborTable, kind: LinkKind) -> usize {
        table.count_kind(kind)
    }

    fn speculative_allowed(&self, now: Instant) -> bool {
        self.last_speculative
            .is_none_or(|t| now.since(t) >= SPECULATIVE_INTERVAL_US)
    }
}
