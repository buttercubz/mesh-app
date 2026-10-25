//! A single media module driven directly through its [`CoreModule`] hooks on a
//! hand-built [`Ctx`], for the media behaviour that the session module (WP-10B) starts
//! through the session → media seam ([`MediaRequest`]): the audio-check audio, cue and
//! prompt playback. Every 10 ms the capture block that ends at `t` is pushed, then the
//! output block is pulled (zeroed first, as the engine does), then the timer path runs.

#![allow(dead_code)]

use mesh_api::UiEvent;
use mesh_core::effects::{Effect, EffectQueue};
use mesh_core::links::{LinkTable, RadioPolicy};
use mesh_core::media::{MediaModule, MediaRequest};
use mesh_core::ui::{DeviceState, UiState};
use mesh_core::{
    BLOCK_SAMPLES, CoreModule, Ctx, EngineConfig, EngineMode, Instant, Platform, PlatformEvent,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

use super::world::{Block, Source};

/// The rig.
pub struct Rig {
    pub ctx: Ctx,
    pub media: MediaModule,
    pub source: Source,
    /// `(pull time, block)`.
    pub out: Vec<(u64, Block)>,
    pub effects: Vec<(u64, Effect)>,
}

impl Rig {
    /// A media module in Idle (no session), as for an audio check before any ride.
    pub fn new(platform: Platform, salt: u8) -> Rig {
        let seed = super::world::seed(0, salt);
        let cfg = EngineConfig::new(platform, seed);
        let now = Instant::ZERO;
        let ctx = Ctx {
            now,
            device: DeviceState::new(now, cfg.wall_ms),
            cfg,
            rng: ChaCha20Rng::from_seed(seed),
            mode: EngineMode::Idle,
            sess: 1,
            fx: EffectQueue::new(),
            links: LinkTable::new(),
            radio: RadioPolicy::new(platform),
            ui: UiState::new(),
            rt_flags: 0,
            media_bus: Vec::new(),
            media_latency: None,
            aware_backbone: std::collections::BTreeSet::new(),
            mesh_seq: 0,
            originated: Vec::new(),
            diag: mesh_core::diag::DiagTracker::default(),
        };
        Rig {
            ctx,
            media: MediaModule::new(),
            source: Source::Silence,
            out: Vec::new(),
            effects: Vec::new(),
        }
    }

    /// Current time, µs.
    pub fn now(&self) -> u64 {
        self.ctx.now.0
    }

    fn drain(&mut self) {
        let t = self.ctx.now.0;
        while let Some(e) = self.ctx.fx.pop() {
            self.effects.push((t, e));
        }
    }

    /// A session → media request at the current time.
    pub fn request(&mut self, req: MediaRequest) {
        let ctx = &mut self.ctx;
        self.media.request(ctx, req);
        self.media.on_timer(ctx);
        self.drain();
    }

    /// A UI command at the current time (as when no earlier module answered it).
    pub fn command(&mut self, cmd: mesh_api::UiCommand) -> Option<mesh_api::CommandResult> {
        let ctx = &mut self.ctx;
        let r = self.media.on_command(ctx, &cmd);
        self.media.on_timer(ctx);
        self.drain();
        r
    }

    /// Enters a group session (engine mode change) and goes live.
    pub fn go_live(&mut self, riders: u8) {
        let mode = EngineMode::Session { group_id: [7; 16] };
        self.ctx.mode = mode;
        let ctx = &mut self.ctx;
        self.media.on_mode(ctx, mode);
        self.request(MediaRequest::GoLive { riders });
    }

    /// Every render request so far.
    pub fn render_requests(&self) -> Vec<mesh_core::PromptRequest> {
        self.effects
            .iter()
            .filter_map(|(_, e)| match e {
                Effect::RenderPrompts(v) => Some(v.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// A platform event at the current time.
    pub fn event(&mut self, ev: PlatformEvent<'_>) {
        let ctx = &mut self.ctx;
        self.media.on_event(ctx, &ev);
        self.media.on_timer(ctx);
        self.drain();
    }

    /// Runs `ms` milliseconds of 10 ms audio ticks.
    pub fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms / 10 {
            let t = self.ctx.now.0 + 10_000;
            self.ctx.now = Instant(t);
            let block = self.source.block(t);
            let mut out = [0i16; BLOCK_SAMPLES];
            let ctx = &mut self.ctx;
            self.media.on_capture(ctx, &block);
            self.media.on_playout(ctx, &mut out);
            if self.media.next_timer().is_some_and(|d| d.0 <= t) {
                self.media.on_timer(ctx);
            }
            self.out.push((t, out));
            self.drain();
        }
    }

    /// UI events with their time.
    pub fn ui_events(&self) -> Vec<(u64, UiEvent)> {
        self.effects
            .iter()
            .filter_map(|(t, e)| match e {
                Effect::Ui(ev) => Some((*t, ev.clone())),
                _ => None,
            })
            .collect()
    }
}
