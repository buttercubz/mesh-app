//! `LinkSelector::evaluate` (see `select.rs` for the policy).

use super::{
    Attempt, Candidate, CandidateClass, DiscoveryMode, LinkSelector, SelectAction, SelectInput,
    reachable_without, two_hop_graph,
};
use crate::neighbor::NeighborTable;
use crate::types::{Instant, LinkId, LinkKind};

impl LinkSelector {
    /// Max links of `kind` now.
    pub fn max_links(&self, now: Instant, kind: LinkKind, aware_data_paths: u8) -> usize {
        match kind {
            LinkKind::Ble => usize::from(self.ble_max(now)),
            LinkKind::Aware => usize::from(self.limits.aware_links_max.min(aware_data_paths)),
            LinkKind::Tunnel => 1,
        }
    }

    /// One evaluation (call when [`LinkSelector::due`]; it reschedules itself 2 s ahead).
    pub fn evaluate(
        &mut self,
        now: Instant,
        input: &SelectInput,
        table: &NeighborTable,
    ) -> Vec<SelectAction> {
        let mut out = Vec::new();
        self.schedule(now);
        self.timeouts(now, &mut out);

        let ble = Self::count(table, LinkKind::Ble);
        let under_target = ble < usize::from(self.limits.ble_links_target.min(self.ble_max(now)));
        let missing = !input.unreachable_members.is_empty();
        let mode = if under_target || missing {
            DiscoveryMode::Forming
        } else {
            DiscoveryMode::Stable
        };
        if self.mode != Some(mode) {
            self.mode = Some(mode);
            out.push(SelectAction::SetDiscovery(mode));
        }

        // Over max (per kind) or too many distinct neighbors: close one redundant link.
        for kind in [LinkKind::Ble, LinkKind::Aware] {
            let max = self.max_links(now, kind, input.aware_data_paths);
            if Self::count(table, kind) > max
                && let Some(link) = self.redundant_link(now, kind, input, table)
            {
                out.push(SelectAction::CloseRedundant { link });
            }
        }
        if table.len() > usize::from(self.limits.neighbors_max)
            && let Some(link) = self.redundant_link(now, LinkKind::Ble, input, table)
            && !out.contains(&SelectAction::CloseRedundant { link })
        {
            out.push(SelectAction::CloseRedundant { link });
        }

        if mode == DiscoveryMode::Forming {
            for kind in [LinkKind::Ble, LinkKind::Aware] {
                if self.outstanding.contains_key(&kind) {
                    continue;
                }
                let room = Self::count(table, kind)
                    < self.max_links(now, kind, input.aware_data_paths)
                    && table.len() < usize::from(self.limits.neighbors_max);
                if !room {
                    continue;
                }
                if let Some(c) = self.pick(now, kind, input, table) {
                    let speculative = c.class == CandidateClass::Speculative;
                    if speculative {
                        self.last_speculative = Some(now);
                    }
                    self.outstanding.insert(
                        kind,
                        Attempt {
                            disc_id: c.disc_id,
                            peer_ref: c.peer_ref,
                            speculative,
                            started: now,
                        },
                    );
                    out.push(SelectAction::Connect {
                        disc_id: c.disc_id,
                        speculative,
                    });
                }
            }
        }
        out
    }

    /// Best candidate of `kind`: unreachable member, unknown, then RSSI; speculative last.
    fn pick(
        &self,
        now: Instant,
        kind: LinkKind,
        input: &SelectInput,
        table: &NeighborTable,
    ) -> Option<Candidate> {
        let under_target =
            Self::count(table, LinkKind::Ble) < usize::from(self.limits.ble_links_target);
        input
            .candidates
            .iter()
            .filter(|c| c.transport == kind)
            .filter(|c| !self.in_backoff(now, c.peer_ref))
            .filter(|c| {
                c.node
                    .is_none_or(|n| n != self.me && table.neighbor(n).is_none())
            })
            .filter(|c| match c.class {
                CandidateClass::Speculative => self.speculative_allowed(now),
                // Extra paths to reachable members only while under target.
                CandidateClass::ReachableMember => under_target,
                CandidateClass::UnreachableMember | CandidateClass::Unknown => true,
            })
            .min_by(|a, b| {
                a.class
                    .cmp(&b.class)
                    .then(b.rssi.cmp(&a.rssi))
                    .then(a.disc_id.cmp(&b.disc_id))
            })
            .copied()
    }

    /// A link of `kind` to a neighbor that appears good in another neighbor's fresh NBR,
    /// worst `q` first, skipping any whose removal would cut off a reachable member.
    fn redundant_link(
        &self,
        now: Instant,
        kind: LinkKind,
        input: &SelectInput,
        table: &NeighborTable,
    ) -> Option<LinkId> {
        let adj = two_hop_graph(now, self.me, table);
        let base = reachable_without(&adj, self.me, None);
        let mut cands: Vec<(u8, LinkId, u16)> = Vec::new();
        for n in table.neighbors() {
            let Some(l) = n.links.get(kind) else { continue };
            let redundant = table.neighbors().any(|o| {
                o.node_id != n.node_id && o.fresh_nbr(now).is_some_and(|r| r.lists_good(n.node_id))
            });
            if !redundant {
                continue;
            }
            // Another kind of link to the same neighbor keeps it reachable.
            let other_kind = n.links.iter().any(|x| x.kind != kind);
            if !other_kind {
                let after = reachable_without(&adj, self.me, Some(n.node_id));
                let cuts = input
                    .reachable_members
                    .iter()
                    .any(|m| base.contains(m) && !after.contains(m));
                if cuts || !after.contains(&n.node_id) {
                    continue;
                }
            }
            cands.push((l.q(now), l.id, n.node_id));
        }
        cands.sort();
        cands.first().map(|c| c.1)
    }
}
