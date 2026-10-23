//! The minimal Lab flood for MESH_PROBE (DESIGN §12.1): TTL + dedup only, over Lab
//! links. The origin sends to every Lab link; a relay checks the first-hop origin,
//! dedups by `(origin, sess, seq)` (after the LAB frame's tag check, §5.3), delivers to
//! the bench runner, appends its `lab_node_id` to the trace and forwards on every other
//! Lab link with `ttl − 1`, `hops + 1` and `age += incoming one-way estimate`.

use mesh_routing::{Dedup, DedupVerdict};
use mesh_wire::mesh::{MeshHeader, MeshPacket, MeshProbe, PacketType, TrafficClass};
use mesh_wire::record::Record;

use crate::engine::Ctx;
use crate::links::{LinkState, Prio};
use crate::{Instant, LinkId};

/// Flood counters (diagnostics and tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FloodStats {
    pub originated: u64,
    pub delivered: u64,
    pub relayed: u64,
    pub duplicates: u64,
    pub dropped: u64,
}

/// A MESH_PROBE that reached this node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub origin: u16,
    pub probe: MeshProbe,
    /// Link traversals including the last one.
    pub hops: u8,
    /// Age at arrival (origin queueing + relays + incoming one-way estimate), µs.
    pub age_us: u64,
}

/// What an inbound MESH record did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloodOutcome {
    /// Delivered (and relayed when the TTL allowed it).
    Delivered(Delivery),
    /// A packet with my own id from another session: an id collision.
    Collision { their_sess: u8 },
    /// Dropped (duplicate, malformed, not a Lab probe, origin check).
    Dropped,
}

/// Flood state of one Lab session.
#[derive(Debug, Clone, Default)]
pub struct LabFlood {
    dedup: Dedup,
    seq: u16,
    pub stats: FloodStats,
}

fn lab_links(ctx: &Ctx, except: Option<LinkId>) -> Vec<LinkId> {
    ctx.links
        .iter()
        .filter(|l| l.state == LinkState::Up && l.is_lab() && Some(l.id) != except)
        .map(|l| l.id)
        .collect()
}

impl LabFlood {
    pub fn new(first_seq: u16) -> Self {
        LabFlood {
            dedup: Dedup::new(),
            seq: first_seq,
            stats: FloodStats::default(),
        }
    }

    /// Originates `probe` from `origin` on every Lab link.
    pub fn originate(&mut self, ctx: &mut Ctx, origin: u16, probe: &MeshProbe) {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        let header = MeshHeader::new(PacketType::MeshProbe, origin, ctx.sess, seq);
        // Our own packet must not come back as new.
        self.dedup
            .check_and_mark(ctx.now.routing(), origin, ctx.sess, seq);
        let Ok(payload) = probe.encode() else {
            return;
        };
        let Ok(bytes) = (MeshPacket {
            header,
            payload: &payload,
        })
        .encode() else {
            return;
        };
        self.stats.originated += 1;
        for link in lab_links(ctx, None) {
            ctx.links
                .send_record(link, Prio::P3, &Record::Mesh(&bytes), None);
        }
    }

    /// Handles a MESH record received on Lab link `link` from peer `peer` (its
    /// announced `lab_node_id`, if known).
    pub fn on_packet(
        &mut self,
        ctx: &mut Ctx,
        link: LinkId,
        peer: Option<u16>,
        me: u16,
        body: &[u8],
    ) -> FloodOutcome {
        let now = ctx.now;
        let Ok(pkt) = MeshPacket::decode(body) else {
            self.stats.dropped += 1;
            return FloodOutcome::Dropped;
        };
        let h = pkt.header;
        if h.ptype != PacketType::MeshProbe || h.class != TrafficClass::Lab {
            // Lab links never carry group traffic.
            self.stats.dropped += 1;
            return FloodOutcome::Dropped;
        }
        if h.hops == 0 && peer.is_some_and(|p| p != h.origin) {
            // First-hop origin check (§5.2).
            self.stats.dropped += 1;
            return FloodOutcome::Dropped;
        }
        if h.origin == me && h.sess != ctx.sess {
            return FloodOutcome::Collision { their_sess: h.sess };
        }
        if self
            .dedup
            .check_and_mark(now.routing(), h.origin, h.sess, h.seq)
            != DedupVerdict::New
        {
            self.stats.duplicates += 1;
            return FloodOutcome::Dropped;
        }
        let Ok(mut probe) = MeshProbe::decode(pkt.payload) else {
            self.stats.dropped += 1;
            return FloodOutcome::Dropped;
        };
        let owd = ctx.links.get(link).map_or(0, |l| l.owd_us());
        let delivery = Delivery {
            origin: h.origin,
            probe,
            hops: h.hops.saturating_add(1).min(MeshHeader::MAX_HOPS),
            age_us: h.age_us() + owd,
        };
        self.stats.delivered += 1;
        if h.ttl > 1 && h.origin != me {
            // A full trace keeps its first 8 relays; the probe still travels.
            let _ = probe.append(me);
            if let (Some(rh), Ok(payload)) = (h.relayed(owd), probe.encode())
                && let Ok(bytes) = (MeshPacket {
                    header: rh,
                    payload: &payload,
                })
                .encode()
            {
                let targets = lab_links(ctx, Some(link));
                if !targets.is_empty() {
                    self.stats.relayed += 1;
                }
                for l in targets {
                    ctx.links
                        .send_record(l, Prio::P3, &Record::Mesh(&bytes), None);
                }
            }
        }
        FloodOutcome::Delivered(delivery)
    }

    /// Expires silent dedup windows.
    pub fn expire(&mut self, now: Instant) {
        self.dedup.expire(now.routing());
    }
}
