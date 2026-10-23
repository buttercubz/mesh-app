//! Origination and local delivery of the group's control packets (DESIGN §4.5-§4.7,
//! §5.2-§5.3): PRESENCE, ROSTER / JOIN_REQ / REKEY segments, LEAVE, RIDE_CTL, GROUP_CTL
//! and POSITION.
//!
//! Origination sends to **every** neighbor (the origin never prunes, §5.2): packets are
//! handed to the mesh glue ([`Ctx::originate`]), which sends them through the same
//! per-link staging, pacing and airtime budget as relayed packets, or, right before the
//! caller closes links, queued on every Up link at once ([`Ctx::originate_now`]). Every
//! `seq` comes from the engine's shared per-origin counter (one dedup window per
//! `(origin, sess)` at every receiver, §5.3). Delivery happens only for authenticated
//! records (the link layer opened the SEC frame), after the first-hop origin check and
//! the `(origin, sess, seq)` dedup.
//! Records are delivered without being claimed, so the mesh module (WP-10D) still sees
//! every MESH record and owns relaying.

use mesh_routing::DedupVerdict;
use mesh_wire::mesh::{
    AssemblyEvent, Leave, MeshHeader, MeshPacket, MeshPayload, PacketType, Position, Presence,
    RideCtl, segment_packets,
};

use super::active::Active;
use crate::engine::{Ctx, OriginPacket};
use crate::links::Prio;
use crate::session::Out;

/// A packet delivered to this node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered {
    Presence {
        origin: u16,
        sess: u8,
        hops: u8,
        age_us: u64,
        p: Presence,
    },
    /// A reassembled ROSTER / JOIN_REQ / REKEY body.
    Body {
        ptype: PacketType,
        origin: u16,
        body: Vec<u8>,
    },
    Leave {
        origin: u16,
        sess: u8,
        leave: Leave,
    },
    RideCtl {
        origin: u16,
        ctl: RideCtl,
    },
    GroupCtl {
        body: Vec<u8>,
    },
    Position {
        origin: u16,
        p: Position,
    },
}

fn prio(ptype: PacketType) -> Prio {
    match ptype {
        // Control that silences or reshapes the group goes ahead of presence/bulk.
        PacketType::Leave | PacketType::RideCtl | PacketType::GroupCtl => Prio::P0,
        _ => Prio::P2,
    }
}

/// How an originated packet reaches the links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Through the mesh glue: per-link staging, pacing, airtime budget (§5.2, §5.6).
    Mesh,
    /// Straight onto every Up link's queue: the caller closes links right behind it.
    BeforeClose,
}

/// Floods `out` as origin `me` to every neighbor.
pub fn originate(ctx: &mut Ctx, act: &mut Active, me: u16, leader: bool, out: &Out, via: Via) {
    if act.up_links().is_empty() {
        return;
    }
    let mut header = MeshHeader::new(out.ptype, me, ctx.sess, 0);
    header.leader = leader;
    let packets: Vec<Vec<u8>> = if out.ptype.is_segmented() {
        let msg_id = act.next_msg_id();
        match segment_packets(&header, ctx.mesh_seq, msg_id, &out.payload) {
            Ok(p) => {
                // Each segment is its own packet with its own seq (§4.6).
                ctx.reserve_mesh_seqs(p.len().min(usize::from(u16::MAX)) as u16);
                p
            }
            Err(_) => return,
        }
    } else {
        header.seq = ctx.next_mesh_seq();
        let payload = MeshPacket {
            header,
            payload: &out.payload,
        };
        match payload.encode() {
            Ok(p) => vec![p],
            Err(_) => return,
        }
    };
    for body in packets {
        if let Ok(h) = MeshHeader::decode(&body) {
            // My own packets echoed back by relays are duplicates.
            act.dedup
                .check_and_mark(ctx.now.routing(), me, h.sess, h.seq);
        }
        let p = OriginPacket {
            prio: prio(out.ptype),
            body,
            deadline: None,
        };
        match via {
            Via::Mesh => ctx.originate(p),
            Via::BeforeClose => {
                ctx.originate_now(&p);
            }
        }
    }
}

/// An authenticated MESH record from the Up link `link` whose peer is `peer`.
pub fn ingest(ctx: &Ctx, act: &mut Active, me: u16, peer: u16, bytes: &[u8]) -> Option<Delivered> {
    let pkt = MeshPacket::decode(bytes).ok()?;
    let h = pkt.header;
    if h.origin == me {
        return None;
    }
    // First-hop origin check (§5.2).
    if h.hops == 0 && h.origin != peer {
        return None;
    }
    let now = ctx.now.routing();
    let origin = h.origin;
    if act.dedup.check_and_mark(now, origin, h.sess, h.seq) != DedupVerdict::New {
        return None;
    }
    let payload = MeshPayload::decode(&pkt).ok()?;
    match payload {
        MeshPayload::Presence(p) => {
            act.dedup.on_presence(origin, h.sess);
            Some(Delivered::Presence {
                origin: h.origin,
                sess: h.sess,
                hops: h.hops,
                age_us: h.age_us(),
                p,
            })
        }
        MeshPayload::Segment(ptype, seg) => {
            match act.asm.push(ctx.now.0, (h.origin, h.sess), &seg).ok()? {
                AssemblyEvent::Complete(body) => Some(Delivered::Body {
                    ptype,
                    origin: h.origin,
                    body,
                }),
                _ => None,
            }
        }
        MeshPayload::Leave(leave) => Some(Delivered::Leave {
            origin: h.origin,
            sess: h.sess,
            leave,
        }),
        MeshPayload::RideCtl(ctl) => Some(Delivered::RideCtl {
            origin: h.origin,
            ctl,
        }),
        MeshPayload::GroupCtl(_) => Some(Delivered::GroupCtl {
            body: pkt.payload.to_vec(),
        }),
        MeshPayload::Position(p) => Some(Delivered::Position {
            origin: h.origin,
            p,
        }),
        _ => None,
    }
}
