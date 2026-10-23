//! The link registry: lifecycle, inbound frames, link-scope records (PING/PONG,
//! LSTAT, CAPS, CLOSE, PAD), per-link timers and queueing (DESIGN §3.3, §4.4, §5.1).
//! The transmit scheduler lives in `transmit.rs`.

use std::collections::BTreeMap;

use mesh_lab::ridelog::{DiagEvent, FrameType};

use mesh_wire::frame::{LinkFrame, RejectReason};
use mesh_wire::record::{Caps, Close, Lstat, Ping, Pong, Record, rtype};

use super::framing::Inbound;
use super::link::{Link, LinkState, OutstandingPing, PendingPong};
use super::queue::{OutItem, Prio};
use super::security::{LinkSecurity, SecurityError, SecurityStep};
use super::types::{LinkDownReason, LinkInfo, LinkKind};
use crate::config::Limits;
use crate::effects::{EffectQueue, RadioCommand};
use crate::{Instant, LinkId, NodeId};

/// Something the engine must tell the modules about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEvent {
    /// Keys are established; the peer's IDPROOF is awaited (group links).
    Established(LinkId),
    /// The link is Up (Lab links right away, group links after `confirm_up`).
    Up(LinkId),
    /// The link is gone.
    Down {
        link: LinkId,
        kind: LinkKind,
        reason: LinkDownReason,
        peer: Option<NodeId>,
        was_up: bool,
        lab: bool,
    },
}

/// At most this many PINGs wait for a PONG; older ones count as lost.
const MAX_OUTSTANDING_PINGS: usize = 8;
/// A Closing link is closed by the platform after this long even if not flushed.
pub const CLOSE_FLUSH_US: u64 = 500_000;
/// Bound of a graceful close that keeps bulk control (P2) queued: a removed member's
/// denying REKEY + ROSTER (≈ 3 KB at 32 entries) at the 8 kbps idle-link grant (§5.6)
/// need about 3 s, still under the 4 s link death.
pub const CLOSE_DRAIN_US: u64 = 3_000_000;

/// All links of the engine, ordered by id (deterministic iteration).
#[derive(Debug, Default)]
pub struct LinkTable {
    pub(crate) links: BTreeMap<LinkId, Link>,
    pub(crate) cursor: Option<LinkId>,
    events: Vec<LinkEvent>,
    /// Diagnostics events of the link layer (moved into `Effect::Diag` by the engine).
    pub(crate) diag: crate::diag::DiagBuf,
    /// Time of the engine call being handled (link-down durations).
    pub(crate) diag_now: Instant,
    own_caps: Option<Caps>,
    fast_ping: bool,
    next_ping_seed: u16,
}

impl LinkTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// The CAPS record sent on every link once it is Up.
    pub fn set_own_caps(&mut self, caps: Caps) {
        self.own_caps = Some(caps);
    }

    pub fn own_caps(&self) -> Option<Caps> {
        self.own_caps
    }

    /// PING every `ping_active` (burst active or Lab) instead of `ping_idle` (§5.1) on
    /// BLE and Tunnel links. Wi-Fi Aware links always PING every `ping_active` (the
    /// §9.6 keepalive behind the 600 ms RX-silence downgrade trigger of §9.7).
    pub fn set_fast_ping(&mut self, fast: bool) {
        self.fast_ping = fast;
    }

    /// The PING period of a link of `kind` now.
    pub fn ping_period(&self, kind: LinkKind, limits: &Limits) -> u64 {
        if self.fast_ping || kind == LinkKind::WifiAware {
            limits.ping_active_us
        } else {
            limits.ping_idle_us
        }
    }

    /// Seeds the first PING id of the next links (from the engine RNG).
    pub fn seed_ping_ids(&mut self, seed: u16) {
        self.next_ping_seed = seed;
    }

    /// Takes the pending events in order.
    pub fn drain_events(&mut self) -> Vec<LinkEvent> {
        core::mem::take(&mut self.events)
    }

    pub fn get(&self, id: LinkId) -> Option<&Link> {
        self.links.get(&id)
    }

    pub fn get_mut(&mut self, id: LinkId) -> Option<&mut Link> {
        self.links.get_mut(&id)
    }

    /// Every link in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Link> {
        self.links.values()
    }

    /// Ids of the links that are Up.
    pub fn up_ids(&self) -> Vec<LinkId> {
        self.links
            .values()
            .filter(|l| l.state == LinkState::Up)
            .map(|l| l.id)
            .collect()
    }

    /// Links that are not closing (counted against the link max).
    pub fn active_count(&self, ble_only: bool) -> usize {
        self.links
            .values()
            .filter(|l| l.state != LinkState::Closing && (!ble_only || l.info.kind.is_ble()))
            .count()
    }

    /// The platform reported a new link; `security` is Plain for Lab links or the
    /// secure module's implementation for group links.
    pub fn add(
        &mut self,
        now: Instant,
        id: LinkId,
        info: LinkInfo,
        security: Box<dyn LinkSecurity>,
        fx: &mut EffectQueue,
    ) {
        if self.links.contains_key(&id) {
            // A duplicate Up for a live id: the platform reused it; drop the old state.
            self.remove(id, LinkDownReason::LocalClose);
        }
        let seed = self.next_ping_seed;
        self.next_ping_seed = seed.wrapping_add(0x3D1B);
        let mut link = Link::new(id, info, security, now, seed);
        let mut out = Vec::new();
        let step = link.security.on_start(now, &mut out);
        for f in out {
            queue_raw(&mut link, &f);
        }
        self.links.insert(id, link);
        self.on_step(now, id, step, fx);
    }

    fn on_step(&mut self, now: Instant, id: LinkId, step: SecurityStep, fx: &mut EffectQueue) {
        let Some(l) = self.links.get_mut(&id) else {
            return;
        };
        match step {
            SecurityStep::Pending => {}
            SecurityStep::Established if l.state == LinkState::Handshake => {
                l.keys = true;
                let ms = now.since(l.created_at) / 1_000;
                let peer = l.peer_node.map(|n| n.0);
                self.diag.push(DiagEvent::Handshake {
                    link: id.0,
                    peer,
                    ok: true,
                    ms: u32::try_from(ms).unwrap_or(u32::MAX),
                    err: None,
                });
                let Some(l) = self.links.get_mut(&id) else {
                    return;
                };
                if l.is_lab() {
                    self.set_up(now, id);
                } else {
                    l.state = LinkState::IdProof;
                    self.events.push(LinkEvent::Established(id));
                }
            }
            SecurityStep::Established => {}
            SecurityStep::Failed(r) => {
                let ms = now.since(l.created_at) / 1_000;
                self.diag.push(DiagEvent::Handshake {
                    link: id.0,
                    peer: l.peer_node.map(|n| n.0),
                    ok: false,
                    ms: u32::try_from(ms).unwrap_or(u32::MAX),
                    err: Some(crate::diag::reject_str(r)),
                });
                self.reject(now, id, r, fx);
            }
        }
    }

    fn set_up(&mut self, now: Instant, id: LinkId) {
        let caps = self.own_caps;
        let Some(l) = self.links.get_mut(&id) else {
            return;
        };
        l.state = LinkState::Up;
        l.diag.up_at = Some(now);
        self.diag.push(DiagEvent::LinkUp {
            link: id.0,
            kind: l.info.kind.lab(),
            role: Some(crate::diag::role(l.info.role)),
            mtu: l.att_mtu,
            psm: None,
        });
        l.next_ping_at = now;
        l.next_lstat_at = now.plus_us(1_000_000);
        if let (Some(c), false) = (caps, l.caps_sent) {
            l.caps_sent = true;
            if let Ok(body) = Record::Caps(c).encode_body() {
                l.queues.push(
                    Prio::P0,
                    OutItem::Record {
                        rtype: rtype::CAPS,
                        body,
                        deadline: None,
                    },
                );
            }
        }
        self.events.push(LinkEvent::Up(id));
    }

    /// The secure module verified the peer's IDPROOF: the link is Up.
    pub fn confirm_up(&mut self, now: Instant, id: LinkId, peer: NodeId) {
        if let Some(l) = self.links.get_mut(&id)
            && l.state == LinkState::IdProof
        {
            l.peer_node = Some(peer);
            self.set_up(now, id);
        }
    }

    /// Records the peer's node id (LAB_CTL announce on Lab links).
    pub fn set_peer(&mut self, id: LinkId, peer: NodeId) {
        if let Some(l) = self.links.get_mut(&id) {
            l.peer_node = Some(peer);
        }
    }

    /// The platform reported the link down: removes it and queues the event.
    pub fn on_down(&mut self, id: LinkId, reason: LinkDownReason) {
        self.remove(id, reason);
    }

    fn remove(&mut self, id: LinkId, reason: LinkDownReason) {
        if let Some(l) = self.links.remove(&id) {
            let up_s = l
                .diag
                .up_at
                .map_or(0, |t| self.diag_now.since(t) / 1_000_000);
            self.diag.push(DiagEvent::LinkDown {
                link: id.0,
                peer: l.peer_node.map(|n| n.0),
                reason: crate::diag::down_str(reason),
                up_s: u32::try_from(up_s).unwrap_or(u32::MAX),
                tx_b: l.counters.tx_bytes,
                rx_b: l.counters.rx_bytes,
            });
            self.events.push(LinkEvent::Down {
                link: id,
                kind: l.info.kind,
                reason,
                peer: l.peer_node,
                was_up: l.state == LinkState::Up || l.closing.is_some(),
                lab: l.is_lab(),
            });
        }
    }

    /// Marks every BLE link down with `reason` (Bluetooth off): no frame can leave.
    pub fn drop_ble(&mut self, reason: LinkDownReason) -> Vec<LinkId> {
        let ids: Vec<LinkId> = self
            .links
            .values()
            .filter(|l| l.info.kind.is_ble())
            .map(|l| l.id)
            .collect();
        for id in &ids {
            self.remove(*id, reason);
        }
        ids
    }

    /// Queues one record. Returns `false` if the link is unknown, not established, or
    /// an older item was dropped for overflow.
    pub fn send_record(
        &mut self,
        id: LinkId,
        prio: Prio,
        rec: &Record<'_>,
        deadline: Option<Instant>,
    ) -> bool {
        let Some(l) = self.links.get_mut(&id).filter(|l| l.established()) else {
            return false;
        };
        match rec.encode_body() {
            Ok(body) => l.queues.push(
                prio,
                OutItem::Record {
                    rtype: rec.rtype(),
                    body,
                    deadline,
                },
            ),
            Err(_) => false,
        }
    }

    /// Queues a complete record plaintext sealed alone (Lab probes keep their size).
    pub fn send_plaintext(&mut self, id: LinkId, prio: Prio, plaintext: Vec<u8>) -> bool {
        match self.links.get_mut(&id).filter(|l| l.established()) {
            Some(l) => l.queues.push(prio, OutItem::Frame { plaintext }),
            None => false,
        }
    }

    /// Closes a link: a CLOSE record is flushed first (when the link is established),
    /// then the platform close follows.
    pub fn close(&mut self, now: Instant, id: LinkId, reason: RejectReason, fx: &mut EffectQueue) {
        let Some(l) = self.links.get_mut(&id) else {
            return;
        };
        if l.closing.is_some() {
            return;
        }
        if l.established() {
            l.queues.clear();
            if let Ok(body) = (Record::Close(Close { reason })).encode_body() {
                l.queues.push(
                    Prio::P0,
                    OutItem::Record {
                        rtype: rtype::CLOSE,
                        body,
                        deadline: None,
                    },
                );
            }
            l.state = LinkState::Closing;
            l.closing = Some((reason, now.plus_us(CLOSE_FLUSH_US)));
        } else {
            self.reject(now, id, reason, fx);
        }
    }

    /// Closes a link once what is already queued up to priority `keep` has left: the
    /// lower-priority queues are dropped, nothing new is accepted (the link is Closing),
    /// the CLOSE record goes out after the kept items and the platform close follows
    /// as soon as the link drained. Ordering, not a timer, decides what the peer still
    /// reads (a removed member's denying ROSTER, a LEAVE, my IDPROOF); the deadline
    /// ([`CLOSE_FLUSH_US`], or [`CLOSE_DRAIN_US`] when P2 is kept) only bounds a link
    /// that cannot drain. A link that is not established yet is refused with REJECT
    /// instead (as [`LinkTable::close`]).
    pub fn close_graceful(
        &mut self,
        now: Instant,
        id: LinkId,
        reason: RejectReason,
        keep: Prio,
        fx: &mut EffectQueue,
    ) {
        let Some(l) = self.links.get_mut(&id) else {
            return;
        };
        if l.closing.is_some() {
            return;
        }
        if !l.established() {
            self.reject(now, id, reason, fx);
            return;
        }
        l.queues.retain_up_to(keep);
        if let Ok(body) = (Record::Close(Close { reason })).encode_body() {
            l.queues.push(
                keep,
                OutItem::Record {
                    rtype: rtype::CLOSE,
                    body,
                    deadline: None,
                },
            );
        }
        let bound = if keep >= Prio::P2 {
            CLOSE_DRAIN_US
        } else {
            CLOSE_FLUSH_US
        };
        l.state = LinkState::Closing;
        l.closing = Some((reason, now.plus_us(bound)));
    }

    /// Refuses a link with a plaintext REJECT frame, then closes it.
    pub fn reject(
        &mut self,
        now: Instant,
        id: LinkId,
        reason: RejectReason,
        _fx: &mut EffectQueue,
    ) {
        let Some(l) = self.links.get_mut(&id) else {
            return;
        };
        if l.closing.is_some() {
            return;
        }
        l.queues.clear();
        // REJECT carries reasons 1-8 only (§4.3); a CLOSE-only reason (redundant,
        // leaving) on a link that never got its keys just closes.
        let close_only = matches!(reason, RejectReason::Redundant | RejectReason::Leaving);
        if !close_only && let Ok(f) = (LinkFrame::Reject { reason }).encode() {
            queue_raw(l, &f);
        }
        l.state = LinkState::Closing;
        l.closing = Some((reason, now.plus_us(CLOSE_FLUSH_US)));
    }

    /// Closes a link at once, without telling the peer (timeouts, peer REJECT).
    pub fn close_now(
        &mut self,
        id: LinkId,
        reason: Option<RejectReason>,
        down: LinkDownReason,
        fx: &mut EffectQueue,
    ) {
        if self.links.contains_key(&id) {
            fx.radio(RadioCommand::Close { link: id, reason });
            self.remove(id, down);
        }
    }

    /// Feeds one platform unit; returns the authenticated record plaintexts.
    pub fn inbound(
        &mut self,
        now: Instant,
        id: LinkId,
        bytes: &[u8],
        fx: &mut EffectQueue,
    ) -> Vec<Vec<u8>> {
        let mut plaintexts = Vec::new();
        let Some(l) = self.links.get_mut(&id) else {
            return plaintexts;
        };
        let frames = match l.framer.inbound(bytes) {
            Inbound::Frames(f) => f,
            Inbound::Dropped => {
                l.counters.bad_frames += 1;
                l.diag.bad();
                return plaintexts;
            }
            Inbound::Fatal(_) => {
                self.close_now(
                    id,
                    Some(RejectReason::HandshakeFailure),
                    LinkDownReason::Error(-1),
                    fx,
                );
                return plaintexts;
            }
        };
        for f in frames {
            self.inbound_frame(now, id, &f, &mut plaintexts, fx);
            if !self.links.contains_key(&id) {
                break;
            }
        }
        plaintexts
    }

    fn inbound_frame(
        &mut self,
        now: Instant,
        id: LinkId,
        frame: &[u8],
        plaintexts: &mut Vec<Vec<u8>>,
        fx: &mut EffectQueue,
    ) {
        let Some(l) = self.links.get_mut(&id) else {
            return;
        };
        let Ok(parsed) = LinkFrame::decode(frame) else {
            l.counters.bad_frames += 1;
            l.diag.bad();
            return;
        };
        let diag = self.diag.on();
        match parsed {
            LinkFrame::Reject { reason } => {
                self.close_now(id, None, LinkDownReason::PeerRejected(reason), fx);
            }
            LinkFrame::Hs {
                step,
                proto_ver,
                noise_msg,
            } => {
                if l.state != LinkState::Handshake {
                    l.counters.bad_frames += 1;
                    l.diag.bad();
                    return;
                }
                if diag {
                    l.diag.rx(FrameType::Hs, frame.len());
                }
                l.last_rx = now;
                let mut out = Vec::new();
                let s = l.security.on_hs(now, step, proto_ver, noise_msg, &mut out);
                for f in out {
                    queue_raw(l, &f);
                }
                self.on_step(now, id, s, fx);
            }
            LinkFrame::Sec { .. } | LinkFrame::Lab { .. } => {
                if !l.established() && l.state != LinkState::Closing {
                    l.counters.bad_frames += 1;
                    l.diag.bad();
                    return;
                }
                match l.security.open(now, &parsed) {
                    Ok(pt) => {
                        l.last_rx = now;
                        l.counters.rx_frames += 1;
                        l.counters.rx_bytes += frame.len() as u64;
                        if diag {
                            let ty = if l.is_lab() {
                                FrameType::Lab
                            } else {
                                crate::diag::classify::plaintext_type(&pt)
                            };
                            l.diag.rx(ty, frame.len());
                        }
                        l.counters.win_rx_frames = l.counters.win_rx_frames.saturating_add(1);
                        l.counters.win_rx_bytes = l
                            .counters
                            .win_rx_bytes
                            .saturating_add(frame.len().try_into().unwrap_or(u32::MAX));
                        if l.state != LinkState::Closing {
                            plaintexts.push(pt);
                        }
                    }
                    Err(SecurityError::Reject(r)) => self.reject(now, id, r, fx),
                    Err(_) => {
                        l.counters.bad_frames += 1;
                        l.diag.bad();
                    }
                }
            }
        }
    }

    /// Handles a link-scope record. Returns `true` when the record was consumed here
    /// (PING, PONG, LSTAT, CLOSE, PAD); CAPS is stored and also returned `false` so the
    /// engine can fan it out to the modules.
    pub fn on_link_record(
        &mut self,
        now: Instant,
        id: LinkId,
        rec: &Record<'_>,
        fx: &mut EffectQueue,
    ) -> bool {
        let Some(l) = self.links.get_mut(&id) else {
            return true;
        };
        match *rec {
            Record::Ping(p) => {
                l.pongs.push(PendingPong {
                    id: p.id,
                    t_tx: p.t_tx,
                    rx_at: now,
                });
                true
            }
            Record::Pong(p) => {
                l.on_pong(now, p.id, p.t_tx, p.hold_us);
                true
            }
            Record::Lstat(s) => {
                l.peer_lstat = Some(s);
                true
            }
            Record::Caps(c) => {
                l.peer_caps = Some(c);
                false
            }
            Record::Close(c) => {
                self.close_now(id, None, LinkDownReason::PeerRejected(c.reason), fx);
                true
            }
            Record::Pad(_) => true,
            _ => false,
        }
    }

    /// Runs the per-link timers: PING, LSTAT, link death, handshake timeout and the
    /// close flush deadline.
    pub fn on_timer(&mut self, now: Instant, limits: &Limits, fx: &mut EffectQueue) {
        let ids: Vec<LinkId> = self.links.keys().copied().collect();
        for id in ids {
            let Some(l) = self.links.get(&id) else {
                continue;
            };
            let (closing, last_rx, state, created_at) =
                (l.closing, l.last_rx, l.state, l.created_at);
            if let Some((reason, at)) = closing {
                if now >= at {
                    self.close_now(id, Some(reason), LinkDownReason::LocalClose, fx);
                }
                continue;
            }
            if now.since(last_rx) >= limits.link_death_us {
                self.close_now(id, None, LinkDownReason::Timeout, fx);
                continue;
            }
            if matches!(state, LinkState::Handshake | LinkState::IdProof)
                && now.since(created_at) >= limits.handshake_timeout_us
            {
                self.reject(now, id, RejectReason::HandshakeFailure, fx);
                continue;
            }
            let ping_every = self.links.get(&id).map_or(limits.ping_idle_us, |l| {
                self.ping_period(l.info.kind, limits)
            });
            let Some(l) = self.links.get_mut(&id).filter(|l| l.state == LinkState::Up) else {
                continue;
            };
            // PING and LSTAT stay on the link's own grid (anchored at link up, not at
            // when the timer happened to run) so they keep landing in the same frame.
            if now >= l.next_ping_at {
                // Built when the next frame is sealed (`seal_ping`); one at a time.
                l.ping_due = true;
                l.next_ping_at = on_grid(l.next_ping_at, ping_every, now);
            } else if l.next_ping_at > now.plus_us(ping_every) {
                l.next_ping_at = now.plus_us(ping_every);
            }
            if now >= l.next_lstat_at {
                send_lstat(l);
                l.next_lstat_at = on_grid(l.next_lstat_at, limits.lstat_period_us, now);
            }
        }
    }

    /// Earliest timer of any link.
    pub fn next_deadline(&self, limits: &Limits) -> Option<Instant> {
        self.links
            .values()
            .map(|l| l.next_deadline(limits.link_death_us, limits.handshake_timeout_us))
            .min()
    }

    /// Radio metrics.
    pub fn on_metrics(&mut self, id: LinkId, rssi: Option<i16>, att_mtu: Option<u16>) {
        if let Some(l) = self.links.get_mut(&id) {
            l.rssi = rssi.or(l.rssi);
            if let Some(m) = att_mtu {
                l.att_mtu = Some(m);
                l.framer.set_att_mtu(m);
            }
        }
    }

    /// Turns the link layer's diagnostics on or off.
    pub fn set_diag(&mut self, on: bool) {
        self.diag.set(on);
    }

    /// Takes the buffered diagnostics events.
    pub(crate) fn take_diag(&mut self) -> Vec<DiagEvent> {
        self.diag.take()
    }

    /// Sets a link's pacing rate (`None` = unpaced).
    pub fn set_pace(&mut self, now: Instant, id: LinkId, rate_bps: Option<u32>) {
        if let Some(l) = self.links.get_mut(&id) {
            l.pace.set_rate(now, rate_bps);
        }
    }
}

/// Frames a raw LinkFrame (HS, REJECT) into units that bypass sealing.
pub(crate) fn queue_raw(l: &mut Link, frame: &[u8]) {
    l.diag.tx(FrameType::Hs, frame.len());
    if let Ok(units) = l.framer.outbound(frame) {
        l.units.extend(units);
    }
}

/// Builds the due PING of a link as its frame is sealed: `t_tx` is the sealing instant,
/// so the RTT never includes the local token-bucket wait (§5.6 `qd = srtt − min_rtt`),
/// and the record goes first in that frame.
pub(crate) fn seal_ping(l: &mut Link, now: Instant) {
    if !l.ping_due {
        return;
    }
    l.ping_due = false;
    if l.outstanding.len() >= MAX_OUTSTANDING_PINGS {
        l.outstanding.remove(0);
        l.counters.win_gaps = l.counters.win_gaps.saturating_add(1);
    }
    let id = l.next_ping_id;
    l.next_ping_id = id.wrapping_add(1);
    let rec = Record::Ping(Ping {
        id,
        t_tx: now.0 as u32,
    });
    if let Ok(body) = rec.encode_body() {
        l.queues.push_front(
            Prio::P0,
            OutItem::Record {
                rtype: rtype::PING,
                body,
                deadline: None,
            },
        );
        l.outstanding.push(OutstandingPing { id, sent_at: now });
        l.counters.pings_sent += 1;
    }
}

/// The first `due + k·period` after `now` (k ≥ 1): a periodic timer that ran late
/// keeps its phase instead of drifting by the lateness every period.
fn on_grid(due: Instant, period: u64, now: Instant) -> Instant {
    let period = period.max(1);
    let late = now.since(due);
    due.plus_us((late / period + 1) * period)
}

fn send_lstat(l: &mut Link) {
    let c = &mut l.counters;
    let s = Lstat {
        rx_frames: c.win_rx_frames.min(u32::from(u16::MAX)) as u16,
        rx_bytes: c.win_rx_bytes,
        gaps: c.win_gaps.min(u32::from(u16::MAX)) as u16,
        pace_kbps: l
            .pace
            .rate_bps
            .map_or(0, |r| (r / 1_000).min(u32::from(u16::MAX)) as u16),
        queue_ms: 0,
    };
    c.win_rx_frames = 0;
    c.win_rx_bytes = 0;
    c.win_gaps = 0;
    let s = Lstat {
        queue_ms: l.queue_ms(),
        ..s
    };
    if let Ok(body) = Record::Lstat(s).encode_body() {
        l.queues.push(
            Prio::P2,
            OutItem::Record {
                rtype: rtype::LSTAT,
                body,
                deadline: None,
            },
        );
    }
}

/// Builds the PONG records of a link with `hold_us` measured now (as its frame is
/// sealed, so the responder's token-bucket wait is excluded too).
pub(crate) fn flush_pongs(l: &mut Link, now: Instant) {
    for p in core::mem::take(&mut l.pongs) {
        let hold = now.since(p.rx_at).min(u64::from(u16::MAX)) as u16;
        let rec = Record::Pong(Pong {
            id: p.id,
            t_tx: p.t_tx,
            hold_us: hold,
        });
        if let Ok(body) = rec.encode_body() {
            l.queues.push(
                Prio::P0,
                OutItem::Record {
                    rtype: rtype::PONG,
                    body,
                    deadline: None,
                },
            );
        }
    }
}

#[cfg(test)]
#[path = "ping_tests.rs"]
mod ping_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::Effect;
    use crate::links::security::Plain;
    use crate::links::types::LinkRole;
    use mesh_wire::frame::lab_tag;
    use mesh_wire::record::RecordIter;

    const A: LinkId = LinkId(1);
    const B: LinkId = LinkId(2);

    /// Two tables joined by one Lab link (`A` on `a`, `B` on `b`), both Up.
    fn pair() -> (LinkTable, LinkTable, EffectQueue) {
        let mut fx = EffectQueue::new();
        let (mut a, mut b) = (LinkTable::new(), LinkTable::new());
        let tag = lab_tag(1234);
        let info = |role| LinkInfo::new(LinkKind::BleL2cap, role);
        let sec = || Box::new(Plain::new(tag));
        a.add(Instant(0), A, info(LinkRole::Initiator), sec(), &mut fx);
        b.add(Instant(0), B, info(LinkRole::Responder), sec(), &mut fx);
        assert_eq!(a.get(A).map(|l| l.state), Some(LinkState::Up));
        a.drain_events();
        b.drain_events();
        (a, b, fx)
    }

    /// Writes every unit `a` emits into `b`; returns the record types `b` read, in
    /// order (padding left out), after the link layer of `b` handled them.
    fn pump(now: Instant, a: &mut LinkTable, b: &mut LinkTable, fx: &mut EffectQueue) -> Vec<u8> {
        let mut seen = Vec::new();
        while let Some(t) = a.poll_transmit(now, fx) {
            for pt in b.inbound(now, B, &t.bytes, fx) {
                for rec in RecordIter::new(&pt).flatten() {
                    if rec.rtype() != rtype::PAD {
                        seen.push(rec.rtype());
                    }
                    b.on_link_record(now, B, &rec, fx);
                }
            }
        }
        seen
    }

    fn downs(t: &mut LinkTable) -> Vec<LinkDownReason> {
        t.drain_events()
            .into_iter()
            .filter_map(|e| match e {
                LinkEvent::Down { reason, .. } => Some(reason),
                _ => None,
            })
            .collect()
    }

    fn lstat() -> Record<'static> {
        Record::Lstat(Lstat {
            rx_frames: 1,
            rx_bytes: 2,
            gaps: 0,
            pace_kbps: 0,
            queue_ms: 0,
        })
    }

    #[test]
    fn graceful_close_sends_kept_records_then_close_and_the_peer_gets_the_reason() {
        let (mut a, mut b, mut fx) = pair();
        let now = Instant(1_000);
        assert!(a.send_record(A, Prio::P2, &lstat(), None));
        assert!(a.send_record(A, Prio::P0, &Record::Ping(Ping { id: 7, t_tx: 1 }), None));
        a.close_graceful(now, A, RejectReason::Leaving, Prio::P0, &mut fx);
        assert!(
            !a.send_record(A, Prio::P0, &Record::Ping(Ping { id: 8, t_tx: 2 }), None),
            "a closing link takes nothing new"
        );
        // The P0 record leaves, then CLOSE; the P2 record was dropped.
        assert_eq!(
            pump(now, &mut a, &mut b, &mut fx),
            [rtype::PING, rtype::CLOSE]
        );
        assert!(a.get(A).is_none(), "closed once drained, no timer needed");
        assert_eq!(downs(&mut a), [LinkDownReason::LocalClose]);
        assert_eq!(
            downs(&mut b),
            [LinkDownReason::PeerRejected(RejectReason::Leaving)]
        );
        let closes: Vec<_> = core::iter::from_fn(|| fx.pop())
            .filter_map(|e| match e {
                Effect::Radio(RadioCommand::Close { link, reason }) => Some((link, reason)),
                _ => None,
            })
            .collect();
        // The peer closes its side on the CLOSE record; mine follows once drained.
        assert_eq!(closes, [(B, None), (A, Some(RejectReason::Leaving))]);
    }

    #[test]
    fn graceful_close_keeps_lower_priorities_up_to_the_one_asked() {
        let (mut a, mut b, mut fx) = pair();
        let now = Instant(1_000);
        assert!(a.send_record(A, Prio::P2, &lstat(), None));
        a.close_graceful(
            now,
            A,
            RejectReason::MemberRemovedOrDenied,
            Prio::P2,
            &mut fx,
        );
        assert_eq!(
            pump(now, &mut a, &mut b, &mut fx),
            [rtype::LSTAT, rtype::CLOSE]
        );
        assert_eq!(
            downs(&mut b),
            [LinkDownReason::PeerRejected(
                RejectReason::MemberRemovedOrDenied
            )]
        );
        assert_eq!(
            LinkDownReason::PeerRejected(RejectReason::MemberRemovedOrDenied).peer_reason(),
            Some(RejectReason::MemberRemovedOrDenied)
        );
        assert!(LinkDownReason::PeerRejected(RejectReason::Leaving).peer_closed());
    }

    #[test]
    fn a_blocked_paced_link_reports_no_stale_pacing_deadline() {
        let (mut a, _b, mut fx) = pair();
        let limits = crate::config::Limits::for_platform(crate::config::Platform::Ios);
        a.set_pace(Instant(0), A, Some(80_000));
        assert!(a.send_record(A, Prio::P2, &lstat(), None));
        assert!(a.send_record(A, Prio::P2, &lstat(), None));
        let t = a.poll_transmit(Instant(0), &mut fx).expect("one frame");
        // The bucket is in debt and the platform refused the unit: `Writable` wakes the
        // link later, so its pacing instant (long past by now) is no deadline.
        a.on_would_block(A, &t.bytes);
        let now = Instant(1_000_000);
        a.on_timer(now, &limits, &mut fx);
        assert!(a.next_deadline(&limits).is_some_and(|d| d > now));
        a.on_writable(A);
        assert!(
            a.poll_transmit(now, &mut fx).is_some(),
            "the unit goes once writable"
        );
    }

    #[test]
    fn a_reject_frame_reason_reaches_the_peer_and_close_only_reasons_send_none() {
        let (mut a, mut b, mut fx) = pair();
        let now = Instant(1_000);
        a.reject(now, A, RejectReason::LabCodeMismatch, &mut fx);
        assert!(pump(now, &mut a, &mut b, &mut fx).is_empty());
        assert_eq!(
            downs(&mut b),
            [LinkDownReason::PeerRejected(RejectReason::LabCodeMismatch)]
        );
        // REJECT carries reasons 1-8 only: a CLOSE-only reason just closes.
        let (mut a, _b, mut fx) = pair();
        a.reject(now, A, RejectReason::Leaving, &mut fx);
        assert!(a.poll_transmit(now, &mut fx).is_none());
        assert!(a.get(A).is_none());
    }
}
