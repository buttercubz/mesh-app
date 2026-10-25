//! The `media` module of mesh-core (DESIGN §3.3, owned by WP-10C): the capture chain
//! and the receive chain of a group session, wired between the sans-IO engine and
//! mesh-voice / mesh-routing.
//!
//! ```text
//! capture (push_capture, 10 ms) ─► CaptureChain (wind guard, HPF, VAD + gate)
//!   ─► TalkFsm (VOX/PTT, RT flags incl. bit3 latch, hard mute, built-in-mic mute)
//!   ─► profile (own links, PATH_HINT, capture route, thermal) ─► Packetizer (pre-roll,
//!      FIRST, burst-relative ts) ─► outbox ─► TALK / VOICE MeshPackets on every Up link
//!
//! MESH records ─► dedup ─► FloorTable (rank, near-talker rejection, FLOOR_REJECT)
//!   ─► ReceivePath (jitter buffer per (origin, sess), decoder pool of 6, rank order)
//!   ─► mixer (≤ 4 by floor rank) + earcons + prompts + audio check ─► limiter
//!      (pull_playout, 10 ms) ─► latency estimator (age-based, per origin and hops)
//! ```
//!
//! Neither `push_capture` nor `pull_playout` allocates in steady state: encoded frames go
//! to a fixed ring ([`outbox`]) that the timer path turns into records, and UI events
//! raised on the 10 ms paths are deferred the same way.
//!
//! **Module seam.** The session module (WP-10B) owns go-live, talk mode, the audio-check
//! lifecycle, reachability and the group identity; this module owns the audio. They meet
//! through [`MediaRequest`]s posted on the engine's inter-module bus
//! (`Ctx::post_media`, drained here in `CoreModule::on_bus` within the same engine
//! call). This module also answers `goLive`, `resumeMic`, `setTalkMode` and `audioCheck`
//! itself when no earlier module did, with a provisional random origin id, so a build
//! without the session module still rides.
//!
//! The latency estimator's export is published at 1 Hz in `Ctx::media_latency` for the
//! ride recorder while a group session runs.

mod bus;
mod check;
mod cmd;
pub mod demo;
mod diag;
mod events;
mod inputs;
pub mod latency;
mod module;
mod outbox;
mod play;
mod prompts;
mod rx;
mod snapshot;
mod state;
mod tx;

pub use bus::{Cue, MediaIdentity, MediaRequest};
pub use latency::{LatencyEstimator, LatencySample};
pub use module::MediaModule;
pub use prompts::{fill_template, prompt_id};

/// Second TALK STOP after PTT up / MAX_BURST (§6.4: "sent twice, 60 ms apart").
pub const STOP_REPEAT_US: u64 = 60_000;
/// TALK KEEP period during a burst (§6.1).
pub const TALK_KEEP_US: u64 = mesh_routing::consts::TALK_KEEP_US;
/// Wind-hog protection: the VOX threshold stays raised this long (§6.5).
pub const VOX_RAISE_US: u64 = 120_000_000;
/// A PATH_HINT asking for BLE12 holds this long (§5.6, Appendix A).
pub const PATH_HINT_HOLD_US: u64 = mesh_routing::consts::PATH_HINT_HOLD_US;
/// Render buffering added by the latency estimator: the render ring target (§3.6, 320
/// samples = 20 ms). [TUNE: the runtime may report its real fill in a later ABI.]
pub const RENDER_BUFFER_US: u64 = 20_000;
/// Engine tick alignment added by the latency estimator: half the 5 ms engine tick, the
/// average residence of a capture block in the ring before the engine drains it (§7.9).
pub const TICK_ALIGN_US: u64 = 2_500;
/// Battery level below which the `batteryLow` prompt plays (once per 10 min).
pub const BATTERY_LOW: f64 = 0.15;
/// Spacing of the `batteryLow` cue.
pub const BATTERY_CUE_INTERVAL_US: u64 = 600_000_000;
/// Bluetooth-off cue repeat while the radio stays off (§6.5).
pub const BLUETOOTH_CUE_REPEAT_US: u64 = 60_000_000;
/// Period of the latency export into `Ctx::media_latency` (ride recorder, §12.6).
pub const LATENCY_EXPORT_US: u64 = 1_000_000;
