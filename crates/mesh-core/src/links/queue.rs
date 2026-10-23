//! Per-link outgoing queues with strict priority (DESIGN §5.6): P0 link control and
//! TALK/FLOOR_REJECT, P1 voice, P2 presence/roster/LSTAT, P3 lab/bulk. Records are
//! coalesced into one plaintext up to the frame budget at send time (the sender never
//! waits to coalesce). Whole-frame items (Lab probes, which must keep their exact
//! size) are sealed alone.

use std::collections::VecDeque;

use mesh_wire::record::{MAX_RECORDS_PER_FRAME, RecordWriter, record_len};

use crate::Instant;

/// Queue priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Prio {
    /// Link control (HS, PONG, IDPROOF, CLOSE, NBR on change), TALK, FLOOR_REJECT.
    P0 = 0,
    /// Voice.
    P1 = 1,
    /// Presence, position, roster, LSTAT, SYNC.
    P2 = 2,
    /// Lab traffic and bulk.
    P3 = 3,
}

impl Prio {
    pub const ALL: [Prio; 4] = [Prio::P0, Prio::P1, Prio::P2, Prio::P3];
}

/// One queued item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutItem {
    /// One record, coalescable with others.
    Record {
        rtype: u8,
        body: Vec<u8>,
        /// Dropped (never sent) after this instant (voice deadlines).
        deadline: Option<Instant>,
    },
    /// A complete record plaintext sealed into its own frame.
    Frame { plaintext: Vec<u8> },
}

impl OutItem {
    fn expired(&self, now: Instant) -> bool {
        matches!(self, OutItem::Record { deadline: Some(d), .. } if now > *d)
    }
}

/// Maximum items per priority queue; the oldest item is dropped beyond it.
pub const QUEUE_LIMIT: [usize; 4] = [256, 64, 256, 512];

/// The four priority queues of one link.
#[derive(Debug, Clone, Default)]
pub struct LinkQueues {
    q: [VecDeque<OutItem>; 4],
    dropped: u32,
    expired: u32,
}

impl LinkQueues {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues an item; returns `false` if an older item had to be dropped.
    pub fn push(&mut self, prio: Prio, item: OutItem) -> bool {
        let q = &mut self.q[prio as usize];
        let mut ok = true;
        if q.len() >= QUEUE_LIMIT[prio as usize] {
            q.pop_front();
            self.dropped = self.dropped.saturating_add(1);
            ok = false;
        }
        q.push_back(item);
        ok
    }

    /// Queues an item ahead of everything at `prio` (a PING stamped as its frame is
    /// sealed, so it rides in that very frame).
    pub fn push_front(&mut self, prio: Prio, item: OutItem) {
        let q = &mut self.q[prio as usize];
        if q.len() >= QUEUE_LIMIT[prio as usize] {
            q.pop_back();
            self.dropped = self.dropped.saturating_add(1);
        }
        q.push_front(item);
    }

    /// `true` when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.q.iter().all(VecDeque::is_empty)
    }

    /// Items queued at `prio`.
    pub fn len(&self, prio: Prio) -> usize {
        self.q[prio as usize].len()
    }

    /// Total queued bytes (record bodies and frame plaintexts).
    pub fn bytes(&self) -> usize {
        self.q
            .iter()
            .flatten()
            .map(|i| match i {
                OutItem::Record { body, .. } => record_len(body.len()),
                OutItem::Frame { plaintext } => plaintext.len(),
            })
            .sum()
    }

    /// Items dropped for overflow / for deadline expiry.
    pub fn drops(&self) -> (u32, u32) {
        (self.dropped, self.expired)
    }

    /// Clears every queue (link down).
    pub fn clear(&mut self) {
        for q in &mut self.q {
            q.clear();
        }
    }

    /// Drops every queue of a lower priority than `keep` (graceful close).
    pub fn retain_up_to(&mut self, keep: Prio) {
        for q in &mut self.q[keep as usize + 1..] {
            q.clear();
        }
    }

    fn drop_expired(&mut self, now: Instant) {
        for q in &mut self.q {
            let before = q.len();
            q.retain(|i| !i.expired(now));
            self.expired = self
                .expired
                .saturating_add((before - q.len()).try_into().unwrap_or(u32::MAX));
        }
    }

    /// Builds the next plaintext of at most `budget` bytes: either one whole-frame item,
    /// or the coalesced records in strict priority order. `None` when empty.
    pub fn next_plaintext(&mut self, now: Instant, budget: usize) -> Option<Vec<u8>> {
        self.drop_expired(now);
        let first = self.q.iter().position(|q| !q.is_empty())?;
        if let Some(OutItem::Frame { .. }) = self.q[first].front() {
            if let Some(OutItem::Frame { plaintext }) = self.q[first].pop_front() {
                return Some(plaintext);
            }
            return None;
        }
        let mut w = RecordWriter::with_capacity_limit(budget);
        'prio: for q in self.q.iter_mut().skip(first) {
            while let Some(OutItem::Record { body, .. }) = q.front() {
                if w.count() >= MAX_RECORDS_PER_FRAME || !w.fits(body.len()) {
                    break 'prio;
                }
                if let Some(OutItem::Record { rtype, body, .. }) = q.pop_front() {
                    // `fits` was checked just above, so the push cannot fail.
                    let _ = w.push_raw(rtype, &body);
                }
            }
            if matches!(q.front(), Some(OutItem::Frame { .. })) {
                break;
            }
        }
        if w.count() == 0 {
            // A single record larger than the budget can never be sent: drop it.
            if let Some(q) = self.q.iter_mut().find(|q| !q.is_empty()) {
                q.pop_front();
                self.dropped = self.dropped.saturating_add(1);
            }
            return None;
        }
        Some(w.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_wire::record::{Record, RecordIter, rtype};

    fn rec(rt: u8, n: usize) -> OutItem {
        OutItem::Record {
            rtype: rt,
            body: vec![0; n],
            deadline: None,
        }
    }

    #[test]
    fn strict_priority_and_coalescing() {
        let mut q = LinkQueues::new();
        q.push(Prio::P2, rec(rtype::LSTAT, 11));
        q.push(Prio::P0, rec(rtype::PONG, 8));
        q.push(Prio::P0, rec(rtype::PING, 6));
        let p = q.next_plaintext(Instant(0), 1003).unwrap();
        let types: Vec<u8> = RecordIter::new(&p)
            .map(|r| r.unwrap().rtype())
            .collect::<Vec<_>>();
        assert_eq!(types, vec![rtype::PONG, rtype::PING, rtype::LSTAT]);
        assert!(q.is_empty());
        assert!(matches!(
            RecordIter::new(&p).next(),
            Some(Ok(Record::Pong(_)))
        ));
    }

    #[test]
    fn frames_travel_alone_and_deadlines_expire() {
        let mut q = LinkQueues::new();
        q.push(
            Prio::P3,
            OutItem::Frame {
                plaintext: vec![1, 2, 3],
            },
        );
        q.push(Prio::P0, rec(rtype::PING, 6));
        q.push(
            Prio::P1,
            OutItem::Record {
                rtype: rtype::MESH,
                body: vec![0; 20],
                deadline: Some(Instant(10)),
            },
        );
        let a = q.next_plaintext(Instant(20), 1003).unwrap();
        assert_eq!(RecordIter::new(&a).count(), 1, "the voice record expired");
        assert_eq!(q.drops(), (0, 1));
        assert_eq!(q.next_plaintext(Instant(20), 1003), Some(vec![1, 2, 3]));
        assert_eq!(q.next_plaintext(Instant(20), 1003), None);
        q.push(Prio::P2, rec(rtype::SYNC, 2000));
        assert_eq!(q.next_plaintext(Instant(20), 1003), None);
        assert!(q.is_empty(), "an oversized record is dropped");
    }

    #[test]
    fn overflow_drops_oldest() {
        let mut q = LinkQueues::new();
        for _ in 0..QUEUE_LIMIT[1] {
            assert!(q.push(Prio::P1, rec(rtype::MESH, 1)));
        }
        assert!(!q.push(Prio::P1, rec(rtype::MESH, 1)));
        assert_eq!(q.len(Prio::P1), QUEUE_LIMIT[1]);
        assert_eq!(q.bytes(), QUEUE_LIMIT[1] * 3);
    }
}
