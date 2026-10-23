//! `labrun/1` records (DESIGN §12.3, contract C7): one JSON object per line, one file per
//! device per run. The schema is `crates/mesh-lab/schema/labrun-1.schema.json`; every
//! line produced here validates against it (tests/schema.rs).

use crate::json::Json;
use crate::kinds::{BtState, LinkKind, LinkRole, RunRole, Verdict};
use crate::params::RunParams;

/// `labrun/1` version (the `v` of the `run` line).
pub const LABRUN_VERSION: u8 = 1;
/// Decimals of line timestamps (`ts`, seconds since run start): 1 ms.
pub const LINE_TS_DECIMALS: u32 = 3;

/// Device platform in the `run` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Ios,
    Android,
    /// Browser fake / WASM simulator.
    Web,
    /// mesh-sim world.
    Sim,
}

impl Platform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Android => "android",
            Platform::Web => "web",
            Platform::Sim => "sim",
        }
    }
}

/// The device that wrote the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Hardware model id, e.g. `iPhone15,2`.
    pub model: String,
    /// OS name and version, e.g. `iOS 27.0`.
    pub os: String,
    /// App version and build, e.g. `0.1.0 (12)`.
    pub app: String,
    pub platform: Platform,
}

/// A Lab peer known at run start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub lab_node: u16,
    pub model: Option<String>,
}

/// The first line of every file.
#[derive(Debug, Clone, PartialEq)]
pub struct RunLine {
    /// `r-YYYYMMDD-HHMM-xxxx`.
    pub run_id: String,
    /// Variant values (`transport`, `role`, `headset`, `pattern`, …).
    pub variant: Vec<(String, Json)>,
    /// ISO 8601 UTC start time stamped by the runtime.
    pub started_at: String,
    pub device: DeviceInfo,
    pub lab_node: u16,
    pub role: RunRole,
    pub peers: Vec<PeerInfo>,
    pub params: RunParams,
}

/// One Lab link in a 1 Hz sample.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkSample {
    pub id: u32,
    pub kind: LinkKind,
    pub role: Option<LinkRole>,
    /// Peer `lab_node_id`, when announced.
    pub peer: Option<u16>,
    pub rssi: Option<i16>,
    pub tx_kbps: f64,
    pub rx_kbps: f64,
    pub loss: f64,
    pub reorder: u32,
    pub rtt_p50: Option<f64>,
    pub rtt_p95: Option<f64>,
    pub owd_p95: Option<f64>,
    pub stalls: u32,
    /// ATT max write length (GATT links), else `None`.
    pub max_write: Option<u16>,
}

/// A GPS fix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpsFix {
    pub lat: f64,
    pub lon: f64,
    /// Speed in m/s.
    pub spd: Option<f64>,
    /// Horizontal accuracy in m.
    pub acc: Option<f64>,
}

/// Audio IO state (`mesh_rt_stats`, route, ladder).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSample {
    pub cap_fps: Option<f64>,
    pub ren_fps: Option<f64>,
    pub running: bool,
    /// Route id, e.g. `hfpWideband`, `builtin`, `none`.
    pub route: String,
    /// Degradation ladder `L0`…`L3`.
    pub ladder: String,
    pub xruns: u32,
}

/// Device state.
#[derive(Debug, Clone, PartialEq)]
pub struct DevSample {
    /// Battery 0..1.
    pub bat: Option<f64>,
    pub chg: Option<bool>,
    /// Thermal state (`nominal`, `fair`, `serious`, `critical`, Android statuses, `unknown`).
    pub therm: String,
    /// `foreground`, `background` or `inactive`.
    pub app: String,
    pub locked: Option<bool>,
}

/// Keep-alive anchor state (§12.1): session, location session, Live Activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorSample {
    /// The run's anchor holds (`None` = the platform did not report this second).
    pub ok: Option<bool>,
    pub session: Option<bool>,
    pub location: Option<bool>,
    pub live_activity: Option<bool>,
}

/// The 1 Hz sample line.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleLine {
    /// Seconds since run start.
    pub ts: f64,
    pub links: Vec<LinkSample>,
    pub gps: Option<GpsFix>,
    pub peer_gps: Vec<(u16, GpsFix)>,
    pub dist_m: Option<f64>,
    pub audio: AudioSample,
    pub dev: DevSample,
    pub anchor: AnchorSample,
    pub bt: BtState,
}

/// One ping exchange for offline re-analysis: `[probe_id, t1, t4, hold, t3_remote]`
/// (t1/t4 in run seconds, hold in seconds, t3 in the remote's raw clock seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawPing {
    pub probe_id: u32,
    pub t1_s: f64,
    pub t4_s: f64,
    pub hold_s: f64,
    pub t3_remote_s: Option<f64>,
}

/// A received MESH_PROBE (multi-hop latency).
#[derive(Debug, Clone, PartialEq)]
pub struct RawMeshProbe {
    pub origin: u16,
    pub probe_id: u32,
    pub t_origin_s: f64,
    pub t_rx_s: f64,
    pub hops: u8,
    pub age_ms: f64,
    pub trace: Vec<u16>,
}

/// One loopback (click train) measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawLoopback {
    pub rep: u32,
    /// `None` when not detected.
    pub delay_ms: Option<f64>,
    pub ncc: Option<f64>,
    pub erl_db: Option<f64>,
}

/// Raw timestamps line.
#[derive(Debug, Clone, PartialEq)]
pub struct RawLine {
    pub ts: f64,
    pub link: Option<u32>,
    pub pings: Vec<RawPing>,
    pub mesh_probes: Vec<RawMeshProbe>,
    pub loopback: Vec<RawLoopback>,
}

/// Percentiles in ms.
pub type Pcts = [f64; 3];

/// The last line of a finished run.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryLine {
    pub duration_s: f64,
    pub tx_kbps_mean: f64,
    pub rx_kbps_mean: f64,
    pub loss: f64,
    pub owd_ms: Option<Pcts>,
    pub rtt_ms: Option<Pcts>,
    pub stalls: u32,
    pub reorder: u64,
    pub conn_interval_ms: Option<f64>,
    /// T5a/T5b: loopback round trip and the per-leg estimate.
    pub loopback_ms: Option<Pcts>,
    pub per_leg_ms: Option<f64>,
    /// Seconds whose anchor was not confirmed.
    pub anchor_drops: u32,
    pub verdict: Verdict,
}

/// One `labrun/1` line.
#[derive(Debug, Clone, PartialEq)]
pub enum LabRecord {
    Run(Box<RunLine>),
    Sample(Box<SampleLine>),
    Event {
        ts: f64,
        ev: String,
        fields: Vec<(String, Json)>,
    },
    Mark {
        ts: f64,
        label: String,
    },
    Raw(Box<RawLine>),
    Summary(Box<SummaryLine>),
}
