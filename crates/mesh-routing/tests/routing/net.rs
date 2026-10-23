//! A small deterministic discrete-event network of real [`Router`]s for property tests:
//! fixed per-link delay, exact RTT (so `owd` and the age estimate are exact), immediate
//! delivery of every `Outgoing` (queues and pacing are tested separately).

use std::collections::{BTreeMap, BTreeSet};

use mesh_routing::{
    Authenticated, BurstKey, Instant, Limits, LinkId, LinkKind, LinkRole, NodeId, Outgoing,
    RouteOutcome, Router, RouterAction, RouterConfig,
};
use mesh_wire::mesh::{MeshHeader, MeshPacket, PacketType};
use mesh_wire::record::{Nbr, NbrEntry, NbrFlags};

/// Link id at `u` toward `v`.
pub fn lid(u: NodeId, v: NodeId) -> LinkId {
    LinkId(u32::from(u) * 1_000 + u32::from(v))
}

#[derive(Debug, Clone)]
enum Ev {
    Deliver {
        to: NodeId,
        link: LinkId,
        bytes: Vec<u8>,
    },
    Poll {
        node: NodeId,
    },
}

/// A delivered packet.
#[derive(Debug, Clone)]
pub struct Rx {
    pub at: Instant,
    pub node: NodeId,
    pub header: MeshHeader,
    pub payload: Vec<u8>,
}

/// One voice transmission on a link.
#[derive(Debug, Clone, Copy)]
pub struct VoiceTx {
    pub at: Instant,
    pub node: NodeId,
    pub link: LinkId,
    pub burst: BurstKey,
}

pub struct Net {
    pub now: Instant,
    pub routers: BTreeMap<NodeId, Router>,
    pub edges: BTreeSet<(NodeId, NodeId)>,
    pub delay_us: u64,
    queue: BTreeMap<(u64, u64), Ev>,
    seq: u64,
    pub rx: Vec<Rx>,
    pub voice_tx: Vec<VoiceTx>,
    pub actions: Vec<(Instant, NodeId, RouterAction)>,
    pub transmissions: u64,
    /// Invariant violations seen (admitted set larger than the cap, voice for a burst not
    /// admitted on the link).
    pub violations: Vec<String>,
}

impl Net {
    pub fn new(n: u16, delay_us: u64, limits: Limits, seed: u64) -> Self {
        let mut routers = BTreeMap::new();
        for id in 1..=n {
            let mut rng_seed = [0u8; 32];
            rng_seed[..8].copy_from_slice(
                &seed
                    .wrapping_mul(31)
                    .wrapping_add(u64::from(id))
                    .to_le_bytes(),
            );
            routers.insert(
                id,
                Router::new(RouterConfig {
                    me: id,
                    sess: (id % 250 + 1) as u8,
                    initial_seq: id.wrapping_mul(977),
                    epoch: 7,
                    limits,
                    multi_hop: true,
                    rng_seed,
                }),
            );
        }
        Net {
            now: Instant::from_secs(1),
            routers,
            edges: BTreeSet::new(),
            delay_us,
            queue: BTreeMap::new(),
            seq: 0,
            rx: Vec::new(),
            voice_tx: Vec::new(),
            actions: Vec::new(),
            transmissions: 0,
            violations: Vec::new(),
        }
    }

    /// Connects `u` and `v` (BLE) with exact RTT samples.
    pub fn connect(&mut self, u: NodeId, v: NodeId) {
        let now = self.now;
        let rtt = 2 * self.delay_us;
        for (a, b, role) in [(u, v, LinkRole::Initiator), (v, u, LinkRole::Responder)] {
            let r = self.routers.get_mut(&a).expect("node");
            r.neighbors
                .link_up(now, b, lid(a, b), LinkKind::Ble, role, 1024)
                .expect("link up");
            r.neighbors.on_rtt(now, lid(a, b), rtt);
        }
        self.edges.insert((u.min(v), u.max(v)));
    }

    pub fn neighbors_of(&self, u: NodeId) -> Vec<NodeId> {
        self.edges
            .iter()
            .filter_map(|&(a, b)| {
                if a == u {
                    Some(b)
                } else if b == u {
                    Some(a)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Every node sends a fresh NBR listing all its neighbors as good and uncongested.
    pub fn exchange_nbr(&mut self) {
        let now = self.now;
        let ids: Vec<NodeId> = self.routers.keys().copied().collect();
        for u in ids {
            let list: Vec<NbrEntry> = self
                .neighbors_of(u)
                .into_iter()
                .take(Nbr::MAX_ENTRIES)
                .map(|n| NbrEntry {
                    node_id: n,
                    q: 95,
                    flags: NbrFlags::new(true, false, 0),
                    vcap: 2,
                })
                .collect();
            let nbr = Nbr::new(0, &list).expect("nbr");
            for v in self.neighbors_of(u) {
                if let Some(r) = self.routers.get_mut(&v) {
                    r.neighbors.on_nbr(now, lid(v, u), &nbr);
                }
            }
        }
    }

    fn push(&mut self, at: Instant, ev: Ev) {
        self.seq += 1;
        self.queue.insert((at.us(), self.seq), ev);
    }

    fn reverse(link: LinkId) -> (NodeId, LinkId) {
        let u = (link.0 / 1_000) as NodeId;
        let v = (link.0 % 1_000) as NodeId;
        (v, lid(v, u))
    }

    /// Schedules the outcome of a router call made by `node` at `self.now`.
    pub fn apply(&mut self, node: NodeId, o: RouteOutcome) {
        let now = self.now;
        for a in &o.actions {
            self.actions.push((now, node, *a));
        }
        self.check_invariants(node, &o.out);
        self.send(node, o.out);
        if let Some(t) = self.routers.get(&node).and_then(|r| r.next_deadline()) {
            self.push(t.max(now), Ev::Poll { node });
        }
    }

    fn send(&mut self, node: NodeId, out: Vec<Outgoing>) {
        let now = self.now;
        for o in out {
            self.transmissions += 1;
            if let Some(b) = o.packet.burst
                && o.packet.header.ptype == PacketType::Voice
            {
                self.voice_tx.push(VoiceTx {
                    at: now,
                    node,
                    link: o.link,
                    burst: b,
                });
            }
            let bytes = o.packet.encode_at(now);
            let (to, link) = Self::reverse(o.link);
            self.push(now.plus_us(self.delay_us), Ev::Deliver { to, link, bytes });
        }
    }

    fn check_invariants(&mut self, node: NodeId, out: &[Outgoing]) {
        let Some(r) = self.routers.get(&node) else {
            return;
        };
        for l in r.neighbors.link_ids() {
            let n = r.admission.admitted(l).len();
            if n > usize::from(r.link_cap(l)) {
                self.violations.push(format!(
                    "node {node} link {l:?}: {n} admitted > cap {}",
                    r.link_cap(l)
                ));
            }
        }
        for o in out {
            if o.packet.header.ptype == PacketType::Voice
                && let Some(b) = o.packet.burst
            {
                // A burst rejected later in the same call (tombstoned) legitimately
                // left the admitted sets after this frame was sent.
                let live = r.floor.get(b).is_some_and(|e| e.live());
                if live && !r.admission.admitted(o.link).contains(&b) {
                    self.violations.push(format!(
                        "node {node}: voice of {b:?} on {:?} not admitted",
                        o.link
                    ));
                }
            }
        }
    }

    /// Originates from `node` now.
    pub fn originate(&mut self, node: NodeId, ptype: PacketType, leader: bool, payload: Vec<u8>) {
        let now = self.now;
        let o = self
            .routers
            .get_mut(&node)
            .expect("node")
            .originate(now, ptype, leader, payload);
        self.apply(node, o);
    }

    /// Runs events up to `until` (inclusive), calling `tick` on every router every 50 ms.
    pub fn run_until(&mut self, until: Instant) {
        loop {
            let next = self.queue.keys().next().copied();
            let next_tick = Instant(self.now.us() - self.now.us() % 50_000 + 50_000);
            match next {
                Some((t, s)) if t <= until.us() && t < next_tick.us() => {
                    let ev = self.queue.remove(&(t, s)).expect("ev");
                    self.now = Instant(t);
                    self.handle(ev);
                }
                _ if next_tick <= until => {
                    self.now = next_tick;
                    let ids: Vec<NodeId> = self.routers.keys().copied().collect();
                    for id in ids {
                        let now = self.now;
                        let acts = self.routers.get_mut(&id).expect("r").tick(now);
                        for a in acts {
                            self.actions.push((now, id, a));
                        }
                    }
                }
                _ => {
                    self.now = until;
                    return;
                }
            }
        }
    }

    fn handle(&mut self, ev: Ev) {
        let now = self.now;
        match ev {
            Ev::Deliver { to, link, bytes } => {
                let Some(r) = self.routers.get_mut(&to) else {
                    return;
                };
                let o = r.on_mesh_packet(now, link, Authenticated::after_verified_frame(&bytes));
                if let Some(h) = o.deliver {
                    let payload = MeshPacket::decode(&bytes)
                        .map(|p| p.payload.to_vec())
                        .unwrap_or_default();
                    self.rx.push(Rx {
                        at: now,
                        node: to,
                        header: h,
                        payload,
                    });
                }
                self.apply(to, o);
            }
            Ev::Poll { node } => {
                let out = self.routers.get_mut(&node).expect("r").poll(now);
                self.check_invariants(node, &out);
                self.send(node, out);
                if let Some(t) = self.routers.get(&node).and_then(|r| r.next_deadline()) {
                    self.push(t.max(now.plus_us(1)), Ev::Poll { node });
                }
            }
        }
    }

    /// Hop distances from `src` over the edges (BFS).
    pub fn distances(&self, src: NodeId) -> BTreeMap<NodeId, u32> {
        let mut d = BTreeMap::from([(src, 0u32)]);
        let mut frontier = vec![src];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for u in frontier {
                let du = d[&u];
                for v in self.neighbors_of(u) {
                    if let std::collections::btree_map::Entry::Vacant(e) = d.entry(v) {
                        e.insert(du + 1);
                        next.push(v);
                    }
                }
            }
            frontier = next;
        }
        d
    }
}

/// Node coordinates and the edge list of a generated graph.
pub type Graph = (Vec<(f64, f64)>, Vec<(NodeId, NodeId)>);

/// Random geometric graph: `n` nodes uniform in a unit square, edges below `radius`,
/// node degree capped at `max_degree` (closest first), as BLE links would be.
pub fn random_geometric(n: u16, radius: f64, max_degree: usize, seed: u64) -> Graph {
    let mut s = seed | 1;
    let mut rnd = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s % 1_000_000) as f64 / 1_000_000.0
    };
    let pts: Vec<(f64, f64)> = (0..n).map(|_| (rnd(), rnd())).collect();
    let mut cand: Vec<(f64, NodeId, NodeId)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let (a, b) = (pts[usize::from(i)], pts[usize::from(j)]);
            let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            if d < radius {
                cand.push((d, i + 1, j + 1));
            }
        }
    }
    cand.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut deg: BTreeMap<NodeId, usize> = BTreeMap::new();
    let mut edges = Vec::new();
    for (_, u, v) in cand {
        if deg.get(&u).copied().unwrap_or(0) < max_degree
            && deg.get(&v).copied().unwrap_or(0) < max_degree
        {
            *deg.entry(u).or_default() += 1;
            *deg.entry(v).or_default() += 1;
            edges.push((u, v));
        }
    }
    (pts, edges)
}
