//! One link: state machine `Handshake → IdProof → Up → Closing` (the `Hello` step
//! happens before the link exists, in the radio policy), framing, security, queues,
//! PING/PONG RTT, LSTAT counters, pacing and liveness (DESIGN §3.3, §4.4, §5.1, §5.6).

use std::collections::VecDeque;
use std::fmt;

use mesh_routing::quality::RttEstimator;
use mesh_wire::frame::RejectReason;
use mesh_wire::record::{Caps, Lstat};

use super::framing::Framer;
use super::queue::LinkQueues;
use super::security::LinkSecurity;
use super::types::LinkInfo;
use crate::{Instant, LinkId, NodeId};

/// Per-link state (DESIGN §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkState {
    /// Noise handshake running (group links).
    Handshake,
    /// Keys established, waiting for the peer's IDPROOF (group links).
    IdProof,
    /// Records flow; modules see the link.
    Up,
    /// CLOSE/REJECT queued; the platform close follows once flushed.
    Closing,
}

/// A PING waiting for its PONG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutstandingPing {
    pub id: u16,
    pub sent_at: Instant,
}

/// A PONG to send; `hold_us` is computed when the frame is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingPong {
    pub id: u16,
    pub t_tx: u32,
    pub rx_at: Instant,
}

/// Token bucket pacing (§5.6). `None` rate means unpaced (Lab links measure raw
/// capacity; WP-10D sets the group-link rate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pace {
    /// Rate in bit/s.
    pub rate_bps: Option<u32>,
    /// Credit in bit·µs/s units (bits × 10⁶), so a refill never loses a fraction of a
    /// bit however often the bucket is looked at (negative = debt after a large frame).
    credit: i128,
    last: Instant,
}

/// Burst allowance of the token bucket: 50 ms worth of bits, at least one 1 KB frame.
const PACE_BURST_US: u64 = 50_000;
/// One bit in credit units.
const BIT: i128 = 1_000_000;

impl Pace {
    fn burst(rate: u32) -> i128 {
        i128::from((u64::from(rate) * PACE_BURST_US / 1_000_000).max(8 * 1026)) * BIT
    }

    fn refill(&mut self, now: Instant) {
        if let Some(rate) = self.rate_bps {
            let add = i128::from(now.since(self.last)) * i128::from(rate);
            self.credit = (self.credit + add).min(Self::burst(rate));
        }
        self.last = now;
    }

    /// `true` when a frame may leave now.
    pub fn allows(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.rate_bps.is_none() || self.credit >= 0
    }

    /// Charges `bytes` sent now.
    pub fn charge(&mut self, now: Instant, bytes: usize) {
        self.refill(now);
        if self.rate_bps.is_some() {
            self.credit -= (bytes as i128) * 8 * BIT;
        }
    }

    /// When the bucket is non-negative again (`None` if it already is or unpaced).
    pub fn ready_at(&self) -> Option<Instant> {
        let rate = self.rate_bps?;
        if self.credit >= 0 || rate == 0 {
            return None;
        }
        let us = (-self.credit) / i128::from(rate) + 1;
        Some(self.last.plus_us(u64::try_from(us).unwrap_or(u64::MAX)))
    }

    /// Sets the rate (keeps the current credit).
    pub fn set_rate(&mut self, now: Instant, rate_bps: Option<u32>) {
        self.refill(now);
        self.rate_bps = rate_bps;
        if let Some(r) = rate_bps {
            self.credit = self.credit.min(Self::burst(r));
        }
    }
}

/// Counters for LSTAT and the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkCounters {
    pub rx_frames: u64,
    pub rx_bytes: u64,
    pub tx_frames: u64,
    pub tx_bytes: u64,
    /// Frames that failed to decode or authenticate.
    pub bad_frames: u64,
    pub pings_sent: u64,
    pub pongs_received: u64,
    /// LSTAT window (since the last LSTAT sent).
    pub win_rx_frames: u32,
    pub win_rx_bytes: u32,
    pub win_gaps: u32,
}

/// One link.
pub struct Link {
    pub id: LinkId,
    pub info: LinkInfo,
    pub state: LinkState,
    pub(crate) security: Box<dyn LinkSecurity>,
    pub(crate) framer: Framer,
    pub(crate) queues: LinkQueues,
    /// Framed units ready to send, in order (GATT chunks, WouldBlock retries, raw HS).
    pub(crate) units: VecDeque<Vec<u8>>,
    pub writable: bool,
    pub created_at: Instant,
    pub last_rx: Instant,
    pub last_tx: Instant,
    /// Node id of the peer (IDPROOF for group links, LAB_CTL announce in Lab).
    pub peer_node: Option<NodeId>,
    pub peer_caps: Option<Caps>,
    pub peer_lstat: Option<Lstat>,
    pub rtt: RttEstimator,
    pub rssi: Option<i16>,
    pub att_mtu: Option<u16>,
    pub counters: LinkCounters,
    /// Diagnostics counters (filled only while diagnostics are on).
    pub diag: crate::diag::link::LinkDiag,
    pub pace: Pace,
    pub(crate) next_ping_id: u16,
    pub(crate) outstanding: Vec<OutstandingPing>,
    pub(crate) pongs: Vec<PendingPong>,
    pub(crate) next_ping_at: Instant,
    /// A PING is due: it is built (`t_tx` stamped) when the next frame of this link is
    /// sealed, so the RTT excludes the local token-bucket wait (§5.6 delay signal).
    pub(crate) ping_due: bool,
    pub(crate) next_lstat_at: Instant,
    pub(crate) caps_sent: bool,
    /// Security established at some point (frames can be sealed, also while closing).
    pub(crate) keys: bool,
    /// Closing: the reason, and the instant after which the platform close is forced.
    pub(crate) closing: Option<(RejectReason, Instant)>,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("id", &self.id)
            .field("info", &self.info)
            .field("state", &self.state)
            .field("peer_node", &self.peer_node)
            .field("writable", &self.writable)
            .finish_non_exhaustive()
    }
}

impl Link {
    pub(crate) fn new(
        id: LinkId,
        info: LinkInfo,
        security: Box<dyn LinkSecurity>,
        now: Instant,
        first_ping_id: u16,
    ) -> Self {
        Link {
            id,
            framer: Framer::new(info.unit, info.att_mtu),
            att_mtu: info.att_mtu,
            info,
            state: LinkState::Handshake,
            security,
            queues: LinkQueues::new(),
            units: VecDeque::new(),
            writable: true,
            created_at: now,
            last_rx: now,
            last_tx: now,
            peer_node: None,
            peer_caps: None,
            peer_lstat: None,
            rtt: RttEstimator::default(),
            rssi: None,
            counters: LinkCounters::default(),
            diag: crate::diag::link::LinkDiag::default(),
            pace: Pace {
                rate_bps: None,
                credit: 0,
                last: now,
            },
            next_ping_id: first_ping_id,
            outstanding: Vec::new(),
            pongs: Vec::new(),
            next_ping_at: now,
            ping_due: false,
            next_lstat_at: now,
            caps_sent: false,
            keys: false,
            closing: None,
        }
    }

    /// `true` once records may flow (Up, or IdProof for the IDPROOF record itself).
    pub fn established(&self) -> bool {
        matches!(self.state, LinkState::IdProof | LinkState::Up)
    }

    /// `true` when outgoing plaintexts can be sealed (established, or closing after
    /// having been established so the CLOSE record still leaves).
    pub fn can_seal(&self) -> bool {
        self.keys && self.state != LinkState::Handshake
    }

    /// `true` for Lab links.
    pub fn is_lab(&self) -> bool {
        self.security.is_lab()
    }

    /// Largest record plaintext one frame of this link carries.
    pub fn plaintext_budget(&self) -> usize {
        self.framer
            .frame_limit(self.info.kind, self.info.max_frame)
            .saturating_sub(self.security.overhead())
    }

    /// Smoothed RTT in µs.
    pub fn srtt_us(&self) -> Option<u64> {
        self.rtt.srtt_us()
    }

    /// One-way delay estimate (RTT/2), used for the mesh `age` rewrite (§4.5).
    pub fn owd_us(&self) -> u64 {
        self.rtt.owd_us()
    }

    /// Items queued at `prio`.
    pub fn queues_len(&self, prio: super::queue::Prio) -> usize {
        self.queues.len(prio)
    }

    /// Bytes waiting to leave this link.
    pub fn backlog_bytes(&self) -> usize {
        self.queues.bytes() + self.units.iter().map(Vec::len).sum::<usize>()
    }

    /// Queue delay estimate at the current pace (ms, saturating at 255) for LSTAT.
    pub fn queue_ms(&self) -> u8 {
        let rate = self.pace.rate_bps.unwrap_or(60_000).max(1);
        let ms = self.backlog_bytes() as u64 * 8 * 1_000 / u64::from(rate);
        ms.min(255) as u8
    }

    /// Something waits for the next sealed frame: queued records, PONGs whose `hold_us`
    /// is measured at sealing, or a due PING stamped at sealing.
    pub(crate) fn has_frame_work(&self) -> bool {
        !self.queues.is_empty() || !self.pongs.is_empty() || self.ping_due
    }

    /// Records the RTT of a PONG (`t_tx` echo, `hold_us` at the responder).
    pub(crate) fn on_pong(&mut self, now: Instant, id: u16, t_tx: u32, hold_us: u16) {
        let Some(pos) = self.outstanding.iter().position(|p| p.id == id) else {
            return;
        };
        self.outstanding.remove(pos);
        self.counters.pongs_received += 1;
        // A saturated hold (≥ 65.5 ms at the responder, e.g. its token bucket in debt
        // at the 8 kbps idle floor) hides part of the responder's wait: no sample, so
        // that local pacing is never read as path delay (§5.6).
        if hold_us == u16::MAX {
            return;
        }
        let rtt = (now.0 as u32).wrapping_sub(t_tx);
        let rtt = u64::from(rtt).saturating_sub(u64::from(hold_us));
        if rtt < 30_000_000 {
            self.rtt.sample(now.routing(), rtt);
        }
    }

    /// Next deadline of this link's own timers (ping, LSTAT, death, handshake, close,
    /// pacing).
    pub(crate) fn next_deadline(&self, death_us: u64, hs_timeout_us: u64) -> Instant {
        let mut t = self.last_rx.plus_us(death_us);
        match self.state {
            LinkState::Handshake | LinkState::IdProof => {
                t = t.min(self.created_at.plus_us(hs_timeout_us));
            }
            LinkState::Up => {
                t = t.min(self.next_ping_at).min(self.next_lstat_at);
            }
            LinkState::Closing => {}
        }
        if let Some((_, at)) = self.closing {
            t = t.min(at);
        }
        // The pacing deadline matters only while the platform accepts writes: a link
        // that answered WouldBlock is woken by `Writable`, and its bucket is refilled
        // only when it transmits again, so a stale `ready_at` would spin the engine.
        if let Some(p) = self.pace.ready_at()
            && self.writable
            && (self.has_frame_work() || !self.units.is_empty())
        {
            t = t.min(p);
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_bucket() {
        let mut p = Pace::default();
        assert!(p.allows(Instant(0)));
        p.set_rate(Instant(0), Some(80_000));
        assert!(p.allows(Instant(0)));
        p.charge(Instant(0), 1000);
        assert!(!p.allows(Instant(0)));
        let at = p.ready_at().unwrap();
        assert_eq!(at, Instant(100_001));
        assert!(p.allows(at));
        p.set_rate(at, None);
        p.charge(at, 100_000);
        assert!(p.allows(at));
    }

    #[test]
    fn frequent_looks_never_starve_the_bucket() {
        // 60 kbps is 0.06 bit/µs: a bucket looked at every 5 µs must still refill.
        let mut p = Pace::default();
        p.set_rate(Instant(0), Some(60_000));
        p.charge(Instant(0), 10);
        let ready = p.ready_at().expect("in debt");
        let mut t = 0;
        while !p.allows(Instant(t)) {
            t += 5;
            assert!(t < 10_000, "the bucket never refilled");
        }
        assert!(t <= ready.0 + 5, "ready at {t} µs, announced {ready:?}");
        assert_eq!(ready, Instant(1_334), "80 bits at 60 kbps");
    }
}
