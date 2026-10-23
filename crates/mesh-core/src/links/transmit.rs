//! Transmit scheduler (DESIGN §3.2 "pacing is in the core", §5.6): round-robin over
//! writable links; per link, pending units first (in order), then a new frame built
//! from the strict-priority queues, sealed and framed for the transport, when the
//! link's token bucket allows it.

use super::link::LinkState;
use super::table::{LinkTable, flush_pongs, seal_ping};
use super::types::LinkDownReason;
use crate::effects::{EffectQueue, Transmit};
use crate::{Instant, LinkId};

impl LinkTable {
    /// The next unit to write, or `None` when nothing may leave now.
    pub fn poll_transmit(&mut self, now: Instant, fx: &mut EffectQueue) -> Option<Transmit> {
        let ids: Vec<LinkId> = self.links.keys().copied().collect();
        if ids.is_empty() {
            return None;
        }
        let start = match self.cursor {
            Some(c) => ids.iter().position(|id| *id > c).unwrap_or(0),
            None => 0,
        };
        for i in 0..ids.len() {
            let id = ids[(start + i) % ids.len()];
            if let Some(t) = self.transmit_one(now, id, fx) {
                self.cursor = Some(id);
                return Some(t);
            }
        }
        None
    }

    fn transmit_one(&mut self, now: Instant, id: LinkId, fx: &mut EffectQueue) -> Option<Transmit> {
        let l = self.links.get_mut(&id)?;
        if !l.writable {
            return None;
        }
        if l.units.is_empty() && l.can_seal() && l.has_frame_work() && l.pace.allows(now) {
            // PONG `hold_us` and PING `t_tx` are measured now, as the frame is sealed.
            flush_pongs(l, now);
            seal_ping(l, now);
            let budget = l.plaintext_budget();
            if let Some(pt) = l.queues.next_plaintext(now, budget) {
                let diag = self.diag.on();
                match l.security.seal(now, &pt) {
                    Ok(frame) => {
                        l.pace.charge(now, frame.len());
                        if diag {
                            let ty = if l.is_lab() {
                                mesh_lab::ridelog::FrameType::Lab
                            } else {
                                crate::diag::classify::plaintext_type(&pt)
                            };
                            l.diag.tx(ty, frame.len());
                        }
                        match l.framer.outbound(&frame) {
                            Ok(units) => {
                                l.counters.tx_frames += 1;
                                l.units.extend(units);
                            }
                            Err(_) => l.counters.bad_frames += 1,
                        }
                    }
                    Err(_) => l.counters.bad_frames += 1,
                }
            }
        }
        if let Some(bytes) = l.units.pop_front() {
            l.last_tx = now;
            l.counters.tx_bytes += bytes.len() as u64;
            return Some(Transmit { link: id, bytes });
        }
        if l.state == LinkState::Closing && l.queues.is_empty() {
            let reason = l.closing.map(|(r, _)| r);
            self.close_now(id, reason, LinkDownReason::LocalClose, fx);
        }
        None
    }

    /// The platform accepts writes on `id` again.
    pub fn on_writable(&mut self, id: LinkId) {
        if let Some(l) = self.links.get_mut(&id) {
            l.writable = true;
        }
    }

    /// The platform refused `unit` (WouldBlock): it is re-sent first once writable, so
    /// stream order is preserved.
    pub fn on_would_block(&mut self, id: LinkId, unit: &[u8]) {
        if let Some(l) = self.links.get_mut(&id) {
            if let Some((count, ms)) = l.diag.would_block(self.diag_now) {
                self.diag
                    .push(mesh_lab::ridelog::DiagEvent::WouldBlockBurst {
                        link: id.0,
                        count,
                        ms,
                    });
            }
            l.writable = false;
            l.units.push_front(unit.to_vec());
            l.counters.tx_bytes = l.counters.tx_bytes.saturating_sub(unit.len() as u64);
        }
    }

    /// `true` when some link has something it could send now (tests, runtime hints).
    pub fn has_pending(&self) -> bool {
        self.links
            .values()
            .any(|l| l.writable && (!l.units.is_empty() || l.has_frame_work()))
    }
}
