//! Relay admission per outgoing link (DESIGN §6.3, hard enforcement):
//!
//! ```text
//! cap(L) = min(S_MAX[kind(L)], cap_pacing(L), group_cap_override?)
//! admits(L, b):
//!   if b ∈ admitted(L): refresh; return true
//!   if |admitted(L)| < cap(L): admit; return true
//!   victim = lowest-ranked entry of admitted(L)
//!   if b.prio > victim.prio
//!      or (b ranks above victim and now − victim.admitted_at < T_STICKY):
//!        evict victim; flood FLOOR_REJECT{victim, PREEMPTED | OUTRANKED}; admit b; true
//!   if (b not yet rejected by me): flood FLOOR_REJECT{b, CAP (or CONGESTION when
//!                                   cap_pacing < S_MAX), at = me}
//!   return false
//! ```
//!
//! Relays never forward voice of a burst outside `admits(L, b)` on L, so even if every
//! talker ignored FLOOR_REJECT no link carries more than its cap. Stream-level shedding
//! (§5.6) removes the lowest-ranked admitted burst of a link entirely: it is not
//! re-admitted on that link until it ends. When `cap(L)` drops below the admitted count,
//! the lowest-ranked bursts are shed the same way.

use std::collections::{BTreeMap, BTreeSet};

use mesh_wire::mesh::FloorRejectReason;

use super::rank::BurstKey;
use super::table::FloorTable;
use crate::types::{Instant, LinkId};

/// Outcome of `admits(L, b)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitDecision {
    /// Forward on L.
    Admitted,
    /// Forward on L; `victim` was evicted from L and must be rejected with `reason`.
    AdmittedEvicting {
        victim: BurstKey,
        reason: FloorRejectReason,
    },
    /// Do not forward on L. `reject` is set the first time this node refuses `b`.
    Refused { reject: Option<FloorRejectReason> },
}

impl AdmitDecision {
    pub fn admitted(&self) -> bool {
        !matches!(self, AdmitDecision::Refused { .. })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LinkAdmit {
    admitted: Vec<(BurstKey, Instant)>,
    shed: BTreeSet<BurstKey>,
}

/// Per-link admitted sets of one node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Admission {
    links: BTreeMap<LinkId, LinkAdmit>,
    pub t_sticky_us: u64,
}

impl Admission {
    pub fn new(t_sticky_us: u64) -> Self {
        Admission {
            links: BTreeMap::new(),
            t_sticky_us,
        }
    }

    /// Bursts currently forwarded on `link`.
    pub fn admitted(&self, link: LinkId) -> Vec<BurstKey> {
        self.links
            .get(&link)
            .map(|l| l.admitted.iter().map(|(k, _)| *k).collect())
            .unwrap_or_default()
    }

    /// `admits(L, b)`. `cap` is `cap(L)`; `congestion` is `cap_pacing(L) < S_MAX`.
    pub fn admits(
        &mut self,
        now: Instant,
        link: LinkId,
        b: BurstKey,
        table: &mut FloorTable,
        cap: u8,
        congestion: bool,
    ) -> AdmitDecision {
        let Some(entry) = table.get(b).copied() else {
            return AdmitDecision::Refused { reject: None };
        };
        if !entry.live() {
            return AdmitDecision::Refused { reject: None };
        }
        let la = self.links.entry(link).or_default();
        // Drop bursts that left the live table.
        la.admitted
            .retain(|(k, _)| table.get(*k).is_some_and(|e| e.live()));
        la.shed.retain(|k| table.get(*k).is_some());
        if la.shed.contains(&b) {
            return AdmitDecision::Refused { reject: None };
        }
        if la.admitted.iter().any(|(k, _)| *k == b) {
            Self::enforce(la, table, cap);
            return if la.admitted.iter().any(|(k, _)| *k == b) {
                AdmitDecision::Admitted
            } else {
                AdmitDecision::Refused { reject: None }
            };
        }
        Self::enforce(la, table, cap);
        if la.admitted.len() < usize::from(cap) {
            la.admitted.push((b, now));
            return AdmitDecision::Admitted;
        }
        let victim = la
            .admitted
            .iter()
            .filter_map(|(k, at)| table.get(*k).map(|e| (e.rank, *k, *at)))
            .max_by(|x, y| x.0.cmp(&y.0));
        if let Some((vrank, vkey, vat)) = victim {
            let higher_prio = entry.rank.prio > vrank.prio;
            let sticky_window = entry.rank.outranks(&vrank) && now.since(vat) < self.t_sticky_us;
            if higher_prio || sticky_window {
                la.admitted.retain(|(k, _)| *k != vkey);
                la.admitted.push((b, now));
                let reason = if higher_prio {
                    FloorRejectReason::Preempted
                } else {
                    FloorRejectReason::Outranked
                };
                return AdmitDecision::AdmittedEvicting {
                    victim: vkey,
                    reason,
                };
            }
        }
        let reject = match table.get_mut(b) {
            Some(e) if !e.rejected_by_me => {
                e.rejected_by_me = true;
                Some(if congestion {
                    FloorRejectReason::Congestion
                } else {
                    FloorRejectReason::Cap
                })
            }
            _ => None,
        };
        AdmitDecision::Refused { reject }
    }

    /// Sheds the lowest-ranked admitted burst over `cap` (cap decreases).
    fn enforce(la: &mut LinkAdmit, table: &FloorTable, cap: u8) {
        while la.admitted.len() > usize::from(cap) {
            let worst = la
                .admitted
                .iter()
                .enumerate()
                .filter_map(|(i, (k, _))| table.get(*k).map(|e| (i, e.rank)))
                .max_by(|x, y| x.1.cmp(&y.1))
                .map(|(i, _)| i);
            let Some(i) = worst else { break };
            let (k, _) = la.admitted.remove(i);
            la.shed.insert(k);
        }
    }

    /// Stream-level shedding (§5.6): drop the lowest-ranked burst admitted on `link`
    /// entirely. Returns the shed burst (the caller marks the link congested).
    pub fn shed_lowest(&mut self, link: LinkId, table: &FloorTable) -> Option<BurstKey> {
        let la = self.links.get_mut(&link)?;
        let (i, _) = la
            .admitted
            .iter()
            .enumerate()
            .filter_map(|(i, (k, _))| table.get(*k).map(|e| (i, e.rank)))
            .max_by(|x, y| x.1.cmp(&y.1))?;
        let (k, _) = la.admitted.remove(i);
        la.shed.insert(k);
        Some(k)
    }

    /// Sheds a specific burst on `link` (the queue chose it).
    pub fn shed(&mut self, link: LinkId, key: BurstKey) {
        let la = self.links.entry(link).or_default();
        la.admitted.retain(|(k, _)| *k != key);
        la.shed.insert(key);
    }

    /// A burst ended (STOP, reject, idle): forget it on every link.
    pub fn on_burst_ended(&mut self, key: BurstKey) {
        for la in self.links.values_mut() {
            la.admitted.retain(|(k, _)| *k != key);
            la.shed.remove(&key);
        }
    }

    /// The link went down.
    pub fn on_link_down(&mut self, link: LinkId) {
        self.links.remove(&link);
    }
}
