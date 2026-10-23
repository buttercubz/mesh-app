//! A simulated radio link between two nodes: both engine-side ids, the connection
//! interval and its anchor, and per direction the OS send queue and the transmit
//! timeline of the radio model (DESIGN §13.2).

use std::collections::VecDeque;

use mesh_core::{LinkId, LinkKind, LinkUnit};

/// One end of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct End {
    pub node: usize,
    pub link: LinkId,
}

/// One direction (`ends[d]` → `ends[1 - d]`).
#[derive(Debug, Clone, Default)]
pub struct Dir {
    /// Units accepted by the OS, not yet on air.
    pub queue: VecDeque<Vec<u8>>,
    pub queued_bytes: usize,
    /// End of the last transmission (µs).
    pub busy_until: u64,
    /// The engine got a WouldBlock and waits for Writable.
    pub want_writable: bool,
    /// A pump event is scheduled.
    pub pumping: bool,
    /// Units and bytes delivered, attempts made (for reports).
    pub delivered: u64,
    pub bytes: u64,
    pub attempts: u64,
    /// Units lost on air (datagram links) or discarded at link drop.
    pub lost: u64,
    /// Writes refused with WouldBlock.
    pub would_blocks: u64,
    /// The last units delivered (replay material for fault injection).
    pub recent: VecDeque<Vec<u8>>,
    /// OS-accept instant of every queued unit (parallel to `queue`).
    pub enq: VecDeque<u64>,
    /// OS accept → delivery delay of delivered units: sum, count, max (µs).
    pub delay_sum_us: u64,
    pub delay_n: u64,
    pub delay_max_us: u64,
    /// Time of the last delivered unit (µs; 0 = none yet).
    pub last_delivery_us: u64,
}

/// A link between `ends[0]` (initiator / central) and `ends[1]` (responder).
#[derive(Debug, Clone)]
pub struct SimLink {
    pub ends: [End; 2],
    pub kind: LinkKind,
    pub unit: LinkUnit,
    pub up: bool,
    pub ci_us: u64,
    /// Connection-event anchor in `[0, ci)`.
    pub anchor_us: u64,
    pub dirs: [Dir; 2],
    /// Since when the PDR is below the drop threshold.
    pub below_since: Option<u64>,
    pub created_at: u64,
    pub down_at: Option<u64>,
    /// ATT MTU (GATT links).
    pub att_mtu: Option<u16>,
    /// The end (index into `ends`) that asked the platform to close; the link is torn
    /// down once that direction has flushed its queue.
    pub closing_by: Option<usize>,
}

impl SimLink {
    /// The direction index whose sender is `node`'s `link`.
    pub fn dir_of(&self, node: usize, link: LinkId) -> Option<usize> {
        self.ends
            .iter()
            .position(|e| e.node == node && e.link == link)
    }

    /// The other node.
    pub fn peer(&self, node: usize) -> Option<usize> {
        match (self.ends[0].node == node, self.ends[1].node == node) {
            (true, _) => Some(self.ends[1].node),
            (_, true) => Some(self.ends[0].node),
            _ => None,
        }
    }

    /// `true` when the link joins `a` and `b` (either order).
    pub fn joins(&self, a: usize, b: usize) -> bool {
        (self.ends[0].node == a && self.ends[1].node == b)
            || (self.ends[0].node == b && self.ends[1].node == a)
    }

    /// Next connection event at or after `t` (µs).
    pub fn next_event(&self, t: u64) -> u64 {
        let ci = self.ci_us.max(1);
        if t <= self.anchor_us {
            return self.anchor_us;
        }
        let k = (t - self.anchor_us).div_ceil(ci);
        self.anchor_us + k * ci
    }

    /// `true` when direction `d` cannot take `len` more bytes now.
    pub fn would_block(&self, d: usize, len: usize, stream_cap: usize, gatt_cap: usize) -> bool {
        let dir = &self.dirs[d];
        match self.unit {
            LinkUnit::Stream => !dir.queue.is_empty() && dir.queued_bytes + len > stream_cap,
            LinkUnit::Message => dir.queue.len() >= gatt_cap,
            // Datagrams are never refused; an overfull socket drops them.
            LinkUnit::Datagram => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(unit: LinkUnit) -> SimLink {
        SimLink {
            ends: [
                End {
                    node: 0,
                    link: LinkId(1),
                },
                End {
                    node: 3,
                    link: LinkId(2),
                },
            ],
            kind: LinkKind::BleL2cap,
            unit,
            up: true,
            ci_us: 30_000,
            anchor_us: 7_000,
            dirs: [Dir::default(), Dir::default()],
            below_since: None,
            created_at: 0,
            down_at: None,
            att_mtu: None,
            closing_by: None,
        }
    }

    #[test]
    fn connection_events_and_lookup() {
        let l = link(LinkUnit::Stream);
        assert_eq!(l.next_event(0), 7_000);
        assert_eq!(l.next_event(7_000), 7_000);
        assert_eq!(l.next_event(7_001), 37_000);
        assert_eq!(l.dir_of(3, LinkId(2)), Some(1));
        assert_eq!(l.dir_of(3, LinkId(1)), None);
        assert_eq!(l.peer(0), Some(3));
        assert!(l.joins(3, 0));
    }

    #[test]
    fn os_buffers_refuse_when_full() {
        let mut l = link(LinkUnit::Stream);
        assert!(
            !l.would_block(0, 5_000, 2_048, 4),
            "an empty queue takes one unit"
        );
        l.dirs[0].queue.push_back(vec![0; 1_500]);
        l.dirs[0].queued_bytes = 1_500;
        assert!(l.would_block(0, 600, 2_048, 4));
        assert!(!l.would_block(0, 500, 2_048, 4));
        let mut g = link(LinkUnit::Message);
        for _ in 0..4 {
            g.dirs[1].queue.push_back(vec![0; 20]);
        }
        assert!(g.would_block(1, 20, 2_048, 4));
    }
}
