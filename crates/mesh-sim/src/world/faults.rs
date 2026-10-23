//! Adversarial and fault injection on the air (S9, DESIGN §13.2, §8.1): replaying units a
//! node already received, corrupting units in flight, capturing every unit of a link,
//! and opening a platform link between two phones regardless of their discovery (a
//! stranger that connected anyway).

use mesh_core::LinkKind;

use super::{Ev, World};

/// What fault injection observed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FaultStats {
    /// Units re-delivered by `replay_recent`.
    pub replayed: u64,
    /// Records the receiving engines accepted out of replayed units (should stay 0).
    pub replay_records: u64,
    /// Units corrupted in flight.
    pub corrupted: u64,
}

impl World {
    /// Re-delivers, `delay_us` from now, the last `n` units that end `to` of link `li`
    /// received (a replay by an on-air attacker).
    pub fn replay_recent(&mut self, li: usize, to: usize, n: usize, delay_us: u64) {
        let units: Vec<Vec<u8>> = {
            let r = &self.links[li].dirs[1 - to].recent;
            r.iter().rev().take(n).rev().cloned().collect()
        };
        let t = self.now.0 + delay_us;
        for bytes in units {
            self.schedule(
                t,
                Ev::Replay {
                    link: li,
                    to,
                    bytes,
                },
            );
        }
    }

    /// Delivers `bytes` to end `to` of link `li` at `t_us` as a replay (e.g. a recorded
    /// handshake message on a new link).
    pub fn schedule_replay(&mut self, li: usize, to: usize, bytes: Vec<u8>, t_us: u64) {
        self.schedule(
            t_us,
            Ev::Replay {
                link: li,
                to,
                bytes,
            },
        );
    }

    /// Corrupts the next `n` units sent by end `from` of link `li` (one byte flipped
    /// past the stream length prefix).
    pub fn corrupt_next(&mut self, li: usize, from: usize, n: u32) {
        *self.corrupt.entry((li, from)).or_default() += n;
    }

    /// Records every unit put on link `li` from now on (`captured`).
    pub fn capture_link(&mut self, li: usize) {
        self.capture.insert(li);
    }

    /// Units captured on link `li`: `(direction, time µs, bytes)`; direction 0 is sent by
    /// the initiator.
    pub fn captured(&self, li: usize) -> Vec<(usize, u64, Vec<u8>)> {
        self.captured
            .iter()
            .filter(|c| c.0 == li)
            .map(|c| (c.1, c.2, c.3.clone()))
            .collect()
    }

    /// Opens a platform link `a` (initiator) → `b` now, as if `a` had connected without
    /// going through `b`'s discovery (a stranger, a replayed connect).
    pub fn force_link(&mut self, a: usize, b: usize, kind: LinkKind) -> usize {
        let li = self.create_link(a, b, kind, None);
        self.settle();
        li
    }

    /// Delivers a replayed unit and counts what the engine accepted from it.
    pub(crate) fn replay_deliver(&mut self, li: usize, to: usize, bytes: &[u8]) {
        let l = &self.links[li];
        if !l.up {
            return;
        }
        let end = l.ends[to];
        let before = self.nodes[end.node].tap.borrow().records;
        let now = self.now;
        self.faults.replayed += 1;
        self.nodes[end.node]
            .engine
            .handle_packet(now, end.link, bytes);
        let after = self.nodes[end.node].tap.borrow().records;
        self.faults.replay_records += after - before;
        self.dirty.insert(end.node);
    }

    /// Applies a pending corruption to a unit leaving end `d` of link `li`.
    pub(crate) fn maybe_corrupt(&mut self, li: usize, d: usize, bytes: &mut [u8]) {
        let Some(n) = self.corrupt.get_mut(&(li, d)) else {
            return;
        };
        if *n == 0 || bytes.len() < 8 {
            return;
        }
        *n -= 1;
        let k = 4 + (bytes.len() - 4) / 2;
        bytes[k] ^= 0x5A;
        self.faults.corrupted += 1;
    }
}
