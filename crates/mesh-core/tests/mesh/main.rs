//! Mesh glue (WP-10D, DESIGN §5, §6.3, §9.5-§9.7): real engines relay over in-memory
//! links with delay and capacity. The secure/group modules are replaced by a test glue
//! (`world.rs`), so these tests exercise the mesh module together with the link layer,
//! the media module and the router of mesh-routing.

mod talk;
mod world;

mod caps;
mod media_e2e;
mod nbr;
mod own;
mod relay;
mod select;
mod seq;
mod upgrade;

use mesh_core::LinkId;
use mesh_wire::mesh::{MeshHeader, PacketType};
use world::World;

/// VOICE packets node `i` wrote on its link `link` from `origin`.
pub fn voice_on(w: &World, i: usize, link: LinkId, origin: u16) -> Vec<(u64, MeshHeader)> {
    w.sent(i)
        .into_iter()
        .filter(|(_, l, h, _)| *l == link && h.ptype == PacketType::Voice && h.origin == origin)
        .map(|(t, _, h, _)| (t, h))
        .collect()
}

/// Packets of `ptype` node `i` wrote on `link`.
pub fn of_type(w: &World, i: usize, link: LinkId, ptype: PacketType) -> Vec<(u64, MeshHeader)> {
    w.sent(i)
        .into_iter()
        .filter(|(_, l, h, _)| *l == link && h.ptype == ptype)
        .map(|(t, _, h, _)| (t, h))
        .collect()
}

mod full;
#[path = "../links/harness.rs"]
mod harness;
#[path = "../secure/support.rs"]
mod support;

/// NBR records node `i` wrote: `(t, link, nbr)`.
pub fn nbrs(w: &World, i: usize) -> Vec<(u64, LinkId, mesh_wire::record::Nbr)> {
    let mut v = Vec::new();
    for (t, l, unit) in &w.nodes[i].tx {
        let Some(frame) = w.frame_of(i, *l, unit) else {
            continue;
        };
        let Ok(mesh_wire::frame::LinkFrame::Lab { records, .. }) =
            mesh_wire::frame::LinkFrame::decode(frame)
        else {
            continue;
        };
        for rec in mesh_wire::record::RecordIter::new(records).flatten() {
            if let mesh_wire::record::Record::Nbr(n) = rec {
                v.push((*t, *l, n));
            }
        }
    }
    v
}

/// Most bursts simultaneously active on one directed link of node `i`. A burst is active
/// on a link from its first VOICE frame there until its last one, end-exclusive: a
/// burst evicted within `T_STICKY` hands over at a frame boundary (its last frame may
/// leave in the same instant as the evicting burst's first), which is not two streams
/// at once.
pub fn max_concurrent_bursts(w: &World, i: usize, link: LinkId) -> usize {
    let mut spans: std::collections::BTreeMap<(u16, u8, u8), (u64, u64)> = Default::default();
    for (t, l, h, body) in w.sent(i) {
        if l != link || h.ptype != PacketType::Voice {
            continue;
        }
        let burst = body.get(MeshHeader::LEN).copied().unwrap_or(0);
        let e = spans.entry((h.origin, h.sess, burst)).or_insert((t, t));
        e.1 = t;
    }
    let mut points: Vec<(u64, i32)> = Vec::new();
    for (a, b) in spans.values() {
        points.push((*a, 1));
        points.push((*b, -1));
    }
    points.sort();
    let (mut cur, mut max) = (0i32, 0i32);
    for (_, d) in points {
        cur += d;
        max = max.max(cur);
    }
    max as usize
}
