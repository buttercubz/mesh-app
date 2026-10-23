//! Two-hop pruning (neighbor elimination), **ctrl and bulk classes only** (DESIGN §5.4).
//!
//! Voice is never pruned. For ctrl/bulk, relay X, having received P from N, skips
//! neighbor M only if both hold:
//! 1. N's NBR is fresh (< 1.5 s) and lists M with `good = 1` and `congested = 0`;
//! 2. P is not a segment of a message whose reassembly X saw time out from N (N may be
//!    shedding it).
//!
//! The origin never prunes (it has no N), which anchors the induction.

use std::collections::BTreeMap;

use mesh_wire::mesh::TrafficClass;

use crate::neighbor::Neighbor;
use crate::types::{Instant, NodeId};

/// How long a reassembly timeout keeps disabling pruning for that message.
pub const SEG_TIMEOUT_MEMORY_US: u64 = 30_000_000;

/// `(N, origin, sess, msg_id)` of segmented messages whose reassembly timed out.
#[derive(Debug, Clone, Default)]
pub struct SegTimeouts {
    seen: BTreeMap<(NodeId, NodeId, u8, u16), Instant>,
}

impl SegTimeouts {
    pub fn new() -> Self {
        Self::default()
    }

    /// The reassembly of `(origin, sess, msg_id)`, fed by segments from `from`, timed
    /// out.
    pub fn note(&mut self, now: Instant, from: NodeId, origin: NodeId, sess: u8, msg_id: u16) {
        self.seen.insert((from, origin, sess, msg_id), now);
    }

    pub fn timed_out(
        &self,
        now: Instant,
        from: NodeId,
        origin: NodeId,
        sess: u8,
        msg_id: u16,
    ) -> bool {
        self.seen
            .get(&(from, origin, sess, msg_id))
            .is_some_and(|t| now.since(*t) < SEG_TIMEOUT_MEMORY_US)
    }

    pub fn expire(&mut self, now: Instant) {
        self.seen
            .retain(|_, t| now.since(*t) < SEG_TIMEOUT_MEMORY_US);
    }
}

/// `pruned(N, M, P)`: `true` when relay X may skip neighbor `m` for a packet of `class`
/// received from `n`. `seg_timed_out` is condition 2 for this packet.
pub fn pruned(
    now: Instant,
    class: TrafficClass,
    n: &Neighbor,
    m: NodeId,
    seg_timed_out: bool,
) -> bool {
    if !matches!(class, TrafficClass::Ctrl | TrafficClass::Bulk) {
        return false;
    }
    if seg_timed_out {
        return false;
    }
    n.fresh_nbr(now)
        .is_some_and(|r| r.lists_good_uncongested(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neighbor::NeighborTable;
    use crate::types::{LinkId, LinkKind, LinkRole};
    use mesh_wire::record::{Nbr, NbrEntry, NbrFlags};

    fn table_with_nbr(entries: &[(u16, bool, bool)], at: Instant) -> NeighborTable {
        let mut t = NeighborTable::new();
        t.link_up(at, 2, LinkId(1), LinkKind::Ble, LinkRole::Initiator, 1024)
            .expect("up");
        let list: Vec<NbrEntry> = entries
            .iter()
            .map(|&(n, good, cong)| NbrEntry {
                node_id: n,
                q: if good { 90 } else { 30 },
                flags: NbrFlags::new(good, cong, 0),
                vcap: 2,
            })
            .collect();
        t.on_nbr(at, LinkId(1), &Nbr::new(0, &list).expect("nbr"));
        t
    }

    #[test]
    fn conditions() {
        let at = Instant::from_secs(1);
        let t = table_with_nbr(&[(5, true, false), (6, true, true), (7, false, false)], at);
        let n = t.neighbor(2).expect("n");
        assert!(pruned(at, TrafficClass::Ctrl, n, 5, false));
        assert!(pruned(at, TrafficClass::Bulk, n, 5, false));
        // Voice and lab are never pruned.
        assert!(!pruned(at, TrafficClass::Voice, n, 5, false));
        assert!(!pruned(at, TrafficClass::Lab, n, 5, false));
        // Congested, not good, not listed, stale, or a timed-out segment: no pruning.
        assert!(!pruned(at, TrafficClass::Ctrl, n, 6, false));
        assert!(!pruned(at, TrafficClass::Ctrl, n, 7, false));
        assert!(!pruned(at, TrafficClass::Ctrl, n, 8, false));
        assert!(!pruned(at.plus_ms(1_500), TrafficClass::Ctrl, n, 5, false));
        assert!(!pruned(at, TrafficClass::Ctrl, n, 5, true));
    }

    #[test]
    fn seg_timeouts() {
        let mut s = SegTimeouts::new();
        let t = Instant::from_secs(1);
        s.note(t, 2, 9, 1, 44);
        assert!(s.timed_out(t, 2, 9, 1, 44));
        assert!(!s.timed_out(t, 3, 9, 1, 44));
        assert!(!s.timed_out(t.plus_us(SEG_TIMEOUT_MEMORY_US), 2, 9, 1, 44));
        s.expire(t.plus_us(SEG_TIMEOUT_MEMORY_US));
        assert!(!s.timed_out(t, 2, 9, 1, 44));
    }
}
