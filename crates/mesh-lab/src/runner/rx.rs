//! Link lifecycle, LAB frame transmit and receive, MESH_PROBE handling.

use mesh_wire::mesh::MeshProbe;
use mesh_wire::record::{LabCtlOp, Lecho, Lprobe, Record, RecordIter, RecordWriter};
use mesh_wire::{Result as WireResult, frame::SEC_OVERHEAD, link::MAX_STREAM_FRAME};

use super::{BenchRunner, GPS_PERIOD_US, LabTx, LinkState, PendingReply};
use crate::clock::ClockEstimator;
use crate::geo;
use crate::json::Json;
use crate::kinds::{LinkKind, LinkRole};
use crate::linkstats::Stats;
use crate::record::{RawMeshProbe, RawPing};
use crate::time::Instant;
use crate::traffic::{TrafficGen, flags};

/// Records bytes that fit one stream LinkFrame padded to the SEC size.
const MAX_RECORDS: usize = MAX_STREAM_FRAME - SEC_OVERHEAD;

impl BenchRunner {
    /// A Lab link came up (or came back). `peer` is the peer's `lab_node_id` once
    /// announced (`LAB_CTL{announce}`); a new link to a peer whose old link is down
    /// closes that rejoin and records its duration.
    pub fn on_link_up(
        &mut self,
        now: Instant,
        link: u32,
        kind: LinkKind,
        role: Option<LinkRole>,
        peer: Option<u16>,
    ) {
        if self.is_finished() {
            return;
        }
        let mut rejoin_us = None;
        if let Some(p) = peer {
            let stale: Vec<u32> = self
                .links
                .iter()
                .filter(|(id, l)| **id != link && !l.up && l.peer == Some(p))
                .map(|(id, _)| *id)
                .collect();
            for id in stale {
                if let Some(old) = self.links.remove(&id) {
                    rejoin_us = old.down_at.map(|d| now.0.saturating_sub(d));
                }
            }
        }
        match self.links.get_mut(&link) {
            Some(l) => {
                if !l.up {
                    rejoin_us = rejoin_us.or(l.down_at.map(|d| now.0.saturating_sub(d)));
                }
                l.up = true;
                l.down_at = None;
                l.kind = kind;
                l.role = role.or(l.role);
                l.peer = peer.or(l.peer);
            }
            None => {
                let state = LinkState {
                    kind,
                    role,
                    peer,
                    up: true,
                    down_at: None,
                    traffic: TrafficGen::new(&self.cfg.params, now),
                    stats: Stats::with_nominal_rate(now.0, self.cfg.params.nominal_probe_rate()),
                    clock: ClockEstimator::default(),
                    replies: Vec::new(),
                    rx_count: 0,
                    rssi: None,
                    max_write: None,
                    gps_due_us: now.0,
                    raw_pings: Vec::new(),
                    last_window: None,
                };
                self.links.insert(link, state);
            }
        }
        let mut fields = vec![
            ("link".to_string(), Json::from(link)),
            ("kind".to_string(), Json::from(kind.as_str())),
        ];
        if let Some(p) = peer {
            fields.push(("peer".to_string(), Json::from(p)));
        }
        if let Some(us) = rejoin_us {
            let s = us as f64 / 1e6;
            self.facts.rejoins_s.push(s);
            fields.push((
                "reconnectS".to_string(),
                Json::Num(crate::json::round(s, 3)),
            ));
        }
        self.event(now, "linkUp", fields);
    }

    /// A Lab link went down (`reason`: `timeout`, `closed`, `blacklist`, …).
    pub fn on_link_down(&mut self, now: Instant, link: u32, reason: &str) {
        if self.is_finished() {
            return;
        }
        let Some(l) = self.links.get_mut(&link) else {
            return;
        };
        if !l.up {
            return;
        }
        l.up = false;
        l.down_at = Some(now.0);
        l.replies.clear();
        l.stats.on_link_down();
        let fields = vec![
            ("link".to_string(), Json::from(link)),
            ("reason".to_string(), Json::from(reason)),
        ];
        self.event(now, "linkDown", fields);
    }

    /// Radio details from the platform: RSSI (dBm) and the ATT max write (GATT links).
    pub fn on_link_info(&mut self, link: u32, rssi: Option<i16>, max_write: Option<u16>) {
        if let Some(l) = self.links.get_mut(&link) {
            l.rssi = rssi.or(l.rssi);
            l.max_write = max_write.or(l.max_write);
        }
    }

    /// Records for the next LAB frame on `link`, or `None` when nothing is due.
    /// `writable` is false while the link's transmit queue is full. Call repeatedly
    /// until it returns `None`. Ping answers and LAB_GPS share one frame; every
    /// generated probe travels alone so its frame keeps the exact VOICE size.
    pub fn poll_tx(&mut self, now: Instant, link: u32, writable: bool) -> Option<LabTx> {
        if self.is_finished() || !writable {
            return None;
        }
        let gps = if self.cfg.params.gps {
            self.platform
                .as_ref()
                .and_then(|p| p.gps.map(|g| (g, p.heading_deg)))
        } else {
            None
        };
        let t_ms = (now.since(self.start) / 1_000) as u32;
        let l = self.links.get_mut(&link).filter(|l| l.up)?;
        let mut w = RecordWriter::with_capacity_limit(MAX_RECORDS);
        let mut probes = 0u64;
        // A ping answer is a LECHO (22 B record) plus a reply LPROBE (15 B record).
        const REPLY_BYTES: usize = 22 + 15;
        let replies = core::mem::take(&mut l.replies);
        for r in replies {
            if w.remaining() < REPLY_BYTES {
                l.replies.push(r);
                continue;
            }
            let hold = now.0.saturating_sub(r.rx_at).min(u64::from(u32::MAX)) as u32;
            push(
                &mut w,
                &Record::Lecho(Lecho {
                    probe_id: r.probe_id,
                    t_tx: r.t_tx,
                    hold_us: hold,
                    rx_count: l.rx_count,
                }),
            );
            push(
                &mut w,
                &Record::Lprobe(Lprobe {
                    probe_id: r.probe_id,
                    t_tx: now.0,
                    flags: flags::REPLY,
                    pad_len: 0,
                }),
            );
        }
        if let Some((fix, heading)) = gps
            && now.0 >= l.gps_due_us
            && w.remaining() >= 20
        {
            l.gps_due_us = now.0 + GPS_PERIOD_US;
            push(
                &mut w,
                &Record::LabGps(geo::to_lab_gps(&fix, t_ms, heading)),
            );
        }
        if w.count() == 0
            && let Some(p) = l.traffic.poll(now, true)
        {
            push(&mut w, &Record::Lprobe(p.lprobe));
            probes = 1;
        }
        if w.count() == 0 {
            return None;
        }
        let records = w.into_bytes();
        let on_air = records.len() + SEC_OVERHEAD + l.kind.framing_overhead();
        l.stats.on_tx(on_air, probes);
        Some(LabTx {
            link,
            records,
            on_air,
        })
    }

    /// Handles the records of a LAB frame received on `link` (`on_air` = its on-air
    /// size). Malformed records end processing of the frame with the wire error.
    pub fn on_rx(
        &mut self,
        now: Instant,
        link: u32,
        on_air: usize,
        records: &[u8],
    ) -> WireResult<()> {
        if self.is_finished() {
            return Ok(());
        }
        let ts_now = self.ts(now);
        let start = self.start.0;
        let Some(l) = self.links.get_mut(&link) else {
            return Ok(());
        };
        l.stats.on_rx(on_air);
        let mut marks = Vec::new();
        let mut peer_fix = None;
        let mut it = RecordIter::new(records);
        // The LECHO just read (index into raw_pings, t1, t4, hold), completed by the
        // reply LPROBE that follows it in the same frame.
        let mut last_echo: Option<(usize, u32, u64, u64, u64)> = None;
        while let Some(raw) = it.next_raw() {
            match raw?.parse()? {
                Record::Lprobe(p) => match flags::class(p.flags) {
                    flags::REPLY => {
                        if let Some((i, id, t1, t4, hold)) = last_echo.take()
                            && id == p.probe_id
                        {
                            if let Some(rp) = l.raw_pings.get_mut(i) {
                                rp.t3_remote_s = Some(p.t_tx as f64 / 1e6);
                            }
                            l.clock.add_ping(t1, t4, p.t_tx, hold);
                        }
                    }
                    class => {
                        l.rx_count = l.rx_count.wrapping_add(1);
                        l.stats.on_probe(p.probe_id, now.0);
                        if let Some(owd) = l.clock.one_way_us(p.t_tx, now.0) {
                            l.stats.on_owd(owd);
                        }
                        if class == flags::PING {
                            l.replies.push(PendingReply {
                                probe_id: p.probe_id,
                                t_tx: p.t_tx,
                                rx_at: now.0,
                            });
                        }
                    }
                },
                Record::Lecho(e) => {
                    let rtt = now
                        .0
                        .saturating_sub(e.t_tx)
                        .saturating_sub(u64::from(e.hold_us));
                    l.stats.on_rtt(rtt);
                    l.stats.on_peer_rx_count(e.rx_count);
                    let rel = |t: u64| t.saturating_sub(start) as f64 / 1e6;
                    l.raw_pings.push(RawPing {
                        probe_id: e.probe_id,
                        t1_s: rel(e.t_tx),
                        t4_s: rel(now.0),
                        hold_s: f64::from(e.hold_us) / 1e6,
                        t3_remote_s: None,
                    });
                    last_echo = Some((
                        l.raw_pings.len() - 1,
                        e.probe_id,
                        e.t_tx,
                        now.0,
                        u64::from(e.hold_us),
                    ));
                }
                Record::LabGps(g) => peer_fix = Some(geo::from_lab_gps(&g)),
                Record::LabCtl(c) if c.op == LabCtlOp::Mark => {
                    marks.push(
                        String::from_utf8_lossy(c.params)
                            .chars()
                            .take(64)
                            .collect::<String>(),
                    );
                }
                // PAD, LAB_CTL start/stop/announce (mesh-core) and unknown records.
                _ => {}
            }
        }
        if let Some(fix) = peer_fix {
            let key = l.peer.unwrap_or(link as u16);
            self.peer_gps.insert(key, fix);
        }
        for label in marks {
            self.out
                .push(crate::record::LabRecord::Mark { ts: ts_now, label });
        }
        Ok(())
    }

    /// T0b origin: the next MESH_PROBE to flood (mesh-core wraps it in a MeshPacket of
    /// class `lab`, TTL 8, and relays append their node id to the trace).
    pub fn poll_mesh_probe(&mut self, now: Instant) -> Option<MeshProbe> {
        if self.is_finished() {
            return None;
        }
        self.mesh_gen.as_mut()?.poll(now)
    }

    /// A MESH_PROBE from `origin` arrived after `hops` link traversals with MeshPacket
    /// `age` = `age_us` (origin queueing + per-hop residence + link one-way estimates).
    pub fn on_mesh_probe(
        &mut self,
        now: Instant,
        origin: u16,
        probe: &MeshProbe,
        hops: u8,
        age_us: u64,
    ) {
        if self.is_finished() {
            return;
        }
        if hops > 0 {
            self.hop_ms.push(age_us as f64 / 1e3 / f64::from(hops));
        }
        self.raw_mesh.push(RawMeshProbe {
            origin,
            probe_id: probe.probe_id,
            t_origin_s: probe.t_origin as f64 / 1e6,
            t_rx_s: self.ts(now),
            hops,
            age_ms: age_us as f64 / 1e3,
            trace: probe.trace().to_vec(),
        });
    }
}

fn push(w: &mut RecordWriter, r: &Record<'_>) {
    // Callers check the remaining space first (replies), or push into an empty frame
    // (GPS 20 B, one probe ≤ 1003 B), so a refusal cannot happen; ignoring it would
    // only drop that record.
    let _ = w.push(r);
}
