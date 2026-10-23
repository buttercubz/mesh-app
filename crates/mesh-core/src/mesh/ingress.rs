//! Inbound MESH records (DESIGN §5.2): only records of Up group links reach here, i.e.
//! from SEC frames whose AEAD tag verified (§4.1, §5.3). My own packets echoed back by
//! relays are dropped first; the per-neighbor ingress caps (300 records/s, 64 KB/s) keep
//! an abusive neighbor from consuming relay capacity; the router then validates,
//! deduplicates (one window per `(origin, sess)`: every module of an origin draws its
//! `seq` from one shared counter, §5.3), updates its floor table and decides the
//! fan-out.

use mesh_routing::Authenticated;
use mesh_wire::mesh::{MeshHeader, PacketType, Segment};

use super::rl;
use super::state::Mesh;
use crate::LinkId;
use crate::engine::Ctx;

impl Mesh {
    /// One MESH record body received on group link `link`.
    pub fn on_mesh(&mut self, ctx: &mut Ctx, link: LinkId, bytes: &[u8]) {
        let Some(meta) = self.links.get(&link) else {
            return;
        };
        let peer = meta.peer;
        let now = ctx.now.routing();
        let header = MeshHeader::decode(bytes).ok();
        if let Some(h) = header
            && h.origin == self.id.node
        {
            // My own packet (or one of my previous engine session) relayed back.
            self.stats.own_echo += 1;
            return;
        }
        let within_caps = self.router.neighbors.on_rx(
            now,
            rl(link),
            1,
            bytes.len().min(u32::MAX as usize) as u32,
        );
        if !within_caps {
            self.stats.over_ingress_cap += 1;
            return;
        }
        self.stats.ingress += 1;
        let out =
            self.router
                .on_mesh_packet(now, rl(link), Authenticated::after_verified_frame(bytes));
        if let Some(h) = out.deliver {
            if h.ptype == PacketType::Presence {
                self.heard.insert(h.origin, ctx.now);
            }
            if h.seg
                && let Some(payload) = bytes.get(MeshHeader::LEN..)
                && let Ok(s) = Segment::decode(payload)
            {
                self.seg
                    .observe(now, peer, (h.origin, h.sess, s.msg_id), s.idx, s.count);
            }
        }
        self.apply(ctx, out);
        self.pump(ctx);
    }

    /// Reassemblies that timed out disable pruning for their remaining segments from
    /// that neighbor (§5.4 condition 2).
    pub fn seg_timer(&mut self, ctx: &Ctx) {
        let now = ctx.now.routing();
        for (from, origin, sess, msg_id) in self.seg.expire(now) {
            self.router
                .seg_timeouts
                .note(now, from, origin, sess, msg_id);
        }
    }
}
