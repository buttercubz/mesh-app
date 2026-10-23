//! In-memory world of real `Engine`s in group mode for the mesh tests.
//!
//! The secure and group modules are replaced by a test module ([`Glue`]): group links
//! carry plaintext LAB-format frames (`TestSec`) and are confirmed Up right after the
//! empty handshake with the peer's real per-group `node_id`; the stored group blob of
//! each node (a leader-created group and members joined from its invite) is installed
//! after the group module loaded, so only the mesh glue reads it; the test glue hands
//! the identity to the media module, as the group module does. A node can inject MESH
//! packets as their origin: from its timer path, like the media outbox, they take the
//! next `seq` of the node's shared per-origin counter and go to the mesh glue
//! (`Ctx::originate`).
//!
//! Links have a one-way delay and a capacity: a unit occupies its direction for
//! `len × 8 / rate` behind the units before it; when the platform buffer holds more
//! than `os_buffer_us` of transmit time the write is refused (WouldBlock) and Writable
//! follows once it drains.
//! The clock advances in 0.5 ms steps; every node runs its timers when due and at a
//! 5 ms engine tick (DESIGN §3.4).

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mesh_api::Vehicle;
use mesh_core::effects::Effect;
use mesh_core::group::state::Group;
use mesh_core::group::store;
use mesh_core::links::{LinkSecurity, Plain, Prio, SecurityError, SecurityStep};
use mesh_core::media::{MediaIdentity, MediaRequest};
use mesh_core::mesh::MeshIdentity;
use mesh_core::{
    BLOCK_SAMPLES, CoreModule, Ctx, Engine, EngineConfig, EngineMode, Instant, LinkId, LinkInfo,
    LinkKind, LinkRole, NodeId, OriginPacket, Platform, PlatformEvent, SessionStart, StoredGroup,
};
use mesh_wire::frame::{LinkFrame, lab_tag};
use mesh_wire::mesh::MeshHeader;
use mesh_wire::record::Record;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

pub const STEP_US: u64 = 500;
pub const TICK_US: u64 = 5_000;

/// Group-link security for tests: plaintext records in LAB-format frames.
pub struct TestSec(Plain);

impl LinkSecurity for TestSec {
    fn on_start(&mut self, now: Instant, out: &mut Vec<Vec<u8>>) -> SecurityStep {
        self.0.on_start(now, out)
    }
    fn on_hs(
        &mut self,
        now: Instant,
        step: u8,
        v: u8,
        msg: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) -> SecurityStep {
        self.0.on_hs(now, step, v, msg, out)
    }
    fn seal(&mut self, now: Instant, records: &[u8]) -> Result<Vec<u8>, SecurityError> {
        self.0.seal(now, records)
    }
    fn open(&mut self, now: Instant, frame: &LinkFrame<'_>) -> Result<Vec<u8>, SecurityError> {
        self.0.open(now, frame)
    }
    fn overhead(&self) -> usize {
        self.0.overhead()
    }
}

/// A packet a test injects at its origin: `(priority, MESH body, deadline)`.
pub type Injected = (Prio, Vec<u8>, Option<Instant>);

/// Shared state between the world and a node's glue.
#[derive(Default)]
pub struct GlueState {
    /// Peer `node_id` per local link id.
    pub peers: BTreeMap<u32, u16>,
    pub inject: Vec<Injected>,
    /// The `ctx.sess` of the running session (read by tests that build packets).
    pub sess: u8,
    /// The first `seq` of the node's shared counter at session start (wrap tests).
    pub first_seq: Option<u16>,
    /// The node never learns its group identity: the mesh glue stays inactive.
    pub no_identity: bool,
    /// Requests seen on the inter-module bus (session → media seam).
    pub bus: Vec<MediaRequest>,
}

/// Records the inter-module bus (registered first, so it sees every request before the
/// media module drains it). Passive.
pub struct BusTap {
    pub shared: Shared,
}

impl CoreModule for BusTap {
    fn name(&self) -> &'static str {
        "bus-tap"
    }
    fn on_bus(&mut self, ctx: &mut Ctx) {
        self.shared
            .borrow_mut()
            .bus
            .extend(ctx.media_bus.iter().cloned());
    }
}

pub type Shared = Rc<RefCell<GlueState>>;

/// Stands in for the secure and group modules.
pub struct Glue {
    pub shared: Shared,
    pub group_id: [u8; 16],
    pub blob: Vec<u8>,
    /// The group identity the media module gets at session start.
    pub media: Option<MediaIdentity>,
}

impl CoreModule for Glue {
    fn name(&self) -> &'static str {
        "test-glue"
    }
    fn on_mode(&mut self, ctx: &mut Ctx, mode: EngineMode) {
        if matches!(mode, EngineMode::Session { .. }) {
            if let Some(s) = self.shared.borrow().first_seq {
                ctx.mesh_seq = s;
            }
            self.shared.borrow_mut().sess = ctx.sess;
            if self.shared.borrow().no_identity {
                return;
            }
            ctx.cfg
                .stored_groups
                .retain(|g| g.group_id != self.group_id);
            ctx.cfg.stored_groups.push(StoredGroup {
                group_id: self.group_id,
                blob: self.blob.clone(),
            });
            // The session → media seam, as the group module does it.
            if let Some(id) = self.media {
                ctx.post_media(MediaRequest::Identity(id));
            }
        }
    }
    fn security_for(
        &mut self,
        _: &mut Ctx,
        _: LinkId,
        _: &LinkInfo,
    ) -> Option<Box<dyn LinkSecurity>> {
        Some(Box::new(TestSec(Plain::new(lab_tag(1)))))
    }
    fn on_link_established(&mut self, ctx: &mut Ctx, link: LinkId) {
        let peer = self.shared.borrow().peers.get(&link.0).copied();
        if let Some(p) = peer {
            let now = ctx.now;
            ctx.links.confirm_up(now, link, NodeId(p));
        }
    }
    fn on_timer(&mut self, ctx: &mut Ctx) {
        let pkts: Vec<Injected> = std::mem::take(&mut self.shared.borrow_mut().inject);
        for (prio, mut body, deadline) in pkts {
            // One counter per (origin, sess) for every class (§5.3).
            if let Ok(mut h) = MeshHeader::decode(&body) {
                h.seq = ctx.next_mesh_seq();
                if let Ok(hb) = h.encode() {
                    body[..MeshHeader::LEN].copy_from_slice(&hb);
                }
            }
            ctx.originate(OriginPacket {
                prio,
                body,
                deadline,
            });
        }
    }
    fn next_timer(&self) -> Option<Instant> {
        (!self.shared.borrow().inject.is_empty()).then_some(Instant::ZERO)
    }
}

/// One node.
pub struct Node {
    pub engine: Engine,
    pub shared: Shared,
    pub node_id: u16,
    pub effects: Vec<(u64, Effect)>,
    /// Units written: `(t, link, bytes)`.
    pub tx: Vec<(u64, LinkId, Vec<u8>)>,
    /// Links refused with WouldBlock and when their direction frees.
    pub blocked: BTreeMap<u32, u64>,
    /// WouldBlock answers given.
    pub would_blocks: u64,
    /// Capture tone (440 Hz) between two instants, for real media tests.
    pub tone: Option<(u64, u64)>,
    /// Peak amplitude of every playout block: `(t, peak)`.
    pub heard: Vec<(u64, i32)>,
}

/// One link between two nodes.
#[derive(Debug, Clone, Copy)]
pub struct SimLink {
    pub a: (usize, LinkId),
    pub b: (usize, LinkId),
    pub up: bool,
    pub delay_us: u64,
    pub rate_bps: u64,
    pub kind: LinkKind,
    /// Busy-until per direction (a → b, b → a).
    pub busy: [u64; 2],
}

pub struct World {
    pub now: u64,
    pub nodes: Vec<Node>,
    pub links: Vec<SimLink>,
    pub group_id: [u8; 16],
    in_flight: BTreeMap<(u64, u64), (usize, LinkId, Vec<u8>)>,
    seq: u64,
    next_link: u32,
    pub delay_us: u64,
    pub rate_bps: u64,
    pub kind: LinkKind,
    /// Platform send buffer, as transmit time: a write is refused (WouldBlock) only when
    /// the direction is busy for longer than this.
    pub os_buffer_us: u64,
    /// Nodes whose NBR records are removed in flight (missing-NBR scenarios).
    pub strip_nbr: Vec<usize>,
}

pub fn seed(i: usize, salt: u8) -> [u8; 32] {
    let mut s = [salt; 32];
    s[0] = i as u8;
    s[1] = 0x3C;
    s
}

impl World {
    /// `n` nodes: node 0 creates the group (leader), the others join from its invite.
    pub fn new(n: usize, salt: u8) -> World {
        Self::with_platform(n, salt, Platform::Ios)
    }

    pub fn with_platform(n: usize, salt: u8, platform: Platform) -> World {
        Self::build(n, salt, platform, false)
    }

    /// `aware_pairing`: the group's Wi-Fi Aware pairing is set up (Phase 4).
    pub fn build(n: usize, salt: u8, platform: Platform, aware_pairing: bool) -> World {
        Self::configured(n, salt, platform, aware_pairing, |_, _| {})
    }

    /// Like [`World::build`], with `tweak(node, cfg)` applied to each engine config.
    pub fn configured(
        n: usize,
        salt: u8,
        platform: Platform,
        aware_pairing: bool,
        tweak: impl Fn(usize, &mut EngineConfig),
    ) -> World {
        let mut rng = ChaCha20Rng::from_seed(seed(99, salt));
        let mut leader = Group::create("Ride", Vehicle::Moto, "N0", 0, &mut rng);
        leader.aware_pairing = aware_pairing;
        let invite = leader.invite().expect("leader invite");
        let group_id = leader.id;
        let mut groups = vec![leader];
        for i in 1..n {
            groups.push(Group::join(
                &invite,
                &format!("N{i}"),
                Vehicle::Moto,
                &mut rng,
            ));
        }
        let nodes = groups
            .into_iter()
            .enumerate()
            .map(|(i, g)| {
                let mut cfg = EngineConfig::new(platform, seed(i, salt));
                // The group module sees an unreadable blob and stays out of the way.
                cfg.stored_groups = vec![StoredGroup {
                    group_id,
                    blob: Vec::new(),
                }];
                tweak(i, &mut cfg);
                let mut engine = Engine::new(cfg, Instant::ZERO);
                let shared = Shared::default();
                let blob = store::encode(&g);
                let media = MeshIdentity::from_blob(group_id, &blob).map(|m| m.media());
                engine.register_module(Box::new(Glue {
                    shared: shared.clone(),
                    group_id,
                    blob,
                    media,
                }));
                engine.register_module_first(Box::new(BusTap {
                    shared: shared.clone(),
                }));
                Node {
                    engine,
                    shared,
                    node_id: g.my_node_id(),
                    effects: Vec::new(),
                    tx: Vec::new(),
                    blocked: BTreeMap::new(),
                    would_blocks: 0,
                    tone: None,
                    heard: Vec::new(),
                }
            })
            .collect();
        World {
            now: 0,
            nodes,
            links: Vec::new(),
            group_id,
            in_flight: BTreeMap::new(),
            seq: 0,
            next_link: 1,
            delay_us: 10_000,
            rate_bps: 1_000_000,
            kind: LinkKind::BleL2cap,
            strip_nbr: Vec::new(),
            os_buffer_us: 20_000,
        }
    }

    pub fn start_all(&mut self) {
        for i in 0..self.nodes.len() {
            let now = Instant(self.now);
            let gid = self.group_id;
            self.nodes[i]
                .engine
                .start(now, SessionStart::Group { group_id: gid })
                .expect("group start");
            self.drain(i);
        }
    }

    /// Opens a link `a` (initiator) ↔ `b`; returns its index.
    pub fn connect(&mut self, a: usize, b: usize) -> usize {
        let la = LinkId(self.next_link);
        let lb = LinkId(self.next_link + 1);
        self.next_link += 2;
        let (ida, idb) = (self.nodes[a].node_id, self.nodes[b].node_id);
        self.nodes[a].shared.borrow_mut().peers.insert(la.0, idb);
        self.nodes[b].shared.borrow_mut().peers.insert(lb.0, ida);
        self.links.push(SimLink {
            a: (a, la),
            b: (b, lb),
            up: true,
            delay_us: self.delay_us,
            rate_bps: self.rate_bps,
            kind: self.kind,
            busy: [0, 0],
        });
        let now = Instant(self.now);
        for (n, l, role) in [(a, la, LinkRole::Initiator), (b, lb, LinkRole::Responder)] {
            let mut info = LinkInfo::new(self.kind, role);
            if role == LinkRole::Initiator {
                info.disc_id = Some(5_000 + b as u32);
            }
            self.nodes[n]
                .engine
                .handle_event(now, PlatformEvent::Up { link: l, info });
            self.drain(n);
        }
        self.links.len() - 1
    }

    /// Takes link `idx` down on both sides.
    pub fn drop_link(&mut self, idx: usize, reason: mesh_core::LinkDownReason) {
        let l = self.links[idx];
        if !l.up {
            return;
        }
        self.links[idx].up = false;
        let now = Instant(self.now);
        for (n, id) in [l.a, l.b] {
            self.nodes[n]
                .engine
                .handle_event(now, PlatformEvent::Down { link: id, reason });
            self.drain(n);
        }
    }

    fn route(&self, node: usize, link: LinkId) -> Option<(usize, usize, LinkId)> {
        self.links.iter().enumerate().find_map(|(i, l)| {
            if !l.up {
                None
            } else if l.a == (node, link) {
                Some((i, l.b.0, l.b.1))
            } else if l.b == (node, link) {
                Some((i, l.a.0, l.a.1))
            } else {
                None
            }
        })
    }

    /// Drains transmits (respecting link capacity) and effects of node `i`.
    pub fn drain(&mut self, i: usize) {
        let now = self.now;
        let freed: Vec<u32> = self.nodes[i]
            .blocked
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(l, _)| *l)
            .collect();
        for l in freed {
            self.nodes[i].blocked.remove(&l);
            self.nodes[i]
                .engine
                .handle_event(Instant(now), PlatformEvent::Writable { link: LinkId(l) });
        }
        while let Some(t) = self.nodes[i].engine.poll_transmit(Instant(now)) {
            let Some((idx, peer, plink)) = self.route(i, t.link) else {
                continue;
            };
            let dir = usize::from(self.links[idx].a.0 != i);
            let free = self.links[idx].busy[dir];
            if free > now + self.os_buffer_us {
                self.nodes[i]
                    .blocked
                    .insert(t.link.0, free - self.os_buffer_us);
                self.nodes[i].would_blocks += 1;
                self.nodes[i].engine.handle_event(
                    Instant(now),
                    PlatformEvent::WouldBlock {
                        link: t.link,
                        unit: &t.bytes,
                    },
                );
                continue;
            }
            let air = (t.bytes.len() as u64 * 8 * 1_000_000).div_ceil(self.links[idx].rate_bps);
            let start = free.max(now);
            self.links[idx].busy[dir] = start + air;
            let at = start + air + self.links[idx].delay_us;
            self.seq += 1;
            let wire = if self.strip_nbr.contains(&i) {
                strip_nbr(&t.bytes)
            } else {
                t.bytes.clone()
            };
            self.in_flight.insert((at, self.seq), (peer, plink, wire));
            self.nodes[i].tx.push((now, t.link, t.bytes));
        }
        while let Some(e) = self.nodes[i].engine.poll_effect() {
            self.nodes[i].effects.push((now, e));
        }
    }

    pub fn step_once(&mut self) {
        let now = self.now;
        while let Some((&(at, s), _)) = self.in_flight.iter().next() {
            if at > now {
                break;
            }
            let (n, l, bytes) = self.in_flight.remove(&(at, s)).expect("in flight");
            let up = self
                .links
                .iter()
                .any(|x| x.up && (x.a == (n, l) || x.b == (n, l)));
            if up {
                self.nodes[n].engine.handle_packet(Instant(now), l, &bytes);
                self.drain(n);
            }
        }
        for i in 0..self.nodes.len() {
            let tick = now.is_multiple_of(TICK_US);
            for _ in 0..8 {
                let due = self.nodes[i]
                    .engine
                    .poll_timeout()
                    .is_some_and(|t| t.0 <= now);
                let blocked_due = self.nodes[i].blocked.values().any(|t| *t <= now);
                if !(due || blocked_due || tick) {
                    break;
                }
                self.nodes[i].engine.handle_timeout(Instant(now));
                self.drain(i);
                if !due {
                    break;
                }
            }
            if now.is_multiple_of(10_000) {
                let block = self.capture_block(i, now);
                let mut out = [0i16; BLOCK_SAMPLES];
                self.nodes[i].engine.push_capture(Instant(now), &block);
                self.nodes[i].engine.pull_playout(Instant(now), &mut out);
                let peak = out.iter().map(|s| i32::from(*s).abs()).max().unwrap_or(0);
                self.nodes[i].heard.push((now, peak));
                self.drain(i);
            }
        }
        self.now += STEP_US;
    }

    fn capture_block(&self, i: usize, now: u64) -> [i16; BLOCK_SAMPLES] {
        let mut b = [0i16; BLOCK_SAMPLES];
        if let Some((from, until)) = self.nodes[i].tone
            && now >= from
            && now < until
        {
            for (k, s) in b.iter_mut().enumerate() {
                let t = (now as f64 + k as f64 * 62.5) / 1e6;
                *s = (8_000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i16;
            }
        }
        b
    }

    pub fn run_ms(&mut self, ms: u64) {
        let end = self.now + ms * 1_000;
        while self.now < end {
            self.step_once();
        }
    }

    /// Runs `f` on node `i`'s engine at the current time and drains it.
    pub fn with<T>(&mut self, i: usize, f: impl FnOnce(&mut Engine, Instant) -> T) -> T {
        let now = Instant(self.now);
        let r = f(&mut self.nodes[i].engine, now);
        self.drain(i);
        r
    }

    /// Queues MESH packets on node `i` as their origin (sent from its next timer call).
    pub fn inject(&mut self, i: usize, p: Injected) {
        self.nodes[i].shared.borrow_mut().inject.push(p);
    }

    /// Injects one POSITION (bulk class, P2) at node `i` as its origin.
    pub fn position(&mut self, i: usize) {
        use mesh_wire::mesh::{MeshPayload, PacketType, Position, encode_packet};
        let h = MeshHeader::new(PacketType::Position, self.nodes[i].node_id, self.sess(i), 0);
        let p = Position {
            lat_e7: 400_000_000,
            lon_e7: -30_000_000,
            speed_dms: 120,
            heading_2deg: 45,
            acc_m: 5,
            fix_age_100ms: 1,
            rsv: 0,
        };
        let body = encode_packet(&h, &MeshPayload::Position(p)).expect("position");
        self.inject(i, (Prio::P2, body, None));
    }

    /// Packets node `i` originated itself (hops 0) and wrote on `link`: `(ptype, seq)`.
    pub fn own_on(&self, i: usize, link: LinkId) -> Vec<(mesh_wire::mesh::PacketType, u16)> {
        let me = self.nodes[i].node_id;
        self.sent(i)
            .into_iter()
            .filter(|(_, l, h, _)| *l == link && h.origin == me && h.hops == 0)
            .map(|(_, _, h, _)| (h.ptype, h.seq))
            .collect()
    }

    /// Requests node `i`'s media module got on the inter-module bus.
    pub fn bus(&self, i: usize) -> Vec<MediaRequest> {
        self.nodes[i].shared.borrow().bus.clone()
    }

    /// The `sess` of node `i`'s running session.
    pub fn sess(&self, i: usize) -> u8 {
        self.nodes[i].shared.borrow().sess
    }

    /// Every MESH packet node `i` wrote: `(t, link, header, body)`.
    pub fn sent(&self, i: usize) -> Vec<(u64, LinkId, mesh_wire::mesh::MeshHeader, Vec<u8>)> {
        let mut v = Vec::new();
        for (t, l, unit) in &self.nodes[i].tx {
            let Some(frame) = self.frame_of(i, *l, unit) else {
                continue;
            };
            let Ok(LinkFrame::Lab { records, .. }) = LinkFrame::decode(frame) else {
                continue;
            };
            for rec in mesh_wire::record::RecordIter::new(records).flatten() {
                if let Record::Mesh(b) = rec
                    && let Ok(p) = mesh_wire::mesh::MeshPacket::decode(b)
                {
                    v.push((*t, *l, p.header, b.to_vec()));
                }
            }
        }
        v
    }

    /// Every record type node `i` wrote on `link`.
    pub fn sent_records(&self, i: usize, link: LinkId) -> Vec<(u64, u8)> {
        let mut v = Vec::new();
        for (t, l, unit) in &self.nodes[i].tx {
            if *l != link {
                continue;
            }
            let Some(frame) = self.frame_of(i, *l, unit) else {
                continue;
            };
            let Ok(LinkFrame::Lab { records, .. }) = LinkFrame::decode(frame) else {
                continue;
            };
            for rec in mesh_wire::record::RecordIter::new(records).flatten() {
                v.push((*t, rec.rtype()));
            }
        }
        v
    }

    /// The LinkFrame inside a unit node `i` wrote on `link` (L2CAP: after `len u16`;
    /// datagram: the unit itself).
    pub fn frame_of<'a>(&self, i: usize, link: LinkId, unit: &'a [u8]) -> Option<&'a [u8]> {
        let kind = self
            .links
            .iter()
            .find(|x| x.a == (i, link) || x.b == (i, link))
            .map_or(LinkKind::BleL2cap, |x| x.kind);
        match kind {
            LinkKind::WifiAware => Some(unit),
            _ => unit.get(2..),
        }
    }

    /// Local link id of node `i` on world link `idx`.
    pub fn local(&self, idx: usize, i: usize) -> LinkId {
        let l = self.links[idx];
        if l.a.0 == i { l.a.1 } else { l.b.1 }
    }

    /// Radio commands of node `i`.
    pub fn radio(&self, i: usize) -> Vec<mesh_core::effects::RadioCommand> {
        self.nodes[i]
            .effects
            .iter()
            .filter_map(|(_, e)| match e {
                Effect::Radio(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Removes the NBR records of an L2CAP unit carrying a LAB-format frame.
fn strip_nbr(unit: &[u8]) -> Vec<u8> {
    let Some(frame) = unit.get(2..) else {
        return unit.to_vec();
    };
    let Ok(LinkFrame::Lab { lab_tag, records }) = LinkFrame::decode(frame) else {
        return unit.to_vec();
    };
    let mut w = mesh_wire::record::RecordWriter::new();
    for rec in mesh_wire::record::RecordIter::new(records).flatten() {
        if !matches!(rec, Record::Nbr(_)) {
            let _ = w.push(&rec);
        }
    }
    let body = w.into_bytes();
    let f = LinkFrame::Lab {
        lab_tag,
        records: &body,
    }
    .encode()
    .expect("lab frame");
    mesh_wire::link::encode_stream(&f).expect("stream unit")
}
