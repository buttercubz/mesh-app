//! The media state of one engine ([`Media`]) and its construction. Behaviour lives in
//! the sibling files (`tx`, `rx`, `play`, `cmd`, `events`, `prompts`, `check`).

use std::collections::BTreeMap;

use mesh_api::{NoticeKey, PttStyle, TalkDeniedReason};
use mesh_routing::{Dedup, FloorTable};
use mesh_voice::{
    BLOCK_US, CaptureChain, Codec, Earcons, EnergyVad, Mixer, Packetizer, Profile, ProfileSelector,
    PromptBank, PromptPlayer, ReceivePath, RnnVad, SimCodec, StreamKey, TalkFsm, TalkMode, Vad,
    VadSensitivity,
};

use super::bus::MediaIdentity;
use super::check::CheckRun;
use super::latency::LatencyEstimator;
use super::outbox::Outbox;
use super::prompts::PromptState;
use crate::config::Platform;
use crate::engine::Ctx;
use crate::{Instant, LinkId};

/// Arrivals remembered per stream for the latency estimator.
pub(super) const ARRIVALS: usize = 32;
/// Receive streams tracked (= decoder pool size).
pub(super) const STREAMS: usize = mesh_voice::receive::POOL_SIZE;

/// One received frame waiting for its playout (latency estimator).
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Arrival {
    pub valid: bool,
    pub burst: u8,
    pub ts: u16,
    pub at: Instant,
    pub age_us: u64,
    pub dur_us: u64,
    pub hops: u8,
}

/// Per-stream playout tracking (latency, talker level).
#[derive(Debug, Clone, Copy)]
pub(super) struct StreamTrack {
    pub key: Option<StreamKey>,
    pub burst: Option<u8>,
    pub arrivals: [Arrival; ARRIVALS],
    pub next: usize,
    /// Burst timeline position (10 ms ticks) of the next block the jitter buffer plays.
    pub play_ts: Option<u32>,
    pub drops: u32,
    pub hops: u8,
    /// RMS (0..1) of the last block played.
    pub level: f32,
}

impl StreamTrack {
    pub const EMPTY: StreamTrack = StreamTrack {
        key: None,
        burst: None,
        arrivals: [Arrival {
            valid: false,
            burst: 0,
            ts: 0,
            at: Instant(0),
            age_us: 0,
            dur_us: 0,
            hops: 0,
        }; ARRIVALS],
        next: 0,
        play_ts: None,
        drops: 0,
        hops: 1,
        level: 0.0,
    };
}

/// The burst being transmitted.
#[derive(Debug, Clone, Copy)]
pub(super) struct BurstTx {
    pub burst: u8,
    pub mode: TalkMode,
    pub profile: Profile,
    pub started: Instant,
    pub next_keep: Instant,
}

/// A second TALK STOP due at `at`.
#[derive(Debug, Clone, Copy)]
pub(super) struct StopAgain {
    pub at: Instant,
    pub burst: u8,
    pub mode: TalkMode,
    pub profile: Profile,
    pub elapsed_10ms: u16,
}

/// Cue state derived from platform events.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PlatformCues {
    pub headset: Option<bool>,
    pub capture_lost: bool,
    pub radio_off: bool,
    pub next_bt_cue: Option<Instant>,
    pub last_battery_cue: Option<Instant>,
}

/// All media state of one engine.
pub(super) struct Media {
    pub codec: Option<Box<dyn Codec>>,
    pub lookahead_us: u64,
    pub identity: Option<MediaIdentity>,
    /// The identity was drawn locally (no session module in this build).
    pub provisional: bool,
    pub in_session: bool,
    pub live: bool,
    // Capture side.
    pub chain: CaptureChain,
    pub vox_base: VadSensitivity,
    pub vox_raised_until: Option<Instant>,
    pub fsm: TalkFsm,
    pub next_burst: u8,
    pub packetizer: Option<Packetizer>,
    pub selector: ProfileSelector,
    pub burst: Option<BurstTx>,
    pub stop_again: Option<StopAgain>,
    pub ptt_style: PttStyle,
    pub builtin_tx_allowed: bool,
    pub builtin_mute_flag: bool,
    pub last_reject: Option<TalkDeniedReason>,
    pub outbox: Outbox,
    // Receive side.
    pub dedup: Dedup,
    pub floor: FloorTable,
    pub recv: Option<ReceivePath>,
    pub streams: [StreamTrack; STREAMS],
    pub latency: LatencyEstimator,
    pub path_hint_until: Option<Instant>,
    pub loss_pct: BTreeMap<LinkId, f32>,
    // Playout side.
    pub earcons: Earcons,
    pub bank: PromptBank,
    pub player: PromptPlayer,
    pub mixer: Mixer,
    pub prompts: PromptState,
    pub check: Option<CheckRun>,
    pub names: BTreeMap<u16, String>,
    pub cues: PlatformCues,
    // UI events raised on the 10 ms paths, emitted from the timer path.
    pub pending_denied: Option<TalkDeniedReason>,
    pub pending_notices: [Option<NoticeKey>; 4],
    pub pending_cues: [Option<mesh_api::PromptKey>; 4],
    pub ui_dirty: bool,
    /// Time of the last hook call (`next_timer` has no `now`).
    pub last_now: Instant,
    /// Diagnostics (`ridelog/1`) state.
    pub diag: super::diag::MediaDiag,
}

/// The VAD of a platform: nnnoiseless on devices; the deterministic energy detector in
/// the browser simulator and host tests (§13.2 scripted talk schedules).
pub(super) fn vad_for(platform: Platform) -> Box<dyn Vad> {
    match platform {
        Platform::WebSim => Box::new(EnergyVad::new()),
        Platform::Ios | Platform::Android => Box::new(RnnVad::new()),
    }
}

/// The codec of a platform: libopus on devices (feature `opus`), SimCodec in the
/// browser simulator, host tests and builds without libopus.
pub(super) fn codec_for(platform: Platform) -> Box<dyn Codec> {
    #[cfg(feature = "opus")]
    if platform != Platform::WebSim {
        return Box::new(mesh_voice::OpusCodec::new());
    }
    let _ = platform;
    Box::new(SimCodec::new())
}

impl Media {
    pub fn new() -> Self {
        let lim = mesh_routing::Limits::V1;
        Media {
            codec: None,
            lookahead_us: 0,
            identity: None,
            provisional: false,
            in_session: false,
            live: false,
            chain: CaptureChain::new(Box::new(EnergyVad::new()), VadSensitivity::Medium),
            vox_base: VadSensitivity::Medium,
            vox_raised_until: None,
            fsm: TalkFsm::new(TalkMode::Vox, 0),
            next_burst: 0,
            packetizer: None,
            selector: ProfileSelector::new(),
            burst: None,
            stop_again: None,
            ptt_style: PttStyle::Hold,
            builtin_tx_allowed: false,
            builtin_mute_flag: false,
            last_reject: None,
            outbox: Outbox::new(),
            dedup: Dedup::new(),
            floor: FloorTable::new(0, lim.t_tie_us, lim.leader_priority),
            recv: None,
            streams: [StreamTrack::EMPTY; STREAMS],
            latency: LatencyEstimator::new(),
            path_hint_until: None,
            loss_pct: BTreeMap::new(),
            earcons: Earcons::new(),
            bank: PromptBank::new(),
            player: PromptPlayer::new(),
            mixer: Mixer::new(),
            prompts: PromptState::new(),
            check: None,
            names: BTreeMap::new(),
            cues: PlatformCues::default(),
            pending_denied: None,
            pending_notices: [None; 4],
            pending_cues: [None; 4],
            ui_dirty: false,
            last_now: Instant::ZERO,
            diag: super::diag::MediaDiag::default(),
        }
    }

    /// Builds the codec-dependent parts (once per engine; the platform is fixed).
    pub fn ensure_codec(&mut self, ctx: &Ctx) {
        if self.codec.is_some() {
            return;
        }
        let codec = codec_for(ctx.cfg.platform);
        let enc = codec.new_encoder().ok();
        self.lookahead_us = enc.as_ref().map_or(0, |e| {
            e.lookahead() as u64 * 1_000_000 / u64::from(mesh_voice::SAMPLE_RATE)
        });
        self.packetizer = enc.map(Packetizer::new);
        self.recv = ReceivePath::new(codec.as_ref()).ok();
        let sens = self.chain.sensitivity();
        self.chain = CaptureChain::new(vad_for(ctx.cfg.platform), sens);
        self.codec = Some(codec);
    }

    /// Enters a group session: fresh burst ids, floor and receive state (packet `seq`s
    /// come from the engine's shared per-origin counter).
    pub fn begin_session(&mut self, ctx: &mut Ctx) {
        self.ensure_codec(ctx);
        self.end_session();
        self.in_session = true;
        self.next_burst = (ctx.rand_u32() & 0xFF) as u8;
        let mode = self.fsm.mode();
        self.fsm = TalkFsm::new(mode, self.next_burst);
        self.earcons.reset_rate_limits();
    }

    /// Leaves the session: drops every per-session audio state (settings are kept).
    pub fn end_session(&mut self) {
        let lim = mesh_routing::Limits::V1;
        self.in_session = false;
        self.live = false;
        if self.provisional {
            self.identity = None;
            self.provisional = false;
        }
        if let Some(p) = self.packetizer.as_mut() {
            p.abort_burst();
        }
        self.burst = None;
        self.stop_again = None;
        self.outbox.clear();
        self.dedup = Dedup::new();
        let (epoch, prio) = self
            .identity
            .map_or((0, lim.leader_priority), |i| (i.epoch, i.leader_priority));
        self.floor = FloorTable::new(epoch, lim.t_tie_us, prio);
        if let Some(c) = self.codec.as_ref() {
            self.recv = ReceivePath::new(c.as_ref()).ok();
        }
        self.streams = [StreamTrack::EMPTY; STREAMS];
        self.latency.clear();
        self.path_hint_until = None;
        self.loss_pct.clear();
        self.earcons.clear();
        self.player.clear();
        self.mixer.reset();
        self.chain.reset();
        self.check = None;
        self.cues = PlatformCues::default();
        self.pending_denied = None;
        self.pending_notices = [None; 4];
        self.pending_cues = [None; 4];
    }

    /// My origin id, drawing a provisional one when no session module supplied it.
    pub fn identity(&mut self, ctx: &mut Ctx) -> MediaIdentity {
        if let Some(i) = self.identity {
            return i;
        }
        let node = loop {
            let n = (ctx.rand_u32() & 0xFFFF) as u16;
            if n != 0 {
                break n;
            }
        };
        let id = MediaIdentity {
            node,
            leader: false,
            epoch: 0,
            leader_priority: mesh_routing::Limits::V1.leader_priority,
        };
        self.identity = Some(id);
        self.provisional = true;
        id
    }

    /// Applies an identity from the session module.
    pub fn set_identity(&mut self, id: MediaIdentity) {
        self.identity = Some(id);
        self.provisional = false;
        self.floor.epoch = id.epoch;
        self.floor.leader_priority = id.leader_priority;
    }

    /// Active VOX sensitivity (raised for 2 min by wind-hog protection).
    pub fn apply_sensitivity(&mut self, now: Instant) {
        let s = match self.vox_raised_until {
            Some(t) if now < t => self.vox_base.raised(),
            _ => {
                self.vox_raised_until = None;
                self.vox_base
            }
        };
        if self.chain.sensitivity() != s {
            self.chain.set_sensitivity(s);
        }
    }

    /// Queues a notice raised on a 10 ms path.
    pub fn defer_notice(&mut self, key: NoticeKey) {
        if let Some(slot) = self.pending_notices.iter_mut().find(|s| s.is_none()) {
            *slot = Some(key);
        }
    }

    /// Queues a prompt raised on a 10 ms path (played from the timer path).
    pub fn defer_prompt(&mut self, key: mesh_api::PromptKey) {
        if let Some(slot) = self.pending_cues.iter_mut().find(|s| s.is_none()) {
            *slot = Some(key);
        }
    }

    /// Something deferred waits for the timer path.
    pub fn has_deferred(&self) -> bool {
        self.pending_denied.is_some()
            || self.pending_notices.iter().any(Option::is_some)
            || self.pending_cues.iter().any(Option::is_some)
            || self.ui_dirty
    }
}

/// Duration of one block, re-exported for the sibling files.
pub(super) const BLOCK: u64 = BLOCK_US;
