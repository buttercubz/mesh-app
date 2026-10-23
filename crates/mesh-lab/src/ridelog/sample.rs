//! Inputs of the 1 Hz `s` line. Every counter here is **cumulative** (monotonic since
//! the producer started it); the [`super::Sampler`] turns them into per-second deltas,
//! so producers (mesh-core, the RT plane, platform code) only keep plain totals.

use super::event::FrameType;
use crate::kinds::{LinkKind, LinkRole};

/// Per-direction frame counters of one link (cumulative).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirCounters {
    pub frames: u64,
    pub bytes: u64,
    /// Frames that failed to decode or authenticate (rx only).
    pub bad: u64,
    /// Frames per [`FrameType`] (indexed by [`FrameType::index`]).
    pub by_type: [u64; FrameType::COUNT],
    /// Bytes per [`FrameType`] (on-air size of the frames counted in `by_type`).
    pub bytes_by_type: [u64; FrameType::COUNT],
}

impl DirCounters {
    /// Counts one frame of `len` bytes.
    pub fn count(&mut self, ty: FrameType, len: usize) {
        self.frames = self.frames.saturating_add(1);
        self.bytes = self.bytes.saturating_add(len as u64);
        let slot = &mut self.by_type[ty.index()];
        *slot = slot.saturating_add(1);
        let b = &mut self.bytes_by_type[ty.index()];
        *b = b.saturating_add(len as u64);
    }
}

/// Send-queue depths of one link (gauges, units queued per class).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueDepth {
    pub voice: u16,
    pub ctrl: u16,
    pub bulk: u16,
    /// Units handed to the platform and not yet confirmed writable.
    pub units: u16,
}

/// One link as the core sees it at sample time.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreLink {
    pub id: u32,
    pub peer: Option<u16>,
    pub kind: LinkKind,
    pub role: Option<LinkRole>,
    /// `up`, `handshake`, `closing`.
    pub state: &'static str,
    pub rssi: Option<i16>,
    pub mtu: Option<u16>,
    pub rtt_p50_ms: Option<f32>,
    pub rtt_p95_ms: Option<f32>,
    /// Loss ratio estimate (0-1).
    pub loss: Option<f32>,
    pub grant_kbps: Option<f32>,
    pub q: QueueDepth,
    /// WouldBlock answers (cumulative).
    pub would_block: u64,
    pub tx: DirCounters,
    pub rx: DirCounters,
}

/// Mesh counters (cumulative) and gauges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshCounters {
    pub nbrs: u16,
    pub reach: u16,
    pub islands: u16,
    pub fwd: u64,
    pub dup: u64,
    pub suppressed: u64,
    pub too_old: u64,
    pub pruned: u64,
    pub floor_refused: u64,
    pub voice_dropped: u64,
    pub bursts_shed: u64,
    pub own_refused: u64,
}

/// Jitter-buffer counters (cumulative), mirroring mesh-voice `JitterStats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JbCounters {
    pub received: u64,
    pub late: u64,
    pub duplicates: u64,
    pub fec_recovered: u64,
    pub concealed: u64,
    pub expanded: u64,
    pub dropped: u64,
}

/// One received stream (per origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoreJb {
    pub origin: u16,
    pub target_ms: u16,
    pub depth_ms: u16,
    pub owd_p95_ms: Option<f32>,
    pub counters: JbCounters,
}

/// Audio state the core decides (gauges and the encoded-frame total).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoreAudio {
    pub route: &'static str,
    pub ladder: &'static str,
    pub vad: bool,
    pub ptt: bool,
    pub muted: bool,
    /// Voice frames encoded and sent (cumulative).
    pub tx_frames: u64,
    pub cap_dbfs: Option<f32>,
    pub ren_dbfs: Option<f32>,
}

/// What the core contributes to the 1 Hz sample (`Effect::Diag(DiagRecord::Sample)`).
#[derive(Debug, Clone, PartialEq)]
pub struct CoreSample {
    pub links: Vec<CoreLink>,
    pub mesh: Option<MeshCounters>,
    /// Active talkers and the local floor state (`idle/talking/queued/refused`).
    pub floor: Option<(u8, &'static str)>,
    pub audio: Option<CoreAudio>,
    pub jb: Vec<CoreJb>,
    /// `ReceivePath::totals()` (all streams, retired ones included).
    pub jb_total: Option<JbCounters>,
    /// Distance to the farthest peer, only when distance sharing is on (rounded to 10 m).
    pub dist_m: Option<u32>,
    /// `Diag` effects the core's queue dropped (cumulative).
    pub dropped_diag: u64,
}

/// RT plane counters (cumulative since the RT plane was activated), as read by the
/// runtime from `RtStats` once per second.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RtCounters {
    pub running: bool,
    pub captured_frames: u64,
    pub rendered_frames: u64,
    pub capture_overruns: u64,
    pub render_underruns: u64,
    pub render_underrun_samples: u64,
    pub capture_overrun_samples: u64,
    /// Largest callback sizes since activation (samples).
    pub max_render_frames: u32,
    pub max_capture_frames: u32,
    /// Render ring refill target (samples, gauge).
    pub render_target: u32,
    /// `[min, max]` capture / render callback sizes seen since the previous read
    /// (samples; `None` when no callback ran).
    pub cb_capture: Option<(u32, u32)>,
    pub cb_render: Option<(u32, u32)>,
    pub capture_rms_dbfs: Option<f32>,
    pub render_rms_dbfs: Option<f32>,
}

/// Device and anchor state from the platform's 1 Hz sample (gauges).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlatDiag {
    pub bat: Option<f32>,
    pub chg: Option<bool>,
    pub therm: Option<&'static str>,
    pub app: Option<&'static str>,
    pub locked: Option<bool>,
    pub call: Option<bool>,
    pub bt: Option<&'static str>,
    pub aware: Option<&'static str>,
    pub anchor_session: Option<bool>,
    pub anchor_location: Option<bool>,
}
