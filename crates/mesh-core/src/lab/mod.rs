//! Lab mode (DESIGN §12, owned by WP-10A): the Lab code and LAB link security, the
//! random `lab_node_id` with announce and collision redraw (§8.2), LAB_CTL controller
//! and follower roles, bench runs from mesh-lab with their audio path and anchor, the
//! minimal TTL + dedup flood for MESH_PROBE, platform samples and labrun/1 output.

pub mod convert;
pub mod ctl;
pub mod flood;
pub mod index;
mod ops;
pub mod run;

use std::collections::BTreeMap;

use mesh_api::{NoticeKey, ParamValue, Params};
use mesh_wire::frame::lab_tag;
use mesh_wire::record::{LabCtl, LabCtlOp};

use crate::config::EngineConfig;
use crate::engine::Ctx;
use crate::links::{LinkState, Prio, RadioTarget};
use crate::{Instant, LinkId};

pub use flood::{FloodOutcome, FloodStats, LabFlood};
pub use index::RunIndex;
pub use run::LabRun;

/// A peer heard on a Lab link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabPeer {
    /// Announced `lab_node_id`.
    pub node: Option<u16>,
    pub nonce: u64,
    pub last_rx: Instant,
}

/// A peer counts as heard for this long after its last frame.
pub const PEER_HEARD_US: u64 = 2_000_000;

/// Lab mode state.
#[derive(Debug)]
pub struct LabMode {
    code: u16,
    lab_tag: u16,
    node_id: u16,
    nonce: u64,
    redraws: u32,
    pub(crate) peers: BTreeMap<LinkId, LabPeer>,
    pub(crate) run: Option<LabRun>,
    pub(crate) flood: LabFlood,
    pub(crate) index: RunIndex,
    capture_index: u64,
    click: Vec<i16>,
    click_pos: usize,
}

impl LabMode {
    pub fn new(cfg: &EngineConfig) -> Self {
        LabMode {
            code: 0,
            lab_tag: lab_tag(0),
            node_id: 0,
            nonce: 0,
            redraws: 0,
            peers: BTreeMap::new(),
            run: None,
            flood: LabFlood::new(0),
            index: RunIndex::new(cfg.lab_runs.clone()),
            capture_index: 0,
            click: Vec::new(),
            click_pos: 0,
        }
    }

    /// The Lab code of the current session.
    pub fn code(&self) -> u16 {
        self.code
    }

    /// `SHA-256("cvic/lab" ‖ u16 code)[0..2]` of the current code.
    pub fn lab_tag(&self) -> u16 {
        self.lab_tag
    }

    /// This node's `lab_node_id`.
    pub fn node_id(&self) -> u16 {
        self.node_id
    }

    /// How many times the id was redrawn after a collision.
    pub fn redraws(&self) -> u32 {
        self.redraws
    }

    /// A run is active on this device.
    pub fn run_active(&self) -> bool {
        self.run.is_some()
    }

    /// The active run.
    pub fn run(&self) -> Option<&LabRun> {
        self.run.as_ref()
    }

    /// Flood counters.
    pub fn flood_stats(&self) -> FloodStats {
        self.flood.stats
    }

    /// Distinct peers that announced an id.
    pub fn peers_known(&self) -> usize {
        let mut ids: Vec<u16> = self.peers.values().filter_map(|p| p.node).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    }

    /// Peers heard within the last 2 s.
    pub fn peers_heard(&self, now: Instant) -> usize {
        self.peers
            .values()
            .filter(|p| p.node.is_some() && now.since(p.last_rx) < PEER_HEARD_US)
            .count()
    }

    /// Enters Lab mode with `code`: draws the id, starts discovery with a Lab HELLO and
    /// advertising, and makes every link ping at the active rate.
    pub fn start(&mut self, ctx: &mut Ctx, code: u16) {
        self.code = code;
        self.lab_tag = lab_tag(code);
        self.peers.clear();
        self.run = None;
        self.draw_id(ctx);
        self.redraws = 0;
        self.flood = LabFlood::new((ctx.rand_u32() & 0xFFFF) as u16);
        let mut token = [0u8; 4];
        ctx.fill(&mut token);
        let caps = crate::links::own_caps(
            ctx.cfg.platform,
            ctx.cfg.app_version,
            &ctx.cfg.limits,
            false,
            (0, 0, 0),
        );
        ctx.links.set_own_caps(caps);
        ctx.links.set_fast_ping(true);
        ctx.radio.start(
            RadioTarget::Lab {
                lab_tag: self.lab_tag,
            },
            token,
            &mut ctx.fx,
        );
        ctx.refresh_radio();
    }

    /// Leaves Lab mode: the active run ends (summary written, audio closed).
    pub fn stop(&mut self, ctx: &mut Ctx) {
        if self.run.is_some() {
            self.end_run(ctx, true);
        }
        self.peers.clear();
        ctx.links.set_fast_ping(false);
    }

    fn draw_id(&mut self, ctx: &mut Ctx) {
        let old = self.node_id;
        loop {
            let id = (ctx.rand_u32() & 0xFFFF) as u16;
            let taken = id == 0 || id == old || self.peers.values().any(|p| p.node == Some(id));
            if !taken {
                self.node_id = id;
                break;
            }
        }
        self.nonce = ctx.rand_u64();
    }

    /// Sends `LAB_CTL{announce}` (direct) on every Lab link.
    fn announce_all(&mut self, ctx: &mut Ctx) {
        let links: Vec<LinkId> = self.peers.keys().copied().collect();
        for l in links {
            self.announce(ctx, l, self.node_id, self.nonce, ctl::ANNOUNCE_DIRECT);
        }
    }

    fn announce(&self, ctx: &mut Ctx, link: LinkId, id: u16, nonce: u64, kind: u32) {
        let params = LabCtl::announce_params(id, nonce);
        let rec = ctl::record(LabCtlOp::Announce, kind, 0, &params);
        ctx.links.send_record(link, Prio::P0, &rec, None);
    }

    /// Test/simulator hook: forces the id (to provoke a collision) and re-announces.
    pub(crate) fn force_node_id(&mut self, ctx: &mut Ctx, id: u16) {
        self.node_id = id;
        self.announce_all(ctx);
    }

    /// A peer announced (or a neighbor relayed) `(id, nonce)`. The side with the larger
    /// nonce redraws and re-announces (§8.2).
    fn check_collision(&mut self, ctx: &mut Ctx, id: u16, nonce: u64) {
        // The same nonce is our own announce (2⁻⁶⁴ otherwise).
        if id != self.node_id || nonce == self.nonce {
            return;
        }
        if self.nonce > nonce {
            self.redraw(ctx);
        }
    }

    fn redraw(&mut self, ctx: &mut Ctx) {
        self.draw_id(ctx);
        self.redraws += 1;
        ctx.fx.log(
            crate::effects::LogLevel::Info,
            format!("lab: node id collision, redrawn as {}", self.node_id),
        );
        self.announce_all(ctx);
    }

    /// A Lab link is Up: announce ourselves; a controller brings the peer into its run.
    pub fn on_link_up(&mut self, ctx: &mut Ctx, link: LinkId) {
        self.peers.insert(
            link,
            LabPeer {
                node: None,
                nonce: 0,
                last_rx: ctx.now,
            },
        );
        self.announce(ctx, link, self.node_id, self.nonce, ctl::ANNOUNCE_DIRECT);
        let Some((kind, role)) = ctx.links.get(link).map(|l| (l.info.kind, l.info.role)) else {
            return;
        };
        if let Some(run) = self.run.as_mut() {
            run.runner.on_link_up(
                ctx.now.lab(),
                link.0,
                kind.lab(),
                Some(run::role_of(role)),
                None,
            );
            self.send_start(ctx, Some(link));
        }
    }

    /// A Lab link went down.
    pub fn on_link_down(&mut self, ctx: &mut Ctx, link: LinkId, reason: crate::LinkDownReason) {
        if let Some(p) = self.peers.remove(&link)
            && let Some(n) = p.node
        {
            let mut params = Params::new();
            params.insert("labNode".into(), ParamValue::from(u32::from(n)));
            ctx.notice(NoticeKey::LabPeerLost, params);
        }
        if let Some(run) = self.run.as_mut() {
            run.runner
                .on_link_down(ctx.now.lab(), link.0, reason.as_str());
        }
    }

    /// Radio metrics of a Lab link (RSSI, ATT max write = MTU − 3).
    pub fn on_metrics(&mut self, link: LinkId, rssi: Option<i16>, att_mtu: Option<u16>) {
        if let Some(run) = self.run.as_mut() {
            run.runner
                .on_link_info(link.0, rssi, att_mtu.map(|m| m.saturating_sub(3)));
        }
    }

    /// Sends `LAB_CTL{start}` of the active run to `only` or to every Lab link.
    fn send_start(&self, ctx: &mut Ctx, only: Option<LinkId>) {
        let Some(run) = self.run.as_ref() else {
            return;
        };
        let p = run.runner.params();
        let params = ctl::start_params(p, run.unix_minutes);
        let rec = ctl::record(LabCtlOp::Start, run.wire_id, p.bench.wire_id(), &params);
        for l in self.lab_links(ctx) {
            if only.is_none_or(|o| o == l) {
                ctx.links.send_record(l, Prio::P0, &rec, None);
            }
        }
    }

    fn lab_links(&self, ctx: &Ctx) -> Vec<LinkId> {
        ctx.links
            .iter()
            .filter(|l| l.state == LinkState::Up && l.is_lab())
            .map(|l| l.id)
            .collect()
    }
}
