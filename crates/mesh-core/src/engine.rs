//! The sans-IO [`Engine`] (contract C3, DESIGN §3.2): dispatch of commands, events,
//! packets and timeouts; the shared [`Ctx`] the modules work on; the [`CoreModule`]
//! hook trait and the [`ModuleRegistry`] filled by `wire::register_all()` (§3.3).

use mesh_api::{CommandResult, ErrorCode, NoticeKey, Params, Snapshot, UiCommand, UiEvent};
use mesh_wire::record::{Caps, Record, RecordIter};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

use crate::config::{EngineConfig, SessionStart};
use crate::effects::{Effect, EffectQueue, Notification, Transmit};
use crate::lab::LabMode;
use crate::links::{
    LinkDownReason, LinkEvent, LinkInfo, LinkSecurity, LinkTable, Prio, RadioPolicy,
};
use crate::ui::{DeviceState, UiState};
use crate::{BLOCK_SAMPLES, Instant, LinkId, NodeId, PlatformEvent};

/// Engine mode (DESIGN §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineMode {
    /// No radios, no audio; group management commands still work.
    Idle,
    /// Lab mode with the shared code (§12).
    Lab { code: u16 },
    /// Group session (lobby or riding).
    Session { group_id: [u8; 16] },
}

/// Why `Engine::start` refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// Demo rides are run by the runtime with mesh-sim, not by the engine.
    DemoIsRuntime,
    /// The Lab code is not a 4-digit code (0-9999).
    BadLabCode,
    /// The group is not stored on this device.
    UnknownGroup,
}

/// A MESH packet this node originates (DESIGN §5.2 "origination"), handed by the module
/// that owns its content (group: PRESENCE, ROSTER…; media: TALK, VOICE, near-talker
/// FLOOR_REJECT) to the mesh glue through [`Ctx::originate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginPacket {
    /// Link-layer priority (§5.6): P0 TALK/FLOOR_REJECT and control that silences the
    /// group, P1 voice, P2 presence/roster/position.
    pub prio: Prio,
    /// The encoded MESH record body (9-byte header + payload). Its `seq` comes from
    /// [`Ctx::next_mesh_seq`]; `age` is the origin's queueing delay so far (§4.5).
    pub body: Vec<u8>,
    /// Never sent after this instant (voice: creation + `VOICE_MAX_AGE`).
    pub deadline: Option<Instant>,
}

/// Everything the engine shares with its modules.
pub struct Ctx {
    /// Time of the call being handled.
    pub now: Instant,
    pub cfg: EngineConfig,
    /// The engine's only RNG (seeded from `cfg.rng_seed`).
    pub rng: ChaCha20Rng,
    pub mode: EngineMode,
    /// Origin session tag: random non-zero u8 drawn at every start (§4.5).
    pub sess: u8,
    /// The next `seq` of this origin: one counter per `(origin, sess)` shared by every
    /// module that originates MESH packets (§4.5, §5.3), drawn at random at every start
    /// next to `sess`. Modules take values with [`Ctx::next_mesh_seq`] /
    /// [`Ctx::reserve_mesh_seqs`]; tests and simulators may set it (e.g. near the wrap).
    pub mesh_seq: u16,
    /// MESH packets originated during this engine call, waiting for the mesh glue
    /// ([`CoreModule::on_originate`]): it sends them to every neighbor through the same
    /// per-link admission, staging queues, pacing and airtime budget as relayed
    /// packets. The engine hands them over at the end of every
    /// `handle_*`/`start`/`stop` call; what no module took is queued on every Up group
    /// link directly (no glue running).
    pub originated: Vec<OriginPacket>,
    pub fx: EffectQueue,
    pub links: LinkTable,
    pub radio: RadioPolicy,
    pub device: DeviceState,
    pub ui: UiState,
    /// Last RT flags (`mesh_rt_set_flags`, §3.6).
    pub rt_flags: u32,
    /// Inter-module bus: requests for the media module posted by other modules (the
    /// session → media seam, [`crate::media::MediaRequest`]) or by the runtime
    /// ([`Engine::media_request`]). The engine hands them to [`CoreModule::on_bus`] at
    /// the end of every `handle_*`/`start`/`stop` call, before the snapshot is built, so
    /// a request posted while handling a command takes effect within the same call.
    /// Requests posted from `push_capture`, `pull_playout` or `set_rt_flags` wait for
    /// the next such call.
    pub media_bus: Vec<crate::media::MediaRequest>,
    /// Latency estimator export (DESIGN §12.6, `LatencyEstimator::export`), refreshed at
    /// 1 Hz by the media module while in a group session; `None` outside one. Read by
    /// the ride recorder.
    pub media_latency: Option<serde_json::Value>,
    /// Diagnostics tick state (`ridelog/1`, [`crate::diag`]).
    pub diag: crate::diag::DiagTracker,
}

impl Ctx {
    /// Posts a request for the media module on the inter-module bus (see
    /// [`Ctx::media_bus`]).
    pub fn post_media(&mut self, req: crate::media::MediaRequest) {
        self.media_bus.push(req);
    }

    /// The `seq` of the next MESH packet this node originates, and advances the shared
    /// per-origin counter (serial-number arithmetic: it wraps from 65535 to 0). Every
    /// originating module draws from it, so receivers keep one dedup window per
    /// `(origin, sess)` (§5.3) whatever the packet class.
    pub fn next_mesh_seq(&mut self) -> u16 {
        let s = self.mesh_seq;
        self.mesh_seq = s.wrapping_add(1);
        s
    }

    /// Reserves `n` consecutive `seq`s (the segments of one ROSTER/JOIN_REQ/REKEY body,
    /// §4.6) and returns the first.
    pub fn reserve_mesh_seqs(&mut self, n: u16) -> u16 {
        let s = self.mesh_seq;
        self.mesh_seq = s.wrapping_add(n);
        s
    }

    /// Originates a MESH packet: the mesh glue sends it to every neighbor at the end of
    /// this engine call (see [`Ctx::originated`]).
    pub fn originate(&mut self, p: OriginPacket) {
        self.originated.push(p);
    }

    /// Originates a MESH packet that must be on the link-layer queue of every Up group
    /// link before the caller closes those links in the same call (a LEAVE, the
    /// GROUP_CTL that closes the group, the ROSTER/REKEY that deny a member whose link
    /// closes right behind them): queued on every Up group link at once, bypassing the
    /// glue's staging and pacing, as the ordering of the link queue is what makes it
    /// leave before `CLOSE`. Returns the number of links it was queued on.
    pub fn originate_now(&mut self, p: &OriginPacket) -> usize {
        let ids: Vec<LinkId> = self
            .links
            .iter()
            .filter(|l| l.state == crate::links::LinkState::Up && !l.is_lab())
            .map(|l| l.id)
            .collect();
        ids.into_iter()
            .filter(|l| {
                self.links
                    .send_record(*l, p.prio, &Record::Mesh(&p.body), p.deadline)
            })
            .count()
    }

    /// Next random u32.
    pub fn rand_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }

    /// Next random u64.
    pub fn rand_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    /// Random non-zero byte.
    pub fn rand_nonzero_u8(&mut self) -> u8 {
        loop {
            let b = (self.rng.next_u32() & 0xFF) as u8;
            if b != 0 {
                return b;
            }
        }
    }

    /// Fills `buf` with random bytes.
    pub fn fill(&mut self, buf: &mut [u8]) {
        self.rng.fill_bytes(buf);
    }

    /// Something the snapshot shows changed.
    pub fn mark_dirty(&mut self) {
        self.ui.mark_dirty();
    }

    /// Emits a UI event.
    pub fn ui_event(&mut self, ev: UiEvent) {
        self.fx.ui(ev);
    }

    /// Emits a notice (i18n key + params) to the UI.
    pub fn notice(&mut self, key: NoticeKey, params: Params) {
        self.fx.ui(UiEvent::Notice { key, params });
    }

    /// Emits a notice and the matching local notification.
    pub fn notice_and_notify(&mut self, key: NoticeKey, params: Params) {
        self.fx.push(Effect::Notify(Notification {
            key,
            params: params.clone(),
        }));
        self.notice(key, params);
    }

    /// Queues one record on an established link.
    pub fn send_record(&mut self, link: LinkId, prio: Prio, rec: &Record<'_>) -> bool {
        self.links.send_record(link, prio, rec, None)
    }

    /// `true` in Lab mode.
    pub fn is_lab(&self) -> bool {
        matches!(self.mode, EngineMode::Lab { .. })
    }

    /// Number of active BLE links (counted against the BLE max).
    pub fn ble_links(&self) -> usize {
        self.links.active_count(true)
    }

    /// Re-evaluates HELLO and discovery after a link-count change.
    pub fn refresh_radio(&mut self) {
        let n = self.links.active_count(true);
        let limits = self.cfg.limits;
        self.radio.refresh(n, &limits, self.now, &mut self.fx);
    }
}

/// Hooks of a mesh-core module (secure, group, session, media, mesh). Every method has
/// a no-op default; a module overrides what it needs. Calls happen in registration
/// order (`wire::register_all`).
pub trait CoreModule {
    /// Module name (logs, diagnostics).
    fn name(&self) -> &'static str;

    /// The engine changed mode (start, stop).
    fn on_mode(&mut self, _ctx: &mut Ctx, _mode: EngineMode) {}

    /// `Engine::stop` is about to close every link: the last chance to queue P0
    /// records on the links that are still Up (the session floods its signed LEAVE,
    /// §1.3 F8). The links then close gracefully: what is queued at P0 leaves first,
    /// followed by `CLOSE{leaving}`; `on_mode(Idle)` comes after.
    fn on_stopping(&mut self, _ctx: &mut Ctx) {}

    /// Supplies the security of a new non-Lab link (the secure module).
    fn security_for(
        &mut self,
        _ctx: &mut Ctx,
        _link: LinkId,
        _info: &LinkInfo,
    ) -> Option<Box<dyn LinkSecurity>> {
        None
    }

    /// Keys are established; IDPROOF is due (then `ctx.links.confirm_up`).
    fn on_link_established(&mut self, _ctx: &mut Ctx, _link: LinkId) {}

    /// A group link is Up.
    fn on_link_up(&mut self, _ctx: &mut Ctx, _link: LinkId) {}

    /// A group link went down.
    fn on_link_down(
        &mut self,
        _ctx: &mut Ctx,
        _link: LinkId,
        _reason: LinkDownReason,
        _peer: Option<NodeId>,
    ) {
    }

    /// The peer's CAPS arrived on a group link.
    fn on_caps(&mut self, _ctx: &mut Ctx, _link: LinkId, _caps: &Caps) {}

    /// A record of a group link that the link layer did not consume. Return `true` when
    /// handled (later modules do not see it).
    fn on_record(&mut self, _ctx: &mut Ctx, _link: LinkId, _rec: &Record<'_>) -> bool {
        false
    }

    /// Timers (called on every `handle_timeout`).
    fn on_timer(&mut self, _ctx: &mut Ctx) {}

    /// The module's next deadline.
    fn next_timer(&self) -> Option<Instant> {
        None
    }

    /// A UI command not handled by the base engine; `Some` answers it.
    fn on_command(&mut self, _ctx: &mut Ctx, _cmd: &UiCommand) -> Option<CommandResult> {
        None
    }

    /// Every platform event (after the base engine handled it).
    fn on_event(&mut self, _ctx: &mut Ctx, _ev: &PlatformEvent<'_>) {}

    /// A HELLO read in group mode; `true` when the module answered with
    /// `OpenLink`/`Abort` itself (retained-epoch tag checks).
    fn on_hello_read(&mut self, _ctx: &mut Ctx, _disc_id: u32, _bytes: &[u8]) -> bool {
        false
    }

    /// 10 ms of capture (must not allocate).
    fn on_capture(&mut self, _ctx: &mut Ctx, _pcm: &[i16; BLOCK_SAMPLES]) {}

    /// 10 ms of playout to mix into `out` (must not allocate).
    fn on_playout(&mut self, _ctx: &mut Ctx, _out: &mut [i16; BLOCK_SAMPLES]) {}

    /// RT flags changed, or a latch toggle (bit3, an edge) arrived again.
    fn on_rt_flags(&mut self, _ctx: &mut Ctx, _flags: u32) {}

    /// MESH packets this node originated are pending in `ctx.originated`: the mesh glue
    /// takes them (drains the vector) and sends them through its per-link path. Called
    /// on every module, in registration order, at the end of an engine call while the
    /// vector is not empty; whatever is left afterwards is queued on every Up group link
    /// directly.
    fn on_originate(&mut self, _ctx: &mut Ctx) {}

    /// Requests are pending on the inter-module bus (`ctx.media_bus`): the media module
    /// drains them. Called on every module, in registration order, at the end of an
    /// engine call while the bus is not empty (bounded to [`MAX_BUS_PASSES`] passes).
    fn on_bus(&mut self, _ctx: &mut Ctx) {}

    /// Adds the module's part of the snapshot.
    fn fill_snapshot(&self, _ctx: &Ctx, _s: &mut Snapshot) {}

    /// The 1 Hz diagnostics tick (only while diagnostics are on): adds the module's
    /// cumulative counters and gauges to the core sample and may emit change events
    /// through `ctx.fx.diag`. Must not change any decision of the module.
    fn fill_diag(&mut self, _ctx: &mut Ctx, _s: &mut mesh_lab::ridelog::CoreSample) {}
}

/// Passes over the modules per engine call while the inter-module bus is not empty;
/// requests still pending after that are dropped and logged (a module that keeps
/// re-posting cannot spin the engine).
pub const MAX_BUS_PASSES: usize = 8;

/// The registered modules, in call order.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: Vec<Box<dyn CoreModule>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a module (called from `wire/<module>.rs`).
    pub fn register(&mut self, m: Box<dyn CoreModule>) {
        self.modules.push(m);
    }

    /// Adds a module ahead of every registered one (instrumentation that must see the
    /// inter-module bus before the media module drains it).
    pub fn register_first(&mut self, m: Box<dyn CoreModule>) {
        self.modules.insert(0, m);
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Module names in call order.
    pub fn names(&self) -> Vec<&'static str> {
        self.modules.iter().map(|m| m.name()).collect()
    }

    pub(crate) fn each(&mut self, mut f: impl FnMut(&mut dyn CoreModule)) {
        for m in &mut self.modules {
            f(m.as_mut());
        }
    }

    pub(crate) fn first<T>(
        &mut self,
        mut f: impl FnMut(&mut dyn CoreModule) -> Option<T>,
    ) -> Option<T> {
        self.modules.iter_mut().find_map(|m| f(m.as_mut()))
    }

    pub(crate) fn next_timer(&self) -> Option<Instant> {
        self.modules.iter().filter_map(|m| m.next_timer()).min()
    }

    pub(crate) fn fill_snapshot(&self, ctx: &Ctx, s: &mut Snapshot) {
        for m in &self.modules {
            m.fill_snapshot(ctx, s);
        }
    }
}

/// The engine.
pub struct Engine {
    pub(crate) ctx: Ctx,
    pub(crate) lab: LabMode,
    pub(crate) modules: ModuleRegistry,
}

impl Engine {
    /// A new engine in Idle (no radios, no audio). Registers every module of
    /// `wire::register_all()`.
    pub fn new(cfg: EngineConfig, now: Instant) -> Engine {
        let mut rng = ChaCha20Rng::from_seed(cfg.rng_seed);
        let mut links = LinkTable::new();
        links.seed_ping_ids((rng.next_u32() & 0xFFFF) as u16);
        let platform = cfg.platform;
        let device = DeviceState::new(now, cfg.wall_ms);
        let lab = LabMode::new(&cfg);
        let mut modules = ModuleRegistry::new();
        crate::wire::register_all(&mut modules);
        let mut ctx = Ctx {
            now,
            cfg,
            rng,
            mode: EngineMode::Idle,
            sess: 1,
            mesh_seq: 0,
            originated: Vec::new(),
            fx: EffectQueue::new(),
            links,
            radio: RadioPolicy::new(platform),
            device,
            ui: UiState::new(),
            rt_flags: 0,
            media_bus: Vec::new(),
            media_latency: None,
            diag: crate::diag::DiagTracker::default(),
        };
        ctx.sess = ctx.rand_nonzero_u8();
        Engine { ctx, lab, modules }
    }

    /// Registers an extra module (tests, simulator instrumentation).
    pub fn register_module(&mut self, m: Box<dyn CoreModule>) {
        self.modules.register(m);
    }

    /// Registers an extra module that runs before every built-in module (tests and
    /// simulator instrumentation, e.g. an observer of `Ctx::media_bus`: its `on_bus`
    /// sees the requests before the media module drains them). Such a module must stay
    /// passive: it answers no command and claims no record.
    #[doc(hidden)]
    pub fn register_module_first(&mut self, m: Box<dyn CoreModule>) {
        self.modules.register_first(m);
    }

    /// Read access to the shared state.
    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    /// Current mode.
    pub fn mode(&self) -> EngineMode {
        self.ctx.mode
    }

    /// Names of the registered modules.
    pub fn module_names(&self) -> Vec<&'static str> {
        self.modules.names()
    }

    /// Starts radios for a group session or Lab mode (audio stays off).
    pub fn start(&mut self, now: Instant, start: SessionStart) -> Result<(), StartError> {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        let mode = match start {
            SessionStart::Demo => return Err(StartError::DemoIsRuntime),
            SessionStart::Lab { lab_code } if lab_code > 9_999 => {
                return Err(StartError::BadLabCode);
            }
            SessionStart::Lab { lab_code } => EngineMode::Lab { code: lab_code },
            SessionStart::Group { group_id } => {
                if !self
                    .ctx
                    .cfg
                    .stored_groups
                    .iter()
                    .any(|g| g.group_id == group_id)
                {
                    return Err(StartError::UnknownGroup);
                }
                EngineMode::Session { group_id }
            }
        };
        if self.ctx.mode != EngineMode::Idle {
            self.stop(now);
        }
        self.ctx.sess = self.ctx.rand_nonzero_u8();
        if let EngineMode::Session { .. } = mode {
            // One random initial `seq` per (origin, sess), shared by every module that
            // originates MESH packets (§4.5, §5.3). Lab mode floods only its own probes
            // and keeps its counter (`lab::flood`).
            self.ctx.mesh_seq = (self.ctx.rand_u32() & 0xFFFF) as u16;
        }
        self.ctx.originated.clear();
        self.ctx.mode = mode;
        self.ctx.ui.set_engine(mesh_api::EngineState::Running);
        match mode {
            EngineMode::Lab { code } => self.lab.start(&mut self.ctx, code),
            EngineMode::Session { .. } => {
                let caps = crate::links::own_caps(
                    self.ctx.cfg.platform,
                    self.ctx.cfg.app_version,
                    &self.ctx.cfg.limits,
                    false,
                    (0, 0, 0),
                );
                self.ctx.links.set_own_caps(caps);
                self.ctx.links.set_fast_ping(false);
            }
            EngineMode::Idle => {}
        }
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_mode(ctx, mode));
        self.after();
        Ok(())
    }

    /// Stops radios and audio and returns to Idle. Modules act first
    /// ([`CoreModule::on_stopping`]); then every established link closes gracefully:
    /// its queued P0 records (the session's LEAVE) leave, then `CLOSE{leaving}`, and
    /// the platform close follows once drained (at most [`crate::links::CLOSE_FLUSH_US`]
    /// later, so the runtime keeps flushing transmits and serving `poll_timeout` in
    /// Idle). Links still in their handshake are closed at once.
    pub fn stop(&mut self, now: Instant) {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        if self.ctx.mode == EngineMode::Idle {
            return;
        }
        if self.ctx.is_lab() {
            self.lab.stop(&mut self.ctx);
        }
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_stopping(ctx));
        // What the modules originated last (the session's LEAVE) goes straight onto the
        // link queues so it leaves ahead of `CLOSE{leaving}`.
        self.flush_originated_direct();
        self.process_link_events();
        let ids: Vec<LinkId> = self.ctx.links.iter().map(|l| l.id).collect();
        let leaving = mesh_wire::frame::RejectReason::Leaving;
        for id in ids {
            let established = self.ctx.links.get(id).is_some_and(|l| l.established());
            if established {
                self.ctx
                    .links
                    .close_graceful(now, id, leaving, Prio::P0, &mut self.ctx.fx);
            } else {
                self.ctx.links.close_now(
                    id,
                    Some(leaving),
                    LinkDownReason::LocalClose,
                    &mut self.ctx.fx,
                );
            }
        }
        self.ctx.radio.stop(&mut self.ctx.fx);
        self.ctx.mode = EngineMode::Idle;
        self.ctx.ui.set_engine(mesh_api::EngineState::Idle);
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_mode(ctx, EngineMode::Idle));
        self.after();
    }

    /// Handles a UI command (works in Idle too).
    pub fn handle_command(&mut self, now: Instant, cmd: UiCommand) -> CommandResult {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        let r = crate::ui::dispatch(self, &cmd);
        self.after();
        r
    }

    /// Handles a platform event.
    pub fn handle_event(&mut self, now: Instant, ev: PlatformEvent<'_>) {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        crate::ui::on_platform_event(self, &ev);
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_event(ctx, &ev));
        self.after();
    }

    /// Handles one inbound unit of `link` (stream chunk, ATT value or datagram).
    pub fn handle_packet(&mut self, now: Instant, link: LinkId, bytes: &[u8]) {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        let pts = self.ctx.links.inbound(now, link, bytes, &mut self.ctx.fx);
        self.process_link_events();
        for pt in pts {
            self.route_plaintext(link, &pt);
        }
        self.after();
    }

    /// Runs every timer that is due.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        let limits = self.ctx.cfg.limits;
        self.ctx.links.on_timer(now, &limits, &mut self.ctx.fx);
        self.process_link_events();
        let n = self.ctx.ble_links();
        self.ctx.radio.on_timer(now, n, &limits, &mut self.ctx.fx);
        if self.ctx.is_lab() {
            self.lab.on_timer(&mut self.ctx);
        }
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_timer(ctx));
        crate::diag::sample::tick(self);
        self.after();
    }

    /// The next instant `handle_timeout` must run.
    pub fn poll_timeout(&self) -> Option<Instant> {
        let limits = &self.ctx.cfg.limits;
        [
            self.ctx.links.next_deadline(limits),
            self.ctx.radio.next_deadline(limits, self.ctx.now),
            if self.ctx.is_lab() {
                self.lab.next_deadline(&self.ctx)
            } else {
                None
            },
            self.modules.next_timer(),
            self.ctx.ui.next_deadline(&self.ctx.cfg.limits),
            self.diag_deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Turns `ridelog/1` diagnostics on or off (off by default). While on, the engine
    /// emits `Effect::Diag` records: typed events at the decision points and a core
    /// sample on a 1 s grid. Never changes a decision of the engine.
    pub fn set_diag(&mut self, on: bool) {
        self.ctx.fx.set_diag(on);
        self.ctx.links.set_diag(on);
        self.ctx.radio.diag.set(on);
        self.ctx.diag = crate::diag::DiagTracker::default();
    }

    /// The diagnostics tick, while diagnostics are on in a session or Lab run.
    fn diag_deadline(&self) -> Option<Instant> {
        (self.ctx.fx.diag_on() && self.ctx.mode != EngineMode::Idle)
            .then(|| self.ctx.diag.next().unwrap_or(self.ctx.now))
    }

    /// Moves the diagnostics events buffered by the link table and the radio policy
    /// into the effect queue.
    fn drain_diag(&mut self) {
        for e in self.ctx.links.take_diag() {
            self.ctx.fx.diag(e);
        }
        for e in self.ctx.radio.diag.take() {
            self.ctx.fx.diag(e);
        }
    }

    /// The next unit to write, paced per link.
    pub fn poll_transmit(&mut self, now: Instant) -> Option<Transmit> {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        let t = self.ctx.links.poll_transmit(now, &mut self.ctx.fx);
        self.process_link_events();
        t
    }

    /// The next effect.
    pub fn poll_effect(&mut self) -> Option<Effect> {
        self.ctx.fx.pop()
    }

    /// 10 ms of capture at 16 kHz mono.
    pub fn push_capture(&mut self, now: Instant, pcm: &[i16; BLOCK_SAMPLES]) {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        if self.ctx.device.on_capture_block(pcm) {
            self.ctx.mark_dirty();
        }
        if self.ctx.is_lab() {
            self.lab.on_capture(&mut self.ctx, pcm);
        }
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_capture(ctx, pcm));
    }

    /// 10 ms of playout at 16 kHz mono.
    pub fn pull_playout(&mut self, now: Instant, out: &mut [i16; BLOCK_SAMPLES]) {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        out.fill(0);
        if self.ctx.is_lab() {
            self.lab.on_playout(&mut self.ctx, out);
        }
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_playout(ctx, out));
    }

    /// RT flags (PTT, mute, latch) mirrored from `mesh_rt_set_flags`. Bits 0-2 are
    /// levels: an unchanged value is not re-delivered. Bit3 (latch toggle) is an edge
    /// consumed by the runtime, so every call that carries it reaches the modules, even
    /// with the same value as the previous call (two lock-screen taps in consecutive
    /// ticks are two toggles).
    pub fn set_rt_flags(&mut self, flags: u32) {
        if flags == self.ctx.rt_flags && flags & crate::rt_flags::LATCH_TOGGLE == 0 {
            return;
        }
        self.ctx.rt_flags = flags;
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_rt_flags(ctx, flags));
    }

    /// The UI is attached (foreground WebView): gates snapshot emission (§3.4).
    pub fn set_ui_attached(&mut self, attached: bool) {
        self.ctx.ui.set_attached(attached);
        self.after();
    }

    /// The current Lab node id (Lab mode).
    pub fn lab_node_id(&self) -> Option<u16> {
        self.ctx.is_lab().then(|| self.lab.node_id())
    }

    /// Test and simulator hook: overrides the Lab node id and re-announces it (used to
    /// force id collisions).
    #[doc(hidden)]
    pub fn force_lab_node_id(&mut self, id: u16) {
        if self.ctx.is_lab() {
            self.lab.force_node_id(&mut self.ctx, id);
            self.after();
        }
    }

    /// Posts a media request from the runtime (e.g. `MediaRequest::DemoStart` at Demo
    /// start, §3.4) and runs it at once, as any engine call.
    pub fn media_request(&mut self, now: Instant, req: crate::media::MediaRequest) {
        self.ctx.now = now;
        self.ctx.links.diag_now = now;
        self.ctx.post_media(req);
        self.after();
    }

    /// Error answer helper used by the dispatcher.
    pub(crate) fn refuse(code: ErrorCode) -> CommandResult {
        CommandResult::error(code)
    }

    fn route_plaintext(&mut self, link: LinkId, pt: &[u8]) {
        let lab_link = self.ctx.links.get(link).is_some_and(|l| l.is_lab());
        if lab_link {
            if self.ctx.is_lab() {
                self.lab.on_plaintext(&mut self.ctx, link, pt);
                self.process_link_events();
            }
            return;
        }
        for rec in RecordIter::new(pt) {
            let Ok(rec) = rec else {
                break;
            };
            if self
                .ctx
                .links
                .on_link_record(self.ctx.now, link, &rec, &mut self.ctx.fx)
            {
                self.process_link_events();
                continue;
            }
            let ctx = &mut self.ctx;
            if let Record::Caps(c) = rec {
                self.modules.each(|m| m.on_caps(ctx, link, &c));
                continue;
            }
            self.modules
                .first(|m| m.on_record(ctx, link, &rec).then_some(()));
        }
    }

    /// Fans the link-table events out to the radio policy, Lab mode and the modules.
    pub(crate) fn process_link_events(&mut self) {
        self.drain_diag();
        loop {
            let events = self.ctx.links.drain_events();
            if events.is_empty() {
                break;
            }
            for ev in events {
                self.ctx.mark_dirty();
                match ev {
                    LinkEvent::Up(link) => {
                        let Some((info, lab)) =
                            self.ctx.links.get(link).map(|l| (l.info, l.is_lab()))
                        else {
                            continue;
                        };
                        self.ctx.radio.on_link_up(link, &info);
                        if lab {
                            self.lab.on_link_up(&mut self.ctx, link);
                        } else {
                            let ctx = &mut self.ctx;
                            self.modules.each(|m| m.on_link_up(ctx, link));
                        }
                    }
                    LinkEvent::Established(link) => {
                        let ctx = &mut self.ctx;
                        self.modules.each(|m| m.on_link_established(ctx, link));
                    }
                    LinkEvent::Down {
                        link,
                        reason,
                        peer,
                        lab,
                        ..
                    } => {
                        let limits = self.ctx.cfg.limits;
                        self.ctx
                            .radio
                            .on_link_down(self.ctx.now, link, reason, &limits);
                        if reason == LinkDownReason::LimitReached && !lab {
                            self.ctx.notice(NoticeKey::LinkLimitReached, Params::new());
                        }
                        if lab {
                            self.lab.on_link_down(&mut self.ctx, link, reason);
                        } else {
                            let ctx = &mut self.ctx;
                            self.modules
                                .each(|m| m.on_link_down(ctx, link, reason, peer));
                        }
                    }
                }
            }
            self.ctx.refresh_radio();
        }
        self.drain_diag();
    }

    /// Hands the inter-module bus to the modules until it is empty, at most
    /// [`MAX_BUS_PASSES`] times; what is left after that is dropped and logged.
    fn run_bus(&mut self) {
        for _ in 0..MAX_BUS_PASSES {
            if self.ctx.media_bus.is_empty() {
                return;
            }
            let ctx = &mut self.ctx;
            self.modules.each(|m| m.on_bus(ctx));
            self.process_link_events();
        }
        if !self.ctx.media_bus.is_empty() {
            let n = self.ctx.media_bus.len();
            self.ctx.media_bus.clear();
            self.ctx.fx.log(
                crate::effects::LogLevel::Warn,
                format!("engine: {n} media request(s) dropped after {MAX_BUS_PASSES} bus passes"),
            );
        }
    }

    /// Hands the packets originated during this call to the modules (the mesh glue takes
    /// them, [`CoreModule::on_originate`]).
    fn run_originate(&mut self) {
        if self.ctx.originated.is_empty() {
            return;
        }
        let ctx = &mut self.ctx;
        self.modules.each(|m| m.on_originate(ctx));
        self.process_link_events();
    }

    /// Queues what no module took on every Up group link directly (no mesh glue runs:
    /// Idle, no group identity yet). Also used by [`Engine::stop`] for the last packets.
    fn flush_originated_direct(&mut self) {
        for p in std::mem::take(&mut self.ctx.originated) {
            self.ctx.originate_now(&p);
        }
    }

    /// Post-call housekeeping: originated packets and the inter-module bus (each may
    /// feed the other: the glue tells the media module that my own link refused my
    /// burst), then snapshot emission (dirty flag, ≤ 10 Hz, attached only).
    fn after(&mut self) {
        self.process_link_events();
        for _ in 0..MAX_BUS_PASSES {
            self.run_originate();
            if self.ctx.media_bus.is_empty() {
                break;
            }
            self.run_bus();
        }
        self.run_originate();
        self.flush_originated_direct();
        if self.ctx.ui.snapshot_due(self.ctx.now, &self.ctx.cfg.limits) {
            let s = crate::ui::build_snapshot(&self.ctx, &self.lab, &self.modules);
            if let Some(s) = self.ctx.ui.offer(self.ctx.now, s) {
                self.ctx.fx.ui(UiEvent::Snapshot { s });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Platform, StoredGroup};
    use crate::group::state::Group;

    /// An engine holding one stored group (created with `salt`).
    fn engine(seed: u8, salt: u8) -> (Engine, [u8; 16]) {
        let mut rng = ChaCha20Rng::from_seed([salt; 32]);
        let g = Group::create("Ride", mesh_api::Vehicle::Moto, "Ana", 0, &mut rng);
        let mut cfg = EngineConfig::new(Platform::Ios, [seed; 32]);
        cfg.stored_groups = vec![StoredGroup {
            group_id: g.id,
            blob: crate::group::store::encode(&g),
        }];
        (Engine::new(cfg, Instant::ZERO), g.id)
    }

    #[test]
    fn the_shared_origin_counter_wraps_and_reserves_consecutive_seqs() {
        let (mut e, _) = engine(1, 9);
        let ctx = &mut e.ctx;
        ctx.mesh_seq = 0xFFFE;
        assert_eq!(ctx.next_mesh_seq(), 0xFFFE);
        assert_eq!(ctx.next_mesh_seq(), 0xFFFF);
        assert_eq!(
            ctx.next_mesh_seq(),
            0,
            "serial-number arithmetic: 65535 → 0"
        );
        // Segments of one body take consecutive seqs, across the wrap too.
        assert_eq!(ctx.reserve_mesh_seqs(3), 1);
        assert_eq!(ctx.next_mesh_seq(), 4);
        ctx.mesh_seq = 0xFFFF;
        assert_eq!(ctx.reserve_mesh_seqs(2), 0xFFFF);
        assert_eq!(ctx.next_mesh_seq(), 1);
    }

    #[test]
    fn every_group_start_draws_the_first_seq_from_the_injected_rng() {
        let start = |seed: u8| {
            let (mut e, gid) = engine(seed, 9);
            let group = SessionStart::Group { group_id: gid };
            e.start(Instant(1), group).expect("start");
            let first = (e.ctx.sess, e.ctx.mesh_seq);
            e.stop(Instant(2));
            e.start(Instant(3), group).expect("restart");
            (first, (e.ctx.sess, e.ctx.mesh_seq))
        };
        // Deterministic for a seed (the simulator replays runs byte for byte) …
        assert_eq!(start(1), start(1));
        // … and drawn afresh with `sess` at every start and for every seed.
        let ((s1, q1), (s2, q2)) = start(1);
        assert_ne!((s1, q1), (s2, q2), "a restart is a new (sess, seq) space");
        assert_ne!(start(1).0.1, start(2).0.1);
        // Lab mode keeps its own probe counter: the shared one is not drawn.
        let (mut e, _) = engine(1, 9);
        let before = e.ctx.mesh_seq;
        e.start(Instant(1), SessionStart::Lab { lab_code: 42 })
            .expect("lab");
        assert_eq!(e.ctx.mesh_seq, before);
    }
}
