//! In-memory world of real `Engine`s in group mode for the media tests.
//!
//! The secure module (WP-10B) is replaced by a test module: group links use plaintext
//! LAB-format frames (`TestSec`, not a Lab link) and are confirmed Up right after the
//! (empty) handshake. Links deliver every unit after a fixed one-way delay. Each node
//! runs a 10 ms audio clock (its own phase): the capture block that ends at `t` is
//! pushed at `t + TICK_ALIGN_US` (ring residence until the engine tick), and the block
//! pulled at `P` reaches the ear at `P + RENDER_BUFFER_US` (render ring). The clock
//! advances in 0.5 ms steps.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mesh_api::{CommandResult, Snapshot, UiCommand, UiEvent};
use mesh_core::effects::Effect;
use mesh_core::links::{LinkSecurity, Plain, SecurityError, SecurityStep};
use mesh_core::media::{RENDER_BUFFER_US, TICK_ALIGN_US};
use mesh_core::{
    BLOCK_SAMPLES, CoreModule, Ctx, Engine, EngineConfig, Instant, LinkId, LinkInfo, LinkKind,
    LinkRole, NodeId, Platform, SessionStart, StoredGroup,
};
use mesh_wire::frame::{LinkFrame, lab_tag};
use mesh_wire::mesh::{MeshPacket, MeshPayload};
use mesh_wire::record::{Record, RecordIter};

pub type Block = [i16; BLOCK_SAMPLES];

pub const GROUP: [u8; 16] = [7; 16];
pub const STEP_US: u64 = 500;
pub const SAMPLE_US: f64 = 62.5;

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
        proto_ver: u8,
        noise_msg: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) -> SecurityStep {
        self.0.on_hs(now, step, proto_ver, noise_msg, out)
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

/// MESH packets a test asks a node to send on every Up link (from its timer path).
pub type Inject = Rc<RefCell<Vec<Vec<u8>>>>;

/// Stands in for the secure module: supplies `TestSec`, confirms links Up, and sends the
/// test's injected MESH packets.
pub struct Glue {
    inject: Inject,
}

impl CoreModule for Glue {
    fn on_timer(&mut self, ctx: &mut Ctx) {
        let pkts: Vec<Vec<u8>> = self.inject.borrow_mut().drain(..).collect();
        let links: Vec<LinkId> = ctx
            .links
            .iter()
            .filter(|l| l.state == mesh_core::LinkState::Up)
            .map(|l| l.id)
            .collect();
        for p in &pkts {
            for &l in &links {
                ctx.send_record(l, mesh_core::links::Prio::P0, &Record::Mesh(p));
            }
        }
    }
    fn next_timer(&self) -> Option<Instant> {
        (!self.inject.borrow().is_empty()).then_some(Instant::ZERO)
    }
    fn name(&self) -> &'static str {
        "test-glue"
    }
    fn security_for(
        &mut self,
        _ctx: &mut Ctx,
        _link: LinkId,
        _info: &LinkInfo,
    ) -> Option<Box<dyn LinkSecurity>> {
        Some(Box::new(TestSec(Plain::new(lab_tag(1)))))
    }
    fn on_link_established(&mut self, ctx: &mut Ctx, link: LinkId) {
        let now = ctx.now;
        ctx.links.confirm_up(now, link, NodeId(link.0 as u16));
    }
}

/// A capture signal as a function of absolute time.
#[derive(Debug, Clone)]
pub enum Source {
    Silence,
    /// Sine `hz` at `amp` between `[from_us, until_us)`.
    Tone {
        hz: f64,
        amp: f64,
        from_us: u64,
        until_us: u64,
    },
    Sum(Vec<Source>),
    /// Pseudo-random block content (exact-copy checks).
    Noise {
        amp: i16,
        seed: u32,
    },
}

impl Source {
    pub fn tone(hz: f64, amp: f64, from_ms: u64, until_ms: u64) -> Source {
        Source::Tone {
            hz,
            amp,
            from_us: from_ms * 1_000,
            until_us: until_ms * 1_000,
        }
    }

    fn sample(&self, t_us: f64) -> f64 {
        match self {
            Source::Silence => 0.0,
            Source::Tone {
                hz,
                amp,
                from_us,
                until_us,
            } => {
                if t_us >= *from_us as f64 && t_us < *until_us as f64 {
                    amp * (2.0 * std::f64::consts::PI * hz * t_us / 1e6).sin()
                } else {
                    0.0
                }
            }
            Source::Sum(v) => v.iter().map(|s| s.sample(t_us)).sum(),
            Source::Noise { amp, seed } => {
                let k = (t_us / SAMPLE_US).round() as u64;
                let mut x = (k as u32).wrapping_mul(0x9E37_79B9) ^ seed;
                x ^= x >> 15;
                x = x.wrapping_mul(0x85EB_CA6B);
                x ^= x >> 13;
                f64::from((x % (2 * *amp as u32 + 1)) as i32 - i32::from(*amp))
            }
        }
    }

    /// The block whose last sample was captured just before `end_us`.
    pub fn block(&self, end_us: u64) -> Block {
        let start = end_us as f64 - 10_000.0;
        let mut b = [0i16; BLOCK_SAMPLES];
        for (j, s) in b.iter_mut().enumerate() {
            *s = self
                .sample(start + j as f64 * SAMPLE_US)
                .round()
                .clamp(-32768.0, 32767.0) as i16;
        }
        b
    }
}

/// One node.
pub struct Node {
    pub engine: Engine,
    pub source: Source,
    pub phase_us: u64,
    /// `(pull time, block)`.
    pub out: Vec<(u64, Block)>,
    pub effects: Vec<(u64, Effect)>,
    /// `(time, link, unit)` written by this node.
    pub tx: Vec<(u64, LinkId, Vec<u8>)>,
    pub audio: bool,
    /// Packets to send on the next timer call (see [`Glue`]).
    pub inject: Inject,
}

/// The world.
pub struct World {
    pub now: u64,
    pub nodes: Vec<Node>,
    /// `(node a, link at a, node b, link at b)`.
    pub links: Vec<(usize, LinkId, usize, LinkId)>,
    pub delay_us: u64,
    inflight: BTreeMap<(u64, u64), (usize, LinkId, Vec<u8>)>,
    seq: u64,
    next_link: u32,
    /// Count allocations made inside `push_capture` / `pull_playout`.
    pub count_audio: bool,
    pub audio_allocs: u64,
    pub audio_calls: u64,
}

pub fn seed(i: usize, salt: u8) -> [u8; 32] {
    let mut s = [salt; 32];
    s[0] = i as u8;
    s[1] = 0x5A;
    s
}

impl World {
    pub fn new(n: usize, platform: Platform, salt: u8) -> World {
        let nodes = (0..n)
            .map(|i| {
                let mut cfg = EngineConfig::new(platform, seed(i, salt));
                cfg.stored_groups = vec![StoredGroup {
                    group_id: GROUP,
                    blob: Vec::new(),
                }];
                let mut engine = Engine::new(cfg, Instant::ZERO);
                let inject = Inject::default();
                engine.register_module(Box::new(Glue {
                    inject: inject.clone(),
                }));
                Node {
                    engine,
                    source: Source::Silence,
                    phase_us: (i as u64 * 1_500) % 10_000,
                    out: Vec::new(),
                    effects: Vec::new(),
                    tx: Vec::new(),
                    audio: true,
                    inject,
                }
            })
            .collect();
        World {
            now: 0,
            nodes,
            links: Vec::new(),
            delay_us: 10_000,
            inflight: BTreeMap::new(),
            seq: 0,
            next_link: 1,
            count_audio: false,
            audio_allocs: 0,
            audio_calls: 0,
        }
    }

    pub fn start_all(&mut self) {
        for i in 0..self.nodes.len() {
            let now = Instant(self.now);
            self.nodes[i]
                .engine
                .start(now, SessionStart::Group { group_id: GROUP })
                .expect("group start");
            self.drain(i);
        }
    }

    pub fn connect(&mut self, a: usize, b: usize) {
        let la = LinkId(self.next_link);
        let lb = LinkId(self.next_link + 1);
        self.next_link += 2;
        self.links.push((a, la, b, lb));
        let now = Instant(self.now);
        for (n, l, role) in [(a, la, LinkRole::Initiator), (b, lb, LinkRole::Responder)] {
            let info = LinkInfo::new(LinkKind::BleL2cap, role);
            self.nodes[n]
                .engine
                .handle_event(now, mesh_core::PlatformEvent::Up { link: l, info });
            self.drain(n);
        }
    }

    /// Full mesh between every pair.
    pub fn connect_all(&mut self) {
        let n = self.nodes.len();
        for a in 0..n {
            for b in a + 1..n {
                self.connect(a, b);
            }
        }
    }

    pub fn command(&mut self, i: usize, cmd: UiCommand) -> CommandResult {
        let now = Instant(self.now);
        let r = self.nodes[i].engine.handle_command(now, cmd);
        self.drain(i);
        r
    }

    pub fn go_live(&mut self, i: usize, mode: mesh_api::TalkMode) {
        let r = self.command(i, UiCommand::GoLive {});
        assert!(r.is_ok(), "goLive: {r:?}");
        let r = self.command(i, UiCommand::SetTalkMode { mode });
        assert!(r.is_ok(), "setTalkMode: {r:?}");
    }

    /// RT flags at the current time (the engine sees `now` through a timer call first).
    pub fn set_flags(&mut self, i: usize, flags: u32) {
        let now = Instant(self.now);
        self.nodes[i].engine.handle_timeout(now);
        self.nodes[i].engine.set_rt_flags(flags);
        self.drain(i);
    }

    pub fn event(&mut self, i: usize, ev: mesh_core::PlatformEvent<'_>) {
        let now = Instant(self.now);
        self.nodes[i].engine.handle_event(now, ev);
        self.drain(i);
    }

    fn peer(&self, node: usize, link: LinkId) -> Option<(usize, LinkId)> {
        self.links.iter().find_map(|&(a, la, b, lb)| {
            if (a, la) == (node, link) {
                Some((b, lb))
            } else if (b, lb) == (node, link) {
                Some((a, la))
            } else {
                None
            }
        })
    }

    pub fn drain(&mut self, i: usize) {
        let now = self.now;
        while let Some(t) = self.nodes[i].engine.poll_transmit(Instant(now)) {
            if let Some((p, pl)) = self.peer(i, t.link) {
                self.seq += 1;
                self.inflight
                    .insert((now + self.delay_us, self.seq), (p, pl, t.bytes.clone()));
            }
            self.nodes[i].tx.push((now, t.link, t.bytes));
        }
        while let Some(e) = self.nodes[i].engine.poll_effect() {
            self.nodes[i].effects.push((now, e));
        }
    }

    fn step(&mut self) {
        let now = self.now;
        while let Some((&(at, s), _)) = self.inflight.iter().next() {
            if at > now {
                break;
            }
            let (n, l, bytes) = self.inflight.remove(&(at, s)).unwrap();
            self.nodes[n].engine.handle_packet(Instant(now), l, &bytes);
            self.drain(n);
        }
        for i in 0..self.nodes.len() {
            for _ in 0..8 {
                match self.nodes[i].engine.poll_timeout() {
                    Some(t) if t.0 <= now => {
                        self.nodes[i].engine.handle_timeout(Instant(now));
                        self.drain(i);
                    }
                    _ => break,
                }
            }
            let n = &mut self.nodes[i];
            if n.audio
                && now >= n.phase_us + TICK_ALIGN_US
                && (now - n.phase_us - TICK_ALIGN_US).is_multiple_of(10_000)
            {
                let block = n.source.block(now - TICK_ALIGN_US);
                let mut out = [0i16; BLOCK_SAMPLES];
                let before = crate::allocations();
                n.engine.push_capture(Instant(now), &block);
                n.engine.pull_playout(Instant(now), &mut out);
                if self.count_audio {
                    self.audio_allocs += crate::allocations() - before;
                    self.audio_calls += 1;
                }
                n.out.push((now, out));
                self.drain(i);
            }
        }
        self.now += STEP_US;
    }

    pub fn run_until_ms(&mut self, ms: u64) {
        while self.now < ms * 1_000 {
            self.step();
        }
    }

    pub fn run_ms(&mut self, ms: u64) {
        let end = self.now + ms * 1_000;
        while self.now < end {
            self.step();
        }
    }

    /// UI events of node `i` with their time.
    pub fn ui_events(&self, i: usize) -> Vec<(u64, UiEvent)> {
        self.nodes[i]
            .effects
            .iter()
            .filter_map(|(t, e)| match e {
                Effect::Ui(ev) => Some((*t, ev.clone())),
                _ => None,
            })
            .collect()
    }

    /// The last snapshot node `i` emitted.
    pub fn snapshot(&self, i: usize) -> Option<Snapshot> {
        self.ui_events(i)
            .into_iter()
            .rev()
            .find_map(|(_, e)| match e {
                UiEvent::Snapshot { s } => Some(s),
                _ => None,
            })
    }

    /// Every MESH packet node `i` wrote: `(time, link, header, payload bytes)`.
    pub fn sent_packets(
        &self,
        i: usize,
    ) -> Vec<(u64, LinkId, mesh_wire::mesh::MeshHeader, Vec<u8>)> {
        let mut v = Vec::new();
        for (t, l, unit) in &self.nodes[i].tx {
            // L2CAP stream unit: len u16 + LinkFrame.
            let Some(frame) = unit.get(2..) else { continue };
            let Ok(LinkFrame::Lab { records, .. }) = LinkFrame::decode(frame) else {
                continue;
            };
            for rec in RecordIter::new(records).flatten() {
                if let Record::Mesh(b) = rec
                    && let Ok(p) = MeshPacket::decode(b)
                {
                    v.push((*t, *l, p.header, b.to_vec()));
                }
            }
        }
        v
    }

    /// Decoded payloads of [`World::sent_packets`] on one link (each packet goes to
    /// every link; one link is enough to see the sequence).
    pub fn sent_payloads(&self, i: usize) -> Vec<(u64, mesh_wire::mesh::MeshHeader, OwnedPayload)> {
        let pk = self.sent_packets(i);
        let Some(first_link) = pk.first().map(|p| p.1) else {
            return Vec::new();
        };
        pk.into_iter()
            .filter(|p| p.1 == first_link)
            .filter_map(|(t, _, h, b)| {
                let p = MeshPacket::decode(&b).ok()?;
                let owned = match MeshPayload::decode(&p).ok()? {
                    MeshPayload::Voice(v) => OwnedPayload::Voice {
                        burst: v.burst,
                        ts: v.ts,
                        cfg: v.cfg.0,
                    },
                    MeshPayload::Talk(t) => OwnedPayload::Talk(t),
                    MeshPayload::FloorReject(r) => OwnedPayload::Reject(r),
                    _ => return None,
                };
                Some((t, h, owned))
            })
            .collect()
    }

    /// Output samples of node `i` with their ear time (µs), between two pull times.
    pub fn ear(&self, i: usize, from_us: u64, to_us: u64) -> Vec<(f64, i16)> {
        let mut v = Vec::new();
        for (p, b) in &self.nodes[i].out {
            if *p < from_us || *p >= to_us {
                continue;
            }
            for (j, &s) in b.iter().enumerate() {
                v.push((
                    *p as f64 + RENDER_BUFFER_US as f64 + j as f64 * SAMPLE_US,
                    s,
                ));
            }
        }
        v
    }
}

/// A decoded payload kept after the unit buffer is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedPayload {
    Voice { burst: u8, ts: u16, cfg: u8 },
    Talk(mesh_wire::mesh::Talk),
    Reject(mesh_wire::mesh::FloorReject),
}
