//! The 2-hop graph used by the articulation check (DESIGN §5.7): me, my neighbors and
//! the nodes listed in their fresh NBR reports.

use std::collections::{BTreeMap, BTreeSet};

use crate::neighbor::NeighborTable;
use crate::types::{Instant, NodeId};

/// Undirected adjacency of the 2-hop graph seen from `me`.
pub fn two_hop_graph(
    now: Instant,
    me: NodeId,
    table: &NeighborTable,
) -> BTreeMap<NodeId, BTreeSet<NodeId>> {
    let mut adj: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    let mut edge = |a: NodeId, b: NodeId| {
        if a != b {
            adj.entry(a).or_default().insert(b);
            adj.entry(b).or_default().insert(a);
        }
    };
    for n in table.neighbors() {
        edge(me, n.node_id);
        if let Some(r) = n.fresh_nbr(now) {
            for m in r.nodes() {
                edge(n.node_id, m);
            }
        }
    }
    adj
}

/// Nodes reachable from `me` when the edge `me – without` is removed.
pub fn reachable_without(
    adj: &BTreeMap<NodeId, BTreeSet<NodeId>>,
    me: NodeId,
    without: Option<NodeId>,
) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::from([me]);
    let mut stack = vec![me];
    while let Some(u) = stack.pop() {
        for &v in adj.get(&u).into_iter().flatten() {
            let cut = without.is_some_and(|w| (u == me && v == w) || (u == w && v == me));
            if !cut && seen.insert(v) {
                stack.push(v);
            }
        }
    }
    seen
}
