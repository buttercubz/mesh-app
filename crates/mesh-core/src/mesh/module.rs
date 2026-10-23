//! [`MeshModule`]: the `CoreModule` registration of the mesh glue (`wire/mesh.rs`).
//!
//! The group identity reaches the media module from the group module
//! (`GroupCore::sync_media` posts `MediaRequest::Identity` at the end of every hook that
//! can change it: session start, new epoch, leader handover, regenerated identity); the
//! glue derives its own copy from the stored group and posts nothing.

use mesh_wire::record::{Caps, Record};

use super::identity::IdentitySource;
use super::rl;
use super::state::{Mesh, MeshStats};
use crate::engine::{CoreModule, Ctx, EngineMode};
use crate::links::{LinkDownReason, LinkKind, LinkState};
use crate::{Instant, LinkId, NodeId, PlatformEvent};

/// The mesh module.
#[derive(Default)]
pub struct MeshModule {
    ident: IdentitySource,
    mesh: Option<Mesh>,
    now: Instant,
    /// Diagnostics: the NBR view `(node, kind) → (good, congested)` last reported.
    diag_nbr: std::collections::BTreeMap<(u16, u8), (bool, bool)>,
}

impl MeshModule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Counters (diagnostics, tests); `None` while inactive.
    pub fn stats(&self) -> Option<MeshStats> {
        self.mesh.as_ref().map(|m| m.stats)
    }

    /// Re-derives the identity from the stored group and (re)builds the glue state.
    fn sync_identity(&mut self, ctx: &mut Ctx) {
        self.now = ctx.now;
        if !self.ident.refresh(ctx) {
            return;
        }
        let Some(id) = self.ident.current().cloned() else {
            self.mesh = None;
            return;
        };
        match self.mesh.as_mut() {
            Some(m) if m.id.node == id.node => m.update_identity(id.clone()),
            _ => {
                let mut m = Mesh::new(ctx, id);
                for link in ctx.links.up_ids() {
                    m.link_up(ctx, link);
                }
                self.mesh = Some(m);
            }
        }
    }

    fn timer(&mut self, ctx: &mut Ctx) {
        let Some(m) = self.mesh.as_mut() else { return };
        let now = ctx.now.routing();
        m.sync_quality(ctx);
        m.after_queue(ctx);
        for a in m.router.tick(now) {
            m.apply(
                ctx,
                mesh_routing::RouteOutcome {
                    deliver: None,
                    out: Vec::new(),
                    actions: vec![a],
                },
            );
        }
        for o in m.router.poll(now) {
            m.schedule(ctx, o);
        }
        m.seg_timer(ctx);
        m.pacing_tick(ctx);
        m.send_nbr(ctx, false);
        m.select_tick(ctx);
        m.upgrade_tick(ctx);
        m.pump(ctx);
    }
}

impl CoreModule for MeshModule {
    fn name(&self) -> &'static str {
        "mesh"
    }

    fn on_mode(&mut self, ctx: &mut Ctx, mode: EngineMode) {
        self.ident.clear();
        self.mesh = None;
        if matches!(mode, EngineMode::Session { .. }) {
            self.sync_identity(ctx);
        }
    }

    fn on_link_up(&mut self, ctx: &mut Ctx, link: LinkId) {
        self.sync_identity(ctx);
        let Some(m) = self.mesh.as_mut() else { return };
        m.link_up(ctx, link);
        let aware_peer = ctx
            .links
            .get(link)
            .filter(|l| l.info.kind == LinkKind::WifiAware && l.state == LinkState::Up)
            .and_then(|l| l.peer_node);
        if let Some(p) = aware_peer
            && m.links.contains_key(&link)
        {
            m.upgrade_link_up(ctx, p.0);
        }
    }

    fn on_link_down(
        &mut self,
        ctx: &mut Ctx,
        link: LinkId,
        reason: LinkDownReason,
        _peer: Option<NodeId>,
    ) {
        self.now = ctx.now;
        if let Some(m) = self.mesh.as_mut() {
            m.link_down(ctx, link, reason);
        }
    }

    fn on_caps(&mut self, ctx: &mut Ctx, link: LinkId, caps: &Caps) {
        self.now = ctx.now;
        let Some(m) = self.mesh.as_mut() else { return };
        if let Some(meta) = m.links.get_mut(&link) {
            meta.peer_caps = Some(*caps);
            let peer = meta.peer;
            if let Some(n) = m.router.neighbors.neighbor_mut(peer) {
                n.platform = Some(caps.platform);
                n.role = Some(caps.role);
            }
        }
    }

    fn on_record(&mut self, ctx: &mut Ctx, link: LinkId, rec: &Record<'_>) -> bool {
        self.now = ctx.now;
        let Some(m) = self.mesh.as_mut() else {
            return false;
        };
        match rec {
            Record::Mesh(bytes) => {
                m.on_mesh(ctx, link, bytes);
                // Not claimed: later modules may still look at MESH records.
                false
            }
            Record::Nbr(n) => {
                m.on_nbr(ctx, link, n);
                true
            }
            Record::Upgrade(u) => {
                m.on_upgrade(ctx, link, u);
                true
            }
            _ => false,
        }
    }

    fn on_timer(&mut self, ctx: &mut Ctx) {
        self.sync_identity(ctx);
        self.timer(ctx);
    }

    fn on_originate(&mut self, ctx: &mut Ctx) {
        self.sync_identity(ctx);
        // Without a group identity the engine queues them on every Up link itself.
        let Some(m) = self.mesh.as_mut() else { return };
        for p in std::mem::take(&mut ctx.originated) {
            m.originate_own(ctx, p);
        }
        m.after_queue(ctx);
        m.pump(ctx);
    }

    fn next_timer(&self) -> Option<Instant> {
        let m = self.mesh.as_ref()?;
        let now = self.now;
        let routing = |t: mesh_routing::Instant| Instant(t.0);
        let mut ds: Vec<Instant> = Vec::new();
        ds.extend(m.router.next_deadline().map(routing));
        ds.extend(m.pacing.next_deadline().map(routing));
        ds.extend(m.seg.next_deadline().map(routing));
        ds.extend(m.pump_at);
        if !m.links.is_empty() {
            ds.push(
                m.nbr_last
                    .map_or(now, |t| t.plus_us(mesh_routing::consts::NBR_HEARTBEAT_US)),
            );
            ds.push(m.next_loss);
            ds.push(m.next_select);
        }
        ds.extend(
            m.router
                .floor
                .entries()
                .iter()
                .filter(|e| e.live())
                .map(|e| {
                    Instant(e.last_packet.0).plus_us(mesh_routing::consts::FLOOR_IDLE_RELEASE_US)
                }),
        );
        ds.into_iter().filter(|t| *t > now).min()
    }

    fn fill_diag(&mut self, ctx: &mut Ctx, s: &mut mesh_lab::ridelog::CoreSample) {
        let Some(m) = self.mesh.as_ref() else {
            self.diag_nbr.clear();
            return;
        };
        let r = &m.router.stats;
        let mc = s.mesh.get_or_insert_with(Default::default);
        mc.fwd = r.forwarded;
        mc.dup = r.dup;
        mc.suppressed = r.suppressed;
        mc.too_old = r.too_old;
        mc.pruned = r.pruned;
        mc.floor_refused = r.floor_refused;
        mc.voice_dropped = m.stats.voice_dropped;
        mc.bursts_shed = m.stats.bursts_shed;
        mc.own_refused = m.stats.own_refused;
        let mut now_view = std::collections::BTreeMap::new();
        for e in m.router.neighbors.own_entries(ctx.now.routing()) {
            let k = match e.kind {
                mesh_routing::LinkKind::Ble => 0u8,
                mesh_routing::LinkKind::Aware => 1,
                mesh_routing::LinkKind::Tunnel => 2,
            };
            now_view.insert((e.node_id, k), (e.good, e.congested, e.q));
        }
        mc.nbrs = u16::try_from(
            now_view
                .keys()
                .map(|(n, _)| *n)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
        )
        .unwrap_or(u16::MAX);
        let lab_kind = |ctx: &Ctx, node: u16, k: u8| {
            ctx.links
                .iter()
                .filter(|l| l.peer_node.map(|p| p.0) == Some(node))
                .map(|l| l.info.kind)
                .find(|kind| match k {
                    0 => kind.is_ble(),
                    1 => *kind == LinkKind::WifiAware,
                    _ => *kind == LinkKind::Tunnel,
                })
                .map_or(
                    match k {
                        0 => mesh_lab::LinkKind::BleL2cap,
                        1 => mesh_lab::LinkKind::Aware,
                        _ => mesh_lab::LinkKind::Tunnel,
                    },
                    LinkKind::lab,
                )
        };
        for (&(node, k), &(good, congested, q)) in &now_view {
            if self.diag_nbr.get(&(node, k)) != Some(&(good, congested)) {
                ctx.fx.diag(mesh_lab::ridelog::DiagEvent::NbrChange {
                    peer: node,
                    q: f32::from(q) / 100.0,
                    good,
                    congested,
                    kind: lab_kind(ctx, node, k),
                });
            }
        }
        for &(node, k) in self.diag_nbr.keys() {
            if !now_view.contains_key(&(node, k)) {
                ctx.fx.diag(mesh_lab::ridelog::DiagEvent::NbrChange {
                    peer: node,
                    q: 0.0,
                    good: false,
                    congested: false,
                    kind: lab_kind(ctx, node, k),
                });
            }
        }
        self.diag_nbr = now_view
            .into_iter()
            .map(|(k, (g, c, _))| (k, (g, c)))
            .collect();
    }

    fn on_event(&mut self, ctx: &mut Ctx, ev: &PlatformEvent<'_>) {
        self.now = ctx.now;
        let Some(m) = self.mesh.as_mut() else { return };
        match ev {
            PlatformEvent::WouldBlock { link, .. } => {
                // Overload shows up as WouldBlock: R ← 0.85 R at the next control step,
                // and the link is congested for 2 s (§5.1, §5.6).
                m.pacing.on_would_block(rl(*link));
                m.router
                    .neighbors
                    .mark_congested(ctx.now.routing(), rl(*link));
            }
            PlatformEvent::ConnectFailed {
                reason: LinkDownReason::LimitReached,
                ..
            } => m.selector.on_limit_reached(ctx.now.routing()),
            _ => {}
        }
    }
}
