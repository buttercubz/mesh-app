//! Pruning condition 2 of §5.4: a relay never prunes a segment of a message whose
//! reassembly it saw time out from that neighbor (the neighbor may be shedding it).
//!
//! The relay forwards segments without reassembling them; this tracker only records,
//! per `(from, origin, sess, msg_id)`, which segment indices arrived and reports the
//! messages that did not complete within the reassembly timeout (5 s, §4.6).

use std::collections::BTreeMap;

use mesh_routing::{Instant, NodeId};

/// Reassembly timeout of segmented messages (§4.6).
pub const SEG_TIMEOUT_US: u64 = 5_000_000;
/// Assemblies tracked per neighbor (bounded memory; the oldest is dropped).
pub const MAX_TRACKED_PER_NEIGHBOR: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Track {
    first: Instant,
    count: u8,
    /// Bit `i` set when segment `i` arrived.
    seen: u16,
}

/// Segment arrivals per neighbor and message.
#[derive(Debug, Clone, Default)]
pub struct SegTracker {
    tracks: BTreeMap<(NodeId, NodeId, u8, u16), Track>,
}

impl SegTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Segment `idx` of `count` of `(origin, sess, msg_id)` arrived from `from`.
    pub fn observe(
        &mut self,
        now: Instant,
        from: NodeId,
        key: (NodeId, u8, u16),
        idx: u8,
        count: u8,
    ) {
        if idx >= count || count > 16 {
            return;
        }
        let k = (from, key.0, key.1, key.2);
        if !self.tracks.contains_key(&k) {
            let mine: Vec<_> = self
                .tracks
                .keys()
                .filter(|t| t.0 == from)
                .copied()
                .collect();
            if mine.len() >= MAX_TRACKED_PER_NEIGHBOR
                && let Some(oldest) = mine
                    .iter()
                    .min_by_key(|t| self.tracks.get(t).map(|x| x.first))
            {
                self.tracks.remove(oldest);
            }
        }
        let t = self.tracks.entry(k).or_insert(Track {
            first: now,
            count,
            seen: 0,
        });
        t.seen |= 1 << idx;
        let full = if t.count >= 16 {
            u16::MAX
        } else {
            (1u16 << t.count) - 1
        };
        if t.seen & full == full {
            self.tracks.remove(&k);
        }
    }

    /// Removes the assemblies that timed out and returns them as
    /// `(from, origin, sess, msg_id)`.
    pub fn expire(&mut self, now: Instant) -> Vec<(NodeId, NodeId, u8, u16)> {
        let gone: Vec<_> = self
            .tracks
            .iter()
            .filter(|(_, t)| now.since(t.first) >= SEG_TIMEOUT_US)
            .map(|(k, _)| *k)
            .collect();
        for k in &gone {
            self.tracks.remove(k);
        }
        gone
    }

    /// Earliest timeout.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.tracks
            .values()
            .map(|t| t.first.plus_us(SEG_TIMEOUT_US))
            .min()
    }

    /// Forgets a neighbor (its links are gone).
    pub fn forget(&mut self, from: NodeId) {
        self.tracks.retain(|k, _| k.0 != from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_messages_never_time_out() {
        let mut s = SegTracker::new();
        let t = Instant::from_ms(10);
        s.observe(t, 2, (9, 1, 44), 0, 2);
        s.observe(t, 2, (9, 1, 44), 1, 2);
        s.observe(t, 3, (9, 1, 45), 0, 3);
        assert_eq!(s.next_deadline(), Some(t.plus_us(SEG_TIMEOUT_US)));
        assert!(s.expire(t.plus_ms(4_999)).is_empty());
        assert_eq!(s.expire(t.plus_us(SEG_TIMEOUT_US)), vec![(3, 9, 1, 45)]);
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn bounded_per_neighbor() {
        let mut s = SegTracker::new();
        for i in 0..(MAX_TRACKED_PER_NEIGHBOR as u16 + 4) {
            s.observe(Instant::from_ms(u64::from(i)), 2, (9, 1, i), 0, 2);
        }
        assert_eq!(s.tracks.len(), MAX_TRACKED_PER_NEIGHBOR);
        s.forget(2);
        assert!(s.tracks.is_empty());
    }
}
