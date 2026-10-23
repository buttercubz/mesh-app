//! The air: OS send queues, the transmit timeline of each link direction (connection
//! events, serialization, retransmissions, node airtime budget), delivery, and the
//! link lifecycle (open, supervision drop, close, teardown).

use mesh_core::effects::Transmit;
use mesh_core::{
    Instant, LinkDownReason, LinkId, LinkInfo, LinkKind, LinkRole, Platform, PlatformEvent,
};

use super::{Ev, World};
use crate::audit::ViolationKind;
use crate::link::{Dir, End, SimLink};

/// ATT MTU each platform negotiates (§9.5): iOS 185, Android 517.
pub(crate) fn platform_mtu(p: Platform) -> u16 {
    match p {
        Platform::Android => 517,
        _ => 185,
    }
}

/// Delivered units kept per link direction for replays.
const RECENT_UNITS: usize = 16;

/// Burst tolerance of the node airtime budget: one iOS connection interval of airtime.
pub const AIR_DEPTH_US: u64 = 30_000;

impl World {
    /// The platform `send` of one unit: accepted into the OS queue or refused with
    /// WouldBlock (delivered to the engine synchronously, as a non-blocking send would).
    pub(crate) fn send(&mut self, i: usize, t: Transmit) {
        let Some(&li) = self.link_index.get(&(i, t.link.0)) else {
            return;
        };
        let p = self.cfg.radio;
        let l = &mut self.links[li];
        let Some(d) = l.dir_of(i, t.link).filter(|_| l.up) else {
            return;
        };
        if l.closing_by.is_some_and(|c| c != d) {
            // The peer already closed: the bytes go nowhere.
            return;
        }
        if l.would_block(d, t.bytes.len(), p.stream_buffer_bytes, p.gatt_buffer_units) {
            l.dirs[d].want_writable = true;
            l.dirs[d].would_blocks += 1;
            let now = self.now;
            self.nodes[i].engine.handle_event(
                now,
                PlatformEvent::WouldBlock {
                    link: t.link,
                    unit: &t.bytes,
                },
            );
            self.dirty.insert(i);
            return;
        }
        let (unit, att) = (l.unit, l.att_mtu.unwrap_or(23));
        let dir = &mut l.dirs[d];
        dir.queued_bytes += t.bytes.len();
        let start_pump = !dir.pumping;
        dir.pumping = true;
        let now = self.now;
        self.audit.on_air(now, i, t.link.0, unit, att, &t.bytes);
        if self.capture.contains(&li) {
            self.captured.push((li, d, now.0, t.bytes.clone()));
        }
        self.links[li].dirs[d].queue.push_back(t.bytes);
        self.links[li].dirs[d].enq.push_back(now.0);
        if start_pump {
            self.schedule(now.0, Ev::Pump { link: li, dir: d });
        }
    }

    fn pdr_of_link(&self, li: usize) -> f64 {
        let l = &self.links[li];
        let (a, b) = (l.ends[0].node, l.ends[1].node);
        if self.is_blocked(a, b) {
            return 0.0;
        }
        let (ma, mb) = (self.nodes[a].spec.mount, self.nodes[b].spec.mount);
        let pdr = self
            .radio
            .pdr(l.kind, a, b, ma, mb, self.distance(a, b), self.now.0);
        if l.kind.is_ble() {
            pdr * self.cfg.radio.scan.availability(self.link_miss(a, b))
        } else {
            pdr
        }
    }

    /// Puts the head unit of direction `d` of link `li` on air.
    pub(crate) fn pump(&mut self, li: usize, d: usize) {
        let now = self.now.0;
        let pdr = self.pdr_of_link(li) * (1.0 - self.cfg.radio.extra_loss.clamp(0.0, 1.0));
        let l = &self.links[li];
        if !l.up || l.dirs[d].queue.is_empty() {
            self.links[li].dirs[d].pumping = false;
            return;
        }
        let sender = l.ends[d].node;
        let kind = l.kind;
        let len = l.dirs[d].queue.front().map_or(0, Vec::len);
        let ser = self.radio.serialization_us(kind, len);
        let (start, end, deliver_at) = if kind.is_ble() {
            if pdr < self.cfg.radio.stall_pdr {
                // Nothing gets through: retry at the next radio tick.
                self.schedule(now + self.cfg.radio_tick_us, Ev::Pump { link: li, dir: d });
                return;
            }
            let cap = self.radio.max_attempts(l.ci_us).saturating_sub(1);
            let ci = l.ci_us;
            let fails = self.rng.geometric_failures(pdr, cap);
            let attempts = u64::from(fails) + 1;
            let budget = self.cfg.limits_for(sender).node_ble_budget_bps();
            let headset = self.nodes[sender].spec.headset;
            // Node airtime budget (leaky bucket, see `AirBucket`): the first attempt
            // is charged when it is scheduled, retransmissions over the events they
            // occupy.
            let air1 = self.radio.airtime_us(len, budget, headset) as f64;
            let bucket = &mut self.nodes[sender].air;
            bucket.update(now);
            let air_ok = bucket.ready_at(now, AIR_DEPTH_US as f64);
            let dir = &l.dirs[d];
            let base = now.max(dir.busy_until).max(air_ok);
            // A busy link continues in the same connection event; an idle one waits
            // for the next event (the CI phase).
            let start = if dir.busy_until >= now && base == now.max(dir.busy_until) {
                base
            } else {
                l.next_event(base)
            };
            let end = start + u64::from(fails) * ci + ser;
            let n = &mut self.nodes[sender];
            n.air.level_us += air1;
            if fails > 0 {
                n.air
                    .retx
                    .push((li, start + ci, end, f64::from(fails) * air1, 0.0));
            }
            let air = attempts * air1 as u64;
            n.air_used_us += air;
            n.ble_tx_bytes += len as u64;
            self.links[li].dirs[d].attempts += attempts;
            (start, end, Some(end))
        } else {
            let start = now.max(l.dirs[d].busy_until);
            let end = start + ser;
            let p = self.cfg.radio;
            let jitter = self.rng.range_u64(0, 2 * p.aware_jitter_us);
            let delay = (p.aware_delay_us + jitter).saturating_sub(p.aware_jitter_us);
            let ok = self.rng.chance(pdr);
            self.links[li].dirs[d].attempts += 1;
            (start, end, ok.then_some(end + delay))
        };
        let l = &mut self.links[li];
        let dir: &mut Dir = &mut l.dirs[d];
        let mut bytes = dir.queue.pop_front().expect("head unit");
        let enq = dir.enq.pop_front().unwrap_or(now);
        if let Some(at) = deliver_at {
            let dl = at.saturating_sub(enq);
            dir.delay_sum_us += dl;
            dir.delay_n += 1;
            dir.delay_max_us = dir.delay_max_us.max(dl);
        }
        dir.queued_bytes -= bytes.len();
        dir.busy_until = end;
        let more = !dir.queue.is_empty();
        let wake = std::mem::take(&mut dir.want_writable);
        if deliver_at.is_none() {
            dir.lost += 1;
        }
        let sender_link = l.ends[d].link;
        self.maybe_corrupt(li, d, &mut bytes);
        if let Some(at) = deliver_at {
            self.schedule(
                at,
                Ev::Deliver {
                    link: li,
                    to: 1 - d,
                    bytes,
                },
            );
        }
        if wake {
            self.schedule(
                start,
                Ev::Writable {
                    node: sender,
                    link: sender_link,
                },
            );
        }
        if more {
            self.schedule(end, Ev::Pump { link: li, dir: d });
        } else {
            self.links[li].dirs[d].pumping = false;
        }
    }

    /// A unit reaches end `to` of link `li`.
    pub(crate) fn deliver(&mut self, li: usize, to: usize, bytes: &[u8]) {
        let l = &mut self.links[li];
        if !l.up || l.closing_by == Some(to) {
            return;
        }
        let now = self.now;
        let d = &mut l.dirs[1 - to];
        d.delivered += 1;
        d.last_delivery_us = now.0;
        d.bytes += bytes.len() as u64;
        if d.recent.len() >= RECENT_UNITS {
            d.recent.pop_front();
        }
        d.recent.push_back(bytes.to_vec());
        let End { node, link } = l.ends[to];
        self.stats.deliveries += 1;
        self.fold_digest(&[now.0, node as u64, u64::from(link.0)], bytes);
        self.nodes[node].engine.handle_packet(now, link, bytes);
        self.dirty.insert(node);
    }

    /// Opens a link `a` (initiator) → `b` (responder) of `kind`; both engines get `Up`.
    pub(crate) fn create_link(
        &mut self,
        a: usize,
        b: usize,
        kind: LinkKind,
        disc: Option<u32>,
    ) -> usize {
        let (pa, pb) = (self.nodes[a].spec.platform, self.nodes[b].spec.platform);
        let now = self.now;
        if kind.is_ble() && pa == Platform::Android && pb == Platform::Ios {
            self.audit.report(
                now,
                ViolationKind::RoleRule,
                format!("Android node {a} initiated a BLE link to iPhone {b}"),
            );
        }
        let la = LinkId(self.next_link_id);
        let lb = LinkId(self.next_link_id + 1);
        self.next_link_id += 2;
        let ci = if kind.is_ble() {
            self.radio.ble_ci_us(pa, pb)
        } else {
            1_000
        };
        let anchor = self.rng.range_u64(0, ci - 1);
        let att = (kind == LinkKind::BleGatt).then(|| platform_mtu(pa).min(platform_mtu(pb)));
        let li = self.links.len();
        self.links.push(SimLink {
            ends: [End { node: a, link: la }, End { node: b, link: lb }],
            kind,
            unit: kind.unit(),
            up: true,
            ci_us: ci,
            anchor_us: now.0 + anchor,
            dirs: [Dir::default(), Dir::default()],
            below_since: None,
            created_at: now.0,
            down_at: None,
            att_mtu: att,
            closing_by: None,
        });
        self.link_index.insert((a, la.0), li);
        self.link_index.insert((b, lb.0), li);
        self.stats.links_created += 1;
        for (node, link, role, disc) in [
            (a, la, LinkRole::Initiator, disc),
            (b, lb, LinkRole::Responder, None),
        ] {
            let mut info = LinkInfo::new(kind, role);
            info.att_mtu = att;
            info.disc_id = disc;
            self.nodes[node]
                .engine
                .handle_event(now, PlatformEvent::Up { link, info });
            self.dirty.insert(node);
        }
        li
    }

    fn unlink(&mut self, li: usize) {
        let l = &mut self.links[li];
        l.up = false;
        l.down_at = Some(self.now.0);
        for d in 0..2 {
            let dir = &mut l.dirs[d];
            dir.lost += dir.queue.len() as u64;
            dir.queue.clear();
            dir.enq.clear();
            dir.queued_bytes = 0;
            dir.want_writable = false;
        }
        let ends = l.ends;
        for e in ends {
            self.nodes[e.node].air.drop_link(li);
            self.link_index.remove(&(e.node, e.link.0));
            self.audit.forget_link(e.node, e.link.0);
        }
        self.stats.links_dropped += 1;
    }

    /// Takes link `li` down; both engines get `Down` with their reason.
    pub(crate) fn drop_link(&mut self, li: usize, reasons: [LinkDownReason; 2]) {
        if !self.links[li].up {
            return;
        }
        let ends = self.links[li].ends;
        self.unlink(li);
        let now = self.now;
        for (e, reason) in ends.into_iter().zip(reasons) {
            self.nodes[e.node].engine.handle_event(
                now,
                PlatformEvent::Down {
                    link: e.link,
                    reason,
                },
            );
            self.dirty.insert(e.node);
        }
    }

    /// Takes the link down because node `gone` vanished (app restart): only the peer
    /// is told (supervision timeout).
    pub(crate) fn drop_link_quiet(&mut self, li: usize, gone: usize) {
        if !self.links[li].up {
            return;
        }
        let ends = self.links[li].ends;
        self.unlink(li);
        let now = self.now;
        for e in ends.into_iter().filter(|e| e.node != gone) {
            self.nodes[e.node].engine.handle_event(
                now,
                PlatformEvent::Down {
                    link: e.link,
                    reason: LinkDownReason::Timeout,
                },
            );
            self.dirty.insert(e.node);
        }
    }

    /// `RadioCommand::Close` from `node`: the link is torn down once the closer's
    /// queued units are on air (CLOSE/REJECT reach the peer); the peer sees PeerClosed.
    pub(crate) fn close_from(&mut self, node: usize, link: LinkId) {
        let Some(&li) = self.link_index.get(&(node, link.0)) else {
            return;
        };
        let Some(d) = self.links[li].dir_of(node, link) else {
            return;
        };
        if self.links[li].closing_by.is_some() {
            return;
        }
        self.links[li].closing_by = Some(d);
        // The closer's engine forgets the link now; only the peer may still receive.
        self.link_index.remove(&(node, link.0));
        let t = self.links[li].dirs[d].busy_until.max(self.now.0) + 1;
        self.schedule(t, Ev::Teardown { link: li });
    }

    pub(crate) fn teardown(&mut self, li: usize) {
        let l = &self.links[li];
        let Some(d) = l.closing_by.filter(|_| l.up) else {
            return;
        };
        if !l.dirs[d].queue.is_empty() || l.dirs[d].busy_until > self.now.0 {
            let t = l.dirs[d].busy_until.max(self.now.0) + 1_000;
            self.schedule(t, Ev::Teardown { link: li });
            return;
        }
        let peer = l.ends[1 - d];
        // In-flight deliveries to the peer land before the teardown event (their
        // times are ≤ busy_until), so the peer has everything.
        self.unlink(li);
        let now: Instant = self.now;
        self.nodes[peer.node].engine.handle_event(
            now,
            PlatformEvent::Down {
                link: peer.link,
                reason: LinkDownReason::PeerClosed,
            },
        );
        self.dirty.insert(peer.node);
    }
}
