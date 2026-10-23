//! The receive tap: a `CoreModule` registered **first** in every simulated engine. It
//! sees every record of an Up group link after the link layer verified and decrypted it
//! (SEC AEAD), before the media and mesh modules, and never consumes one. It records the
//! mesh-level ground truth the scenarios assert on (DESIGN §13.2):
//!
//! - every VOICE copy that arrives: first arrival per frame `(origin, sess, burst, ts)`,
//!   hop count, copies (duplicates the dedup must drop);
//! - per incoming link, the span of every burst it carried (the "no link exceeds its
//!   cap" assertion and the per-link stream counts);
//! - TALK / FLOOR_REJECT / PRESENCE / control arrivals per origin and session.
//!
//! Capture-side truth (what the talker said and when) comes from the scripted mic of the
//! simulated platform, playout truth from the 10 ms render blocks the world pulls.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mesh_core::{CoreModule, Ctx, LinkId};
use mesh_wire::mesh::{MeshPacket, MeshPayload, PacketType, TalkState};
use mesh_wire::record::Record;

/// A burst whose frames pause longer than this on a link stopped being forwarded there
/// (three 40 ms frame periods).
pub const SPAN_GAP_US: u64 = 120_000;

/// A voice frame identity: `(origin, sess, burst, ts)` packed into a `u64`.
pub fn frame_key(origin: u16, sess: u8, burst: u8, ts: u16) -> u64 {
    (u64::from(origin) << 32) | (u64::from(sess) << 24) | (u64::from(burst) << 16) | u64::from(ts)
}

/// Unpacks a [`frame_key`].
pub fn unpack_frame(k: u64) -> (u16, u8, u8, u16) {
    ((k >> 32) as u16, (k >> 24) as u8, (k >> 16) as u8, k as u16)
}

/// A burst identity `(origin, sess, burst)` packed into a `u32`.
pub fn burst_key(origin: u16, sess: u8, burst: u8) -> u32 {
    (u32::from(origin) << 16) | (u32::from(sess) << 8) | u32::from(burst)
}

/// First reception of one voice frame at one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRx {
    /// First arrival (µs).
    pub t: u64,
    /// Link traversals of the first copy (`hops + 1`).
    pub traversals: u8,
    /// Copies received (≥ 1).
    pub copies: u16,
    /// Frame duration in 10 ms ticks.
    pub ticks: u8,
    /// MeshPacket `age` of the first copy at arrival (µs, 2 ms units).
    pub age_us: u32,
}

/// One TALK / FLOOR_REJECT arrival.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TalkRx {
    pub t: u64,
    pub origin: u16,
    pub sess: u8,
    pub burst: u8,
    pub state: Option<TalkState>,
    /// A FLOOR_REJECT aimed at `origin` (instead of a TALK).
    pub reject: bool,
    /// FLOOR_REJECT: the rejecting node and the reason.
    pub at: u16,
    pub reason: Option<mesh_wire::mesh::FloorRejectReason>,
}

/// What the tap of one node recorded.
#[derive(Debug, Clone, Default)]
pub struct TapState {
    /// First reception of every voice frame (cleared by the scenario when it aggregates).
    pub frames: BTreeMap<u64, FrameRx>,
    /// Burst spans per incoming link: `(link, burst_key) → [(first, last)]` arrivals;
    /// a pause longer than [`SPAN_GAP_US`] (an eviction and re-admission) starts a new
    /// span.
    pub spans: BTreeMap<(u32, u32), Vec<(u64, u64)>>,
    /// TALK and FLOOR_REJECT arrivals (every copy).
    pub talk: Vec<TalkRx>,
    /// PRESENCE per `(origin, sess)`: first and last arrival.
    pub presence: BTreeMap<(u16, u8), (u64, u64)>,
    /// Arrivals per packet type (every copy), indexed by the 5-bit ptype.
    pub ptype_count: [u64; 32],
    /// MESH records that did not decode (malformed, from a peer).
    pub undecodable: u64,
    /// Records of any type seen on group links.
    pub records: u64,
    /// Keep per-frame receptions (long scenarios may switch it off).
    pub keep_frames: bool,
    /// PRESENCE copies received and the distinct PRESENCE packets `(origin, sess, seq)`
    /// (control-flood cost, S3/S11).
    pub presence_copies: u64,
    pub presence_keys: std::collections::BTreeSet<(u16, u8, u16)>,
    /// Changes of the neighbor's NBR view per incoming link:
    /// `(t, link, [(node, q, congested, vcap)])`.
    pub nbr: Vec<NbrChange>,
}

/// One NBR view change: `(t, link, [(node, q, congested, vcap)])`.
pub type NbrChange = (u64, u32, Vec<(u16, u8, bool, u8)>);

/// Shared handle on a node's tap.
pub type TapHandle = Rc<RefCell<TapState>>;

/// A fresh tap handle that keeps frames.
pub fn new_tap() -> TapHandle {
    Rc::new(RefCell::new(TapState {
        keep_frames: true,
        ..TapState::default()
    }))
}

/// The tap module.
pub struct Tap {
    pub state: TapHandle,
}

impl TapState {
    fn on_mesh(&mut self, now: u64, link: LinkId, body: &[u8]) {
        let Ok(pkt) = MeshPacket::decode(body) else {
            self.undecodable += 1;
            return;
        };
        let h = pkt.header;
        let bits = match h.ptype {
            PacketType::Unknown(v) => v,
            p => p.bits(),
        };
        self.ptype_count[usize::from(bits & 0x1F)] += 1;
        let Ok(payload) = MeshPayload::decode(&pkt) else {
            self.undecodable += 1;
            return;
        };
        match payload {
            MeshPayload::Voice(v) => {
                let bk = burst_key(h.origin, h.sess, v.burst);
                let segs = self.spans.entry((link.0, bk)).or_default();
                match segs.last_mut() {
                    Some(last) if now.saturating_sub(last.1) <= SPAN_GAP_US => last.1 = now,
                    _ => segs.push((now, now)),
                }
                if !self.keep_frames {
                    return;
                }
                let k = frame_key(h.origin, h.sess, v.burst, v.ts);
                let ticks = v.cfg.dur().ticks() as u8;
                self.frames
                    .entry(k)
                    .and_modify(|f| f.copies = f.copies.saturating_add(1))
                    .or_insert(FrameRx {
                        t: now,
                        traversals: h.hops.saturating_add(1),
                        copies: 1,
                        ticks,
                        age_us: h.age_us() as u32,
                    });
            }
            MeshPayload::Talk(t) => self.talk.push(TalkRx {
                t: now,
                origin: h.origin,
                sess: h.sess,
                burst: t.burst,
                state: Some(t.state),
                reject: false,
                at: 0,
                reason: None,
            }),
            MeshPayload::FloorReject(r) => self.talk.push(TalkRx {
                t: now,
                origin: r.target,
                sess: r.target_sess,
                burst: r.burst,
                state: None,
                reject: true,
                at: r.at,
                reason: Some(r.reason),
            }),
            MeshPayload::Presence(_) => {
                let e = self
                    .presence
                    .entry((h.origin, h.sess))
                    .or_insert((now, now));
                e.1 = now;
                self.presence_copies += 1;
                self.presence_keys.insert((h.origin, h.sess, h.seq));
            }
            _ => {}
        }
    }

    /// Distinct `(origin, sess)` pairs heard in PRESENCE.
    pub fn presence_of(&self, origin: u16) -> Vec<(u8, u64, u64)> {
        self.presence
            .iter()
            .filter(|((o, _), _)| *o == origin)
            .map(|((_, s), (a, b))| (*s, *a, *b))
            .collect()
    }

    /// Count of arrivals of `p`.
    pub fn count(&self, p: PacketType) -> u64 {
        self.ptype_count[usize::from(p.bits() & 0x1F)]
    }
}

impl CoreModule for Tap {
    fn name(&self) -> &'static str {
        "sim-tap"
    }

    fn on_record(&mut self, ctx: &mut Ctx, link: LinkId, rec: &Record<'_>) -> bool {
        let mut s = self.state.borrow_mut();
        s.records += 1;
        match rec {
            Record::Mesh(body) => s.on_mesh(ctx.now.0, link, body),
            Record::Nbr(n) => {
                let v: Vec<(u16, u8, bool, u8)> = n.entries
                    [..usize::from(n.count).min(n.entries.len())]
                    .iter()
                    .map(|e| (e.node_id, e.q, e.flags.congested(), e.vcap))
                    .collect();
                let last = s.nbr.iter().rev().find(|x| x.1 == link.0).map(|x| &x.2);
                if last != Some(&v) {
                    s.nbr.push((ctx.now.0, link.0, v));
                }
            }
            _ => {}
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        let k = frame_key(0xBEEF, 7, 200, 1_234);
        assert_eq!(unpack_frame(k), (0xBEEF, 7, 200, 1_234));
        assert_eq!(burst_key(1, 2, 3), 0x0001_0203);
    }
}
