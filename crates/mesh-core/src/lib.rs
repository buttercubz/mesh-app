//! Sans-IO engine (contract C3, DESIGN §3.2-§3.3): links, group, session, media, mesh
//! glue, Lab mode and UI dispatch.
//!
//! The [`Engine`] owns no thread, clock, socket or radio. The runtime (mesh-ffi,
//! mesh-wasm, mesh-sim) feeds it time ([`Instant`], µs), UI commands, platform events
//! and link bytes, and drains what it decided: [`Transmit`] units, [`Effect`]s
//! (radio, audio, UI, persistence, notifications, Lab files, prompts, logs) and the
//! next deadline.
//!
//! Module map (DESIGN §3.3): `engine`, `effects`, `config`, `wire`, `links`, `lab` and
//! `ui` belong to WP-10A; `secure`, `group`, `session` (WP-10B), `media` (WP-10C) and
//! `mesh` (WP-10D) plug in through [`CoreModule`] registrations in `wire/<module>.rs`.
//!
//! Sans-IO crate: the determinism lints below are denied here and only here (clippy.toml
//! holds the list; the workspace allows them for every other crate).
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]

pub mod config;
pub mod diag;
pub mod effects;
pub mod engine;
pub mod lab;
pub mod links;
pub mod ui;
pub mod wire;

// Sub-WP module directories (stub hand-off, DESIGN §16.2): WP-10B owns `secure`,
// `group` and `session`, WP-10C `media`, WP-10D `mesh`.
pub mod group;
pub mod media;
pub mod mesh;
pub mod secure;
pub mod session;

pub use config::{DeviceDescriptor, EngineConfig, Limits, Platform, SessionStart, StoredGroup};
pub use effects::{
    AudioCommand, AudioSessionKind, AudioSessionProfile, DiscoveryMode, Effect, LabFile,
    LinkPriority, LogLevel, Notification, OpenVia, PromptRequest, RadioCommand, ScanIntensity,
    Transmit,
};
pub use engine::{CoreModule, Ctx, Engine, EngineMode, ModuleRegistry, OriginPacket, StartError};
pub use links::{
    AdvData, Discovery, LinkDownReason, LinkInfo, LinkKind, LinkRole, LinkSecurity, LinkState,
    LinkUnit, Plain, RadioAvail, SecurityError, SecurityStep, Transport,
};

pub use mesh_api::{AudioRouteKind, PromptKey};

/// Monotonic time in microseconds, supplied by the caller (DESIGN §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Instant(pub u64);

impl Instant {
    /// Time zero.
    pub const ZERO: Instant = Instant(0);

    /// `ms` milliseconds after time zero.
    pub const fn from_ms(ms: u64) -> Self {
        Instant(ms.saturating_mul(1_000))
    }

    /// Microseconds.
    pub const fn us(self) -> u64 {
        self.0
    }

    /// `self + us`, saturating.
    pub const fn plus_us(self, us: u64) -> Self {
        Instant(self.0.saturating_add(us))
    }

    /// `self + ms`, saturating.
    pub const fn plus_ms(self, ms: u64) -> Self {
        self.plus_us(ms.saturating_mul(1_000))
    }

    /// Microseconds since `earlier` (0 when `earlier` is later).
    pub const fn since(self, earlier: Instant) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// The same instant for mesh-lab.
    pub const fn lab(self) -> mesh_lab::Instant {
        mesh_lab::Instant(self.0)
    }

    /// The same instant for mesh-routing.
    pub const fn routing(self) -> mesh_routing::Instant {
        mesh_routing::Instant(self.0)
    }
}

/// A link id assigned by the platform (unique while the link exists).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LinkId(pub u32);

/// A mesh node id (per-group `node_id`, or the per-run `lab_node_id` in Lab mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u16);

/// RT flag bits mirrored from `mesh_rt_set_flags` (DESIGN §3.6).
pub mod rt_flags {
    /// PTT held down.
    pub const PTT_DOWN: u32 = 1 << 0;
    /// Hard mute.
    pub const HARD_MUTE: u32 = 1 << 1;
    /// Motorcycle built-in mic mute.
    pub const BUILTIN_MIC_MUTE: u32 = 1 << 2;
    /// PTT latch toggled (edge).
    pub const LATCH_TOGGLE: u32 = 1 << 3;
}

/// Current audio route as reported by the platform (§3.2 `AudioRoute`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioRouteInfo {
    /// Route kind of the UI contract.
    pub kind: AudioRouteKind,
    /// Hardware input sample rate in Hz (8k/16k/32k/48k); 0 when no input.
    pub input_rate: u32,
    /// Output route kind (differs from `kind` for split routes, e.g. HFP in, speaker out).
    pub output: AudioRouteKind,
    /// A headset (HFP, LE Audio, wired, car) is connected.
    pub headset_present: bool,
}

impl AudioRouteInfo {
    /// No route (before the first report).
    pub const NONE: AudioRouteInfo = AudioRouteInfo {
        kind: AudioRouteKind::None,
        input_rate: 0,
        output: AudioRouteKind::None,
        headset_present: false,
    };
}

/// Why the audio IO was interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptReason {
    /// Phone or VoIP call.
    Call,
    /// Siri / assistant.
    Assistant,
    /// Another app took the session.
    OtherApp,
    /// Route change (headset off).
    RouteChange,
    /// Media services reset.
    MediaReset,
    /// Anything else.
    Other,
}

/// Audio IO state (§3.2 `AudioIo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioIoState {
    /// Capture and render running.
    Running,
    /// Interrupted by the OS.
    Interrupted { reason: InterruptReason },
    /// The bridge restarts the IO.
    Restarting,
    /// Capture is lost (render may still run).
    CaptureLost,
    /// Render is lost.
    RenderLost,
    /// Restored after a loss.
    Restored,
    /// No session open.
    Stopped,
}

/// A GPS fix (only with consent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpsFix {
    pub lat: f64,
    pub lon: f64,
    /// Speed in m/s.
    pub speed_mps: Option<f64>,
    /// Horizontal accuracy in metres.
    pub acc_m: Option<f64>,
    /// Course over ground in degrees.
    pub heading_deg: Option<f64>,
}

/// Thermal state (iOS `ProcessInfo.thermalState`, Android `PowerManager` status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thermal {
    Nominal,
    Fair,
    Serious,
    Critical,
}

/// Keep-alive anchor state (§12.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnchorState {
    /// Audio session active (VPIO running / FGS up).
    pub session: Option<bool>,
    /// Location anchor active (CLBackgroundActivitySession / FGS location type).
    pub location: Option<bool>,
    /// Live Activity active (iOS).
    pub live_activity: Option<bool>,
}

/// RT audio counters derived from `mesh_rt_stats` by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AudioCounters {
    /// Capture frames per second over the last second.
    pub capture_fps: Option<f64>,
    /// Render frames per second over the last second.
    pub render_fps: Option<f64>,
    /// Overruns + underruns since the session opened.
    pub xruns: u32,
}

/// 1 Hz platform sample (§3.2 `Platform`, §12.1 "Platform samples").
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlatformSample {
    /// Wall clock (Unix ms) at the sample, used for run ids and `startedAt`.
    pub wall_ms: Option<u64>,
    pub gps: Option<GpsFix>,
    /// Battery level 0..1.
    pub battery: Option<f64>,
    pub charging: Option<bool>,
    pub thermal: Option<Thermal>,
    /// App in background.
    pub background: bool,
    /// Device locked (protected data unavailable).
    pub locked: Option<bool>,
    /// A phone call is active.
    pub on_call: bool,
    pub audio: AudioCounters,
    pub anchor: AnchorState,
}

/// Every platform → engine event (§3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum PlatformEvent<'a> {
    /// A link came up.
    Up { link: LinkId, info: LinkInfo },
    /// A link went down.
    Down {
        link: LinkId,
        reason: LinkDownReason,
    },
    /// The link accepts writes again.
    Writable { link: LinkId },
    /// The platform refused `unit` (WouldBlock); it is re-sent first on `Writable`.
    WouldBlock { link: LinkId, unit: &'a [u8] },
    /// Radio metrics of a link.
    Metrics {
        link: LinkId,
        rssi: Option<i16>,
        att_mtu: Option<u16>,
    },
    /// A peer was discovered (advertising).
    Discovered(Discovery),
    /// The HELLO characteristic of `disc_id` was read.
    HelloRead { disc_id: u32, bytes: &'a [u8] },
    /// A connect attempt failed before any link existed.
    ConnectFailed {
        disc_id: u32,
        reason: LinkDownReason,
    },
    /// The local L2CAP channel was published with `psm` (0 = GATT only).
    L2capPublished { psm: u16 },
    /// Audio route.
    AudioRoute(AudioRouteInfo),
    /// Audio IO state.
    AudioIo(AudioIoState),
    /// 1 Hz platform sample.
    Platform(PlatformSample),
    /// Radio availability.
    RadioState { ble: RadioAvail, aware: RadioAvail },
    /// A rendered spoken prompt (16 kHz mono, §7.7).
    PromptPcm { key: PromptKey, pcm: &'a [i16] },
}

/// Samples per 10 ms RT block at 16 kHz mono.
pub const BLOCK_SAMPLES: usize = 160;
