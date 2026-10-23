//! In-memory world of real `Engine`s for the mesh-core integration tests (shared by the
//! `links`, `lab` and `ui` targets through `#[path]`): simulated stream/GATT links with
//! a fixed one-way delay, a simulated radio (advertising, connect, HELLO read, open),
//! and a discrete-event loop driven by `poll_timeout`.
//!
//! A link closed by one side (`RadioCommand::Close`) reaches the other side as
//! `Down{PeerClosed}` only after every unit written before the close was delivered (an
//! ordered channel close), so a CLOSE record or REJECT frame is always read first.
//! `tick_us` adds the runtime's periodic wakeup: every node runs `handle_timeout` at
//! each multiple of it, whether or not a deadline is due (DESIGN §3.4 engine loop).
//! Every node records the requests posted on its inter-module bus (`Node::bus`).

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mesh_api::{CommandResult, UiCommand, UiEvent};
use mesh_core::effects::{AdvPayload, DiscoveryMode, Effect, LabFile, OpenVia, RadioCommand};
use mesh_core::media::MediaRequest;
use mesh_core::{
    AdvData, CoreModule, Ctx, Discovery, Engine, EngineConfig, Instant, LinkDownReason, LinkId,
    LinkInfo, LinkKind, LinkRole, LinkUnit, Platform, PlatformEvent, SessionStart, Transport,
};

/// Media requests posted on a node's inter-module bus, in posting order.
pub type BusLog = Rc<RefCell<Vec<MediaRequest>>>;

/// Passive observer registered ahead of every module: it copies the bus before the
/// media module drains it.
struct BusTap {
    log: BusLog,
}

impl CoreModule for BusTap {
    fn name(&self) -> &'static str {
        "bus-tap"
    }

    fn on_bus(&mut self, ctx: &mut Ctx) {
        self.log.borrow_mut().extend(ctx.media_bus.iter().cloned());
    }
}

/// One node of the world.
pub struct Node {
    pub engine: Engine,
    /// Every effect, in order.
    pub effects: Vec<Effect>,
    /// Effect counts at the start of each `command` call: `(start, end)` index ranges.
    pub cmd_ranges: Vec<(usize, usize)>,
    pub hello: Option<[u8; 16]>,
    pub discovery: (DiscoveryMode, Option<AdvPayload>),
    pub platform: Platform,
    /// Transmit log `(t, link, bytes)` for determinism checks.
    pub tx_log: Vec<(u64, u32, Vec<u8>)>,
    /// Units refused once with WouldBlock (`would_block_next` > 0).
    pub would_block_next: u32,
    /// Requests posted on the inter-module bus (session → media).
    pub bus: BusLog,
}

impl Node {
    /// A node around `engine`, with the bus observer attached.
    pub fn new(mut engine: Engine, platform: Platform) -> Node {
        let bus: BusLog = Rc::new(RefCell::new(Vec::new()));
        engine.register_module_first(Box::new(BusTap { log: bus.clone() }));
        Node {
            engine,
            effects: Vec::new(),
            cmd_ranges: Vec::new(),
            hello: None,
            discovery: (DiscoveryMode::Off, None),
            platform,
            tx_log: Vec::new(),
            would_block_next: 0,
            bus,
        }
    }
}

/// One delivery to `(node, link)`: a unit, or `None` for the peer's close, which arrives
/// as `Down{PeerClosed}` after the units written before it.
type Delivery = (usize, LinkId, Option<Vec<u8>>);

/// One simulated link between two nodes.
#[derive(Debug, Clone, Copy)]
pub struct SimLink {
    pub a: (usize, LinkId),
    pub b: (usize, LinkId),
    pub up: bool,
    pub blackout: bool,
}

/// The world.
pub struct World {
    pub now: Instant,
    pub nodes: Vec<Node>,
    pub links: Vec<SimLink>,
    pub delay_us: u64,
    /// Deliveries by `(at, seq)`.
    in_flight: BTreeMap<(u64, u64), Delivery>,
    /// Periodic wakeup of every node (µs), on top of the engines' own deadlines.
    pub tick_us: Option<u64>,
    next_tick: u64,
    seq: u64,
    next_link: u32,
    /// Pending connects: `(node, disc_id) → target node`.
    pending: BTreeMap<(usize, u32), usize>,
    pub kind: LinkKind,
    pub att_mtu: u16,
    /// Pairs `(a, b)` that can hear each other's advertising.
    pub in_range: Vec<(usize, usize)>,
    /// Link capacity (bit/s): a unit occupies its link for `len × 8 / rate`; the
    /// platform answers WouldBlock while the link is busy and Writable when it frees.
    pub rate_bps: u64,
    busy: BTreeMap<(usize, u32), u64>,
    writable_at: BTreeMap<(u64, u64), (usize, LinkId)>,
}

/// `disc_id` under which node `i` is discovered.
pub fn disc_of(i: usize) -> u32 {
    1_000 + i as u32
}

pub fn seed(i: usize, salt: u8) -> [u8; 32] {
    let mut s = [salt; 32];
    s[0] = i as u8;
    s[1] = 0xA5;
    s
}

impl World {
    pub fn new(n: usize, platform: Platform, salt: u8) -> World {
        let nodes = (0..n)
            .map(|i| {
                let mut cfg = EngineConfig::new(platform, seed(i, salt));
                cfg.wall_ms = 1_792_746_723_120; // 2026-10-23T09:12:03.120Z
                cfg.device.model = format!("iPhone15,{i}");
                cfg.device.os = "iOS 27.0".into();
                cfg.device.app = "0.1.0 (12)".into();
                Node::new(Engine::new(cfg, Instant::ZERO), platform)
            })
            .collect();
        World {
            now: Instant::ZERO,
            nodes,
            links: Vec::new(),
            delay_us: 10_000,
            in_flight: BTreeMap::new(),
            tick_us: None,
            next_tick: 0,
            seq: 0,
            next_link: 1,
            pending: BTreeMap::new(),
            kind: LinkKind::BleL2cap,
            att_mtu: 185,
            in_range: Vec::new(),
            rate_bps: 1_000_000,
            busy: BTreeMap::new(),
            writable_at: BTreeMap::new(),
        }
    }

    /// Starts every node in Lab mode with `code`.
    pub fn start_lab(&mut self, code: u16) {
        for i in 0..self.nodes.len() {
            self.start_lab_one(i, code);
        }
    }

    pub fn start_lab_one(&mut self, i: usize, code: u16) {
        let now = self.now;
        self.nodes[i]
            .engine
            .start(now, SessionStart::Lab { lab_code: code })
            .expect("lab start");
        self.collect(i);
    }

    fn info(&self, role: LinkRole, disc: Option<u32>) -> LinkInfo {
        let mut info = LinkInfo::new(self.kind, role);
        if self.kind == LinkKind::BleGatt {
            info.unit = LinkUnit::Message;
            info.att_mtu = Some(self.att_mtu);
        }
        info.disc_id = disc;
        info
    }

    /// Opens a link `a` (initiator) ↔ `b` (responder).
    pub fn connect(&mut self, a: usize, b: usize) -> usize {
        self.connect_with(a, b, None)
    }

    fn connect_with(&mut self, a: usize, b: usize, disc: Option<u32>) -> usize {
        let la = LinkId(self.next_link);
        let lb = LinkId(self.next_link + 1);
        self.next_link += 2;
        self.links.push(SimLink {
            a: (a, la),
            b: (b, lb),
            up: true,
            blackout: false,
        });
        let now = self.now;
        let ia = self.info(LinkRole::Initiator, disc);
        let ib = self.info(LinkRole::Responder, None);
        self.nodes[a]
            .engine
            .handle_event(now, PlatformEvent::Up { link: la, info: ia });
        self.collect(a);
        self.nodes[b]
            .engine
            .handle_event(now, PlatformEvent::Up { link: lb, info: ib });
        self.collect(b);
        self.links.len() - 1
    }

    fn peer_of(&self, node: usize, link: LinkId) -> Option<(usize, usize, LinkId)> {
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

    /// Takes the link down on both sides with `reason` (as the platform would).
    pub fn drop_link(&mut self, idx: usize, reason: LinkDownReason) {
        let l = self.links[idx];
        if !l.up {
            return;
        }
        self.links[idx].up = false;
        let now = self.now;
        for (n, id) in [l.a, l.b] {
            self.nodes[n]
                .engine
                .handle_event(now, PlatformEvent::Down { link: id, reason });
            self.collect(n);
        }
    }

    /// Runs a command on node `i` and records the effect range it produced.
    pub fn command(&mut self, i: usize, cmd: UiCommand) -> CommandResult {
        let now = self.now;
        let start = self.nodes[i].effects.len();
        let r = self.nodes[i].engine.handle_command(now, cmd);
        self.collect(i);
        let end = self.nodes[i].effects.len();
        self.nodes[i].cmd_ranges.push((start, end));
        r
    }

    /// Delivers a platform event to node `i`.
    pub fn event(&mut self, i: usize, ev: PlatformEvent<'_>) {
        let now = self.now;
        self.nodes[i].engine.handle_event(now, ev);
        self.collect(i);
    }

    /// Drains transmits and effects of node `i`, applying radio commands.
    pub fn collect(&mut self, i: usize) {
        loop {
            let mut progressed = false;
            let now = self.now;
            while let Some(t) = self.nodes[i].engine.poll_transmit(now) {
                progressed = true;
                if self.nodes[i].would_block_next > 0 {
                    self.nodes[i].would_block_next -= 1;
                    let link = t.link;
                    self.nodes[i].engine.handle_event(
                        now,
                        PlatformEvent::WouldBlock {
                            link,
                            unit: &t.bytes,
                        },
                    );
                    self.nodes[i]
                        .engine
                        .handle_event(now, PlatformEvent::Writable { link });
                    continue;
                }
                let key = (i, t.link.0);
                let free_at = self.busy.get(&key).copied().unwrap_or(0);
                if free_at > now.0 {
                    let link = t.link;
                    self.nodes[i].engine.handle_event(
                        now,
                        PlatformEvent::WouldBlock {
                            link,
                            unit: &t.bytes,
                        },
                    );
                    self.seq += 1;
                    self.writable_at.insert((free_at, self.seq), (i, link));
                    continue;
                }
                let ser = t.bytes.len() as u64 * 8 * 1_000_000 / self.rate_bps.max(1);
                self.busy.insert(key, now.0 + ser);
                self.nodes[i]
                    .tx_log
                    .push((now.0, t.link.0, t.bytes.clone()));
                if let Some((li, peer, plink)) = self.peer_of(i, t.link)
                    && !self.links[li].blackout
                {
                    self.seq += 1;
                    self.in_flight.insert(
                        (now.0 + ser + self.delay_us, self.seq),
                        (peer, plink, Some(t.bytes)),
                    );
                }
            }
            while let Some(e) = self.nodes[i].engine.poll_effect() {
                progressed = true;
                self.apply(i, &e);
                self.nodes[i].effects.push(e);
            }
            if !progressed {
                break;
            }
        }
    }

    fn apply(&mut self, i: usize, e: &Effect) {
        let Effect::Radio(cmd) = e else {
            return;
        };
        match cmd {
            RadioCommand::SetHello { bytes } => self.nodes[i].hello = Some(*bytes),
            RadioCommand::SetDiscovery { mode, adv, .. } => {
                self.nodes[i].discovery = (*mode, adv.clone());
            }
            RadioCommand::Close { link, .. } => {
                if let Some((li, peer, plink)) = self.peer_of(i, *link) {
                    self.links[li].up = false;
                    // The peer learns of the close after what was written before it
                    // (delivered by the main loop as `Down{PeerClosed}`).
                    let written = self.busy.get(&(i, link.0)).copied().unwrap_or(0);
                    let at = written.max(self.now.0) + self.delay_us;
                    self.seq += 1;
                    self.in_flight.insert((at, self.seq), (peer, plink, None));
                }
            }
            RadioCommand::Connect { disc_id } => {
                let target = (*disc_id as usize).wrapping_sub(1_000);
                if target < self.nodes.len() {
                    self.pending.insert((i, *disc_id), target);
                }
            }
            RadioCommand::OpenLink { disc_id, via } => {
                if let Some(t) = self.pending.remove(&(i, *disc_id)) {
                    let saved = self.kind;
                    if matches!(via, OpenVia::Gatt) {
                        self.kind = LinkKind::BleGatt;
                    }
                    self.connect_with(i, t, Some(*disc_id));
                    self.kind = saved;
                }
            }
            RadioCommand::Abort { disc_id, .. } => {
                self.pending.remove(&(i, *disc_id));
            }
            _ => {}
        }
    }

    /// One advertising round: every node in range and discovering sees the others.
    pub fn advertise(&mut self) {
        let pairs = self.in_range.clone();
        for (a, b) in pairs {
            for (x, y) in [(a, b), (b, a)] {
                let (mode, adv) = self.nodes[y].discovery.clone();
                let Some(adv) = adv else { continue };
                if mode == DiscoveryMode::Off || self.nodes[x].discovery.0 == DiscoveryMode::Off {
                    continue;
                }
                let data = match self.nodes[y].platform {
                    Platform::Android => AdvData::ServiceData(adv.service_data.to_vec()),
                    _ => AdvData::LocalName(adv.local_name.clone()),
                };
                let d = Discovery {
                    disc_id: disc_of(y),
                    transport: Transport::Ble,
                    rssi: Some(-60),
                    adv: data,
                    platform_hint: None,
                };
                self.event(x, PlatformEvent::Discovered(d));
            }
        }
        // Serve the HELLO reads of the connects just issued.
        let pend: Vec<((usize, u32), usize)> = self.pending.iter().map(|(k, v)| (*k, *v)).collect();
        for ((x, disc), t) in pend {
            if let Some(h) = self.nodes[t].hello {
                self.event(
                    x,
                    PlatformEvent::HelloRead {
                        disc_id: disc,
                        bytes: &h,
                    },
                );
            }
        }
    }

    /// Advances simulated time to `until`, delivering packets and firing timeouts.
    pub fn run_until(&mut self, until: Instant) {
        let mut spins = 0u32;
        let mut last_now = self.now;
        loop {
            if self.now == last_now {
                spins += 1;
                assert!(
                    spins < 100_000,
                    "event loop stuck at {:?}: timeouts {:?}, links {:?}",
                    self.now,
                    self.nodes
                        .iter()
                        .map(|n| n.engine.poll_timeout())
                        .collect::<Vec<_>>(),
                    self.nodes
                        .iter()
                        .map(|n| n
                            .engine
                            .ctx()
                            .links
                            .iter()
                            .map(|l| format!("{l:?}"))
                            .collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                );
            } else {
                spins = 0;
                last_now = self.now;
            }
            for i in 0..self.nodes.len() {
                self.collect(i);
            }
            let next_pkt = self
                .in_flight
                .keys()
                .chain(self.writable_at.keys())
                .map(|(t, _)| Instant(*t))
                .min();
            let next_timer = self
                .nodes
                .iter()
                .filter_map(|n| n.engine.poll_timeout())
                .min();
            if let Some(p) = self.tick_us
                && self.next_tick <= self.now.0
            {
                self.next_tick = (self.now.0 / p.max(1) + 1) * p.max(1);
            }
            let next_tick = self.tick_us.map(|_| Instant(self.next_tick));
            let next = [next_pkt, next_timer, next_tick]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(Instant(u64::MAX))
                .max(self.now);
            if next > until {
                self.now = until;
                break;
            }
            self.now = next;
            let due: Vec<(u64, u64)> = self
                .in_flight
                .keys()
                .copied()
                .take_while(|(t, _)| *t <= next.0)
                .collect();
            for k in due {
                let now = self.now;
                match self.in_flight.remove(&k) {
                    Some((n, link, Some(bytes))) => {
                        self.nodes[n].engine.handle_packet(now, link, &bytes);
                        self.collect(n);
                    }
                    Some((n, link, None)) => {
                        let reason = LinkDownReason::PeerClosed;
                        self.nodes[n]
                            .engine
                            .handle_event(now, PlatformEvent::Down { link, reason });
                        self.collect(n);
                    }
                    None => {}
                }
            }
            let free: Vec<(u64, u64)> = self
                .writable_at
                .keys()
                .copied()
                .take_while(|(t, _)| *t <= next.0)
                .collect();
            for k in free {
                if let Some((n, link)) = self.writable_at.remove(&k) {
                    let now = self.now;
                    self.nodes[n]
                        .engine
                        .handle_event(now, PlatformEvent::Writable { link });
                    self.collect(n);
                }
            }
            let tick = self.tick_us.is_some() && self.next_tick <= self.now.0;
            for i in 0..self.nodes.len() {
                if tick
                    || self.nodes[i]
                        .engine
                        .poll_timeout()
                        .is_some_and(|t| t <= self.now)
                {
                    let now = self.now;
                    self.nodes[i].engine.handle_timeout(now);
                    self.collect(i);
                }
            }
        }
        for i in 0..self.nodes.len() {
            self.collect(i);
        }
    }

    /// Advances by `ms`.
    pub fn run_ms(&mut self, ms: u64) {
        let t = self.now.plus_ms(ms);
        self.run_until(t);
    }

    /// UI events of node `i`.
    pub fn ui_events(&self, i: usize) -> Vec<&UiEvent> {
        self.nodes[i]
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::Ui(ev) => Some(ev),
                _ => None,
            })
            .collect()
    }

    /// labrun/1 lines written by node `i` for `run_id` (JSONL file only).
    pub fn lab_lines(&self, i: usize, run_id: &str) -> Vec<String> {
        self.nodes[i]
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::LabWrite {
                    run_id: r,
                    file: LabFile::Jsonl,
                    line,
                } if r == run_id => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    /// Radio commands of node `i`.
    pub fn radio_cmds(&self, i: usize) -> Vec<&RadioCommand> {
        self.nodes[i]
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::Radio(c) => Some(c),
                _ => None,
            })
            .collect()
    }
}
