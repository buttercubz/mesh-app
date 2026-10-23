//! [`MediaModule`]: the `CoreModule` registration of the media state (`wire/media.rs`),
//! plus the timer path that turns what the 10 ms paths queued into records and UI
//! events.

use mesh_api::{CommandResult, Params, PromptKey, Snapshot, UiCommand, UiEvent};
use mesh_wire::record::Record;

use super::LATENCY_EXPORT_US;
use super::bus::MediaRequest;
use super::latency::LatencyEstimator;
use super::state::Media;
use crate::engine::{CoreModule, Ctx, EngineMode};
use crate::links::LinkDownReason;
use crate::{BLOCK_SAMPLES, Instant, LinkId, NodeId, PlatformEvent};

/// The media module.
pub struct MediaModule {
    m: Media,
    rt_prev: u32,
    /// Next refresh of `Ctx::media_latency` (in a session only). Not a deadline of its
    /// own: the runtime runs the timer path at least every engine tick (≤ 5 ms, §3.4),
    /// and simulations get no extra wakeup per second.
    next_latency: Option<Instant>,
}

impl Default for MediaModule {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaModule {
    pub fn new() -> Self {
        MediaModule {
            m: Media::new(),
            rt_prev: 0,
            next_latency: None,
        }
    }

    /// Applies a session request (the session → media seam, see [`MediaRequest`]).
    pub fn request(&mut self, ctx: &mut Ctx, req: MediaRequest) {
        self.m.last_now = ctx.now;
        self.m.request(ctx, req);
    }

    /// The latency estimator (diagnostics, ride recorder).
    pub fn latency(&self) -> &LatencyEstimator {
        &self.m.latency
    }

    /// Bytes of rendered prompt PCM held in the prompt bank (≤ 512 KB, §7.7).
    pub fn prompt_bank_bytes(&self) -> usize {
        self.m.bank.used_bytes()
    }

    /// `true` while capture feeds the talk path (Go live done).
    pub fn live(&self) -> bool {
        self.m.live
    }

    /// Applies every request pending on the engine's inter-module bus, in posting order.
    fn drain_bus(&mut self, ctx: &mut Ctx) {
        if ctx.media_bus.is_empty() {
            return;
        }
        for req in std::mem::take(&mut ctx.media_bus) {
            self.request(ctx, req);
        }
        // The snapshot of this engine call already shows the change.
        if self.m.ui_dirty {
            self.m.ui_dirty = false;
            ctx.mark_dirty();
        }
        self.sync_latency(ctx);
    }

    /// The 1 Hz latency export into `Ctx::media_latency` (ride recorder, §12.6) while a
    /// session runs, refreshed from the timer path once a period has passed; cleared
    /// outside a session.
    fn sync_latency(&mut self, ctx: &mut Ctx) {
        if !self.m.in_session {
            self.next_latency = None;
            ctx.media_latency = None;
            return;
        }
        match self.next_latency {
            None => self.next_latency = Some(ctx.now.plus_us(LATENCY_EXPORT_US)),
            Some(t) if ctx.now >= t => {
                ctx.media_latency = Some(self.m.latency.export());
                self.next_latency = Some(ctx.now.plus_us(LATENCY_EXPORT_US));
            }
            Some(_) => {}
        }
    }

    /// The timer path.
    fn timer(&mut self, ctx: &mut Ctx) {
        let m = &mut self.m;
        m.last_now = ctx.now;
        m.diag_timer(ctx);
        if m.in_session {
            m.apply_sensitivity(ctx.now);
            m.tx_timer(ctx);
            m.floor_timer(ctx);
            m.update_loss(ctx);
            m.platform_timer(ctx);
        }
        if let Some(reason) = m.pending_denied.take() {
            ctx.ui_event(UiEvent::TalkDenied { reason });
        }
        for k in m.pending_notices.iter_mut().filter_map(Option::take) {
            ctx.notice(k, Params::new());
        }
        let prompts: Vec<PromptKey> = m.pending_cues.iter_mut().filter_map(Option::take).collect();
        for k in prompts {
            // MAX_BURST already played its busy tone: no second fallback earcon.
            m.speak(ctx, k, None, k != PromptKey::TalkLimit);
        }
        if let Some(c) = m.check.as_mut() {
            let route = ctx.device.route_kind();
            let rate = ctx.device.route.input_rate;
            let evs: Vec<UiEvent> = c.drain_events(route, rate).collect();
            for ev in evs {
                ctx.ui_event(ev);
            }
        }
        if m.ui_dirty {
            m.ui_dirty = false;
            ctx.mark_dirty();
        }
        if !m.outbox.is_empty() {
            match m.identity {
                Some(me) if m.in_session => {
                    m.outbox.flush(ctx, me);
                }
                _ => m.outbox.clear(),
            }
        }
        self.sync_latency(ctx);
    }
}

impl CoreModule for MediaModule {
    fn name(&self) -> &'static str {
        "media"
    }

    fn on_mode(&mut self, ctx: &mut Ctx, mode: EngineMode) {
        self.m.last_now = ctx.now;
        match mode {
            EngineMode::Session { .. } => self.m.begin_session(ctx),
            EngineMode::Idle | EngineMode::Lab { .. } => self.m.end_session(),
        }
        // A new session starts a fresh export one period from now.
        self.next_latency = None;
        ctx.media_latency = None;
        self.sync_latency(ctx);
    }

    fn on_link_up(&mut self, ctx: &mut Ctx, _link: LinkId) {
        self.m.last_now = ctx.now;
        self.m.ui_dirty = true;
    }

    fn on_link_down(
        &mut self,
        ctx: &mut Ctx,
        link: LinkId,
        _reason: LinkDownReason,
        _peer: Option<NodeId>,
    ) {
        self.m.last_now = ctx.now;
        self.m.loss_pct.remove(&link);
        self.m.ui_dirty = true;
    }

    fn on_record(&mut self, ctx: &mut Ctx, link: LinkId, rec: &Record<'_>) -> bool {
        self.m.last_now = ctx.now;
        if let Record::Mesh(body) = rec {
            self.m.on_mesh(ctx, link, body);
        }
        // Never consumed: the mesh glue (WP-10D) still relays what it must.
        false
    }

    fn on_timer(&mut self, ctx: &mut Ctx) {
        self.timer(ctx);
    }

    fn next_timer(&self) -> Option<Instant> {
        let m = &self.m;
        let pending = !m.outbox.is_empty()
            || m.has_deferred()
            || m.check.as_ref().is_some_and(|c| c.has_pending());
        if pending {
            return Some(m.last_now);
        }
        if !m.in_session {
            return None;
        }
        let floor = m
            .floor
            .entries()
            .iter()
            .map(|e| Instant(e.last_packet.0).plus_us(mesh_routing::consts::FLOOR_IDLE_RELEASE_US))
            .min();
        [m.tx_deadline(), floor, m.cues.next_bt_cue]
            .into_iter()
            .flatten()
            .min()
    }

    fn on_command(&mut self, ctx: &mut Ctx, cmd: &UiCommand) -> Option<CommandResult> {
        self.m.last_now = ctx.now;
        self.m.on_command(ctx, cmd)
    }

    fn on_event(&mut self, ctx: &mut Ctx, ev: &PlatformEvent<'_>) {
        self.m.last_now = ctx.now;
        self.m.on_event(ctx, ev);
    }

    fn on_capture(&mut self, ctx: &mut Ctx, pcm: &[i16; BLOCK_SAMPLES]) {
        self.m.last_now = ctx.now;
        self.m.on_capture(ctx, pcm);
    }

    fn on_playout(&mut self, ctx: &mut Ctx, out: &mut [i16; BLOCK_SAMPLES]) {
        self.m.last_now = ctx.now;
        self.m.on_playout(ctx, out);
    }

    fn on_bus(&mut self, ctx: &mut Ctx) {
        self.m.last_now = ctx.now;
        self.drain_bus(ctx);
    }

    fn on_rt_flags(&mut self, ctx: &mut Ctx, flags: u32) {
        let prev = self.rt_prev;
        self.rt_prev = flags;
        self.m.on_rt_flags(ctx, prev, flags);
    }

    fn fill_snapshot(&self, ctx: &Ctx, s: &mut Snapshot) {
        self.m.fill_snapshot(ctx, s);
    }

    fn fill_diag(&mut self, ctx: &mut Ctx, s: &mut mesh_lab::ridelog::CoreSample) {
        self.m.fill_diag(ctx, s);
    }
}
