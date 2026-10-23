//! Lab (Field test) types of the UI contract (DESIGN §3.7, §12). Phase 0 subset.

use serde::{Deserialize, Serialize};

use crate::macros::string_enum;

string_enum! {
    /// Field bench (DESIGN §12.2); the same ids appear in `labrun/1` `bench`.
    pub enum LabBench {
        /// BLE throughput / latency / loss between two phones.
        T0 = "T0",
        /// Three-phone relay chain.
        T0b = "T0b",
        /// Multi-link hub.
        T0c = "T0c",
        /// Range between vehicles.
        T1 = "T1",
        /// Background survival.
        T2 = "T2",
        /// Rediscovery.
        T3 = "T3",
        /// Wi-Fi Aware (Phase 4).
        T4 = "T4",
        /// Headset loopback latency.
        T5a = "T5a",
        /// Headset behaviour checklist (Phase 1).
        T5b = "T5b",
        /// Battery and thermal.
        T6 = "T6",
        /// UI and PTT input latency (Phase 1).
        T8 = "T8",
        /// VOX calibration (Phase 1).
        VoxCal = "voxCal",
        /// Passive ride recorder (Phase 2).
        Ride = "ride",
    }
}

string_enum! {
    /// Audio path of a Lab run (mandatory, DESIGN §12.1). `renderOnly` and `full` are
    /// started with `gestureCommand`.
    pub enum LabAudio {
        /// Radio-only bench, no audio session.
        Off = "off",
        /// Session + output only: keeps the `audio` anchor, plays bench tones, no mic.
        RenderOnly = "renderOnly",
        /// Session + capture and render; HFP opened when a headset is connected.
        Full = "full",
    }
}

string_enum! {
    /// Keep-alive anchor of a Lab run (mandatory, DESIGN §12.1).
    pub enum LabAnchor {
        /// Control variant that measures suspension itself; never the default.
        None = "none",
        /// VPIO running (render-only or full).
        Audio = "audio",
        /// Audio plus background location updates.
        AudioLocation = "audioLocation",
    }
}

string_enum! {
    /// LPROBE traffic pattern (DESIGN §12.1-12.2).
    pub enum LabPattern {
        /// Saturate the link.
        Saturate = "saturate",
        /// Voice-like CBR streams padded to exact VOICE frame sizes.
        VoiceLike = "voiceLike",
        /// Ping only.
        Ping = "ping",
        /// Ping while voice-like streams load the link.
        PingUnderLoad = "pingUnderLoad",
    }
}

string_enum! {
    /// BLE transport variant of a bench.
    pub enum LabTransport {
        /// L2CAP connection-oriented channel.
        L2cap = "l2cap",
        /// GATT notifications.
        GattNotify = "gattNotify",
        /// GATT write without response.
        GattWrite = "gattWrite",
    }
}

string_enum! {
    /// BLE role variant of the local phone.
    pub enum LabLinkRole {
        /// Local phone is the central.
        Central = "central",
        /// Local phone is the peripheral.
        Peripheral = "peripheral",
    }
}

string_enum! {
    /// Go / no-go verdict of a finished run against the bench thresholds.
    pub enum LabVerdict {
        /// Meets the go threshold.
        Go = "go",
        /// Fails the go threshold.
        NoGo = "noGo",
        /// Not enough valid data (e.g. the anchor dropped).
        Inconclusive = "inconclusive",
    }
}

/// Parameters of `labStart`. `audio`, `anchor` and `gps` are mandatory: a command
/// without them fails to deserialize (DESIGN §3.7, review blocker 2). The other fields
/// are bench-specific and optional; the engine applies the bench defaults (WP-05
/// catalog) for any field left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabParams {
    /// Audio path of the run.
    pub audio: LabAudio,
    /// Keep-alive anchor of the run.
    pub anchor: LabAnchor,
    /// Per-run GPS consent.
    pub gps: bool,
    /// Run length in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<u32>,
    /// Traffic pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<LabPattern>,
    /// Number of voice-like streams (1-4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streams: Option<u8>,
    /// Payload size per probe in bytes (e.g. 118 B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_b: Option<u16>,
    /// Probe interval in milliseconds (e.g. 40 ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u16>,
    /// Ping rate in hertz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_hz: Option<u8>,
    /// BLE transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<LabTransport>,
    /// BLE role of this phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<LabLinkRole>,
    /// A headset (HFP) is active during the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headset: Option<bool>,
    /// Mixable audio session variant (T2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixable: Option<bool>,
    /// Free variant label recorded in the `run` line (e.g. "pocket"); user content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl LabParams {
    /// Minimal parameters: only the mandatory fields, bench defaults for the rest.
    pub fn new(audio: LabAudio, anchor: LabAnchor, gps: bool) -> Self {
        LabParams {
            audio,
            anchor,
            gps,
            duration_s: None,
            pattern: None,
            streams: None,
            payload_b: None,
            interval_ms: None,
            ping_hz: None,
            transport: None,
            role: None,
            headset: None,
            mixable: None,
            variant: None,
        }
    }
}

/// Live numbers of a running Lab run (`labProgress`, 2 Hz; DESIGN §12.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabLiveSample {
    /// Lab links currently up.
    pub links: u8,
    /// Lab peers heard in the last 2 s.
    pub peers: u8,
    /// Transmit throughput over all Lab links, kbit/s.
    pub tx_kbps: f64,
    /// Receive throughput over all Lab links, kbit/s.
    pub rx_kbps: f64,
    /// Probe loss ratio over the last second (0..1).
    pub loss: f64,
    /// RTT p50 in ms, `null` before the first PONG.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub rtt_p50_ms: Option<f64>,
    /// RTT p95 in ms, `null` before the first PONG.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub rtt_p95_ms: Option<f64>,
    /// Strongest link RSSI in dBm, `null` when unknown.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub rssi: Option<i16>,
    /// Distance to the nearest peer in metres (GPS consent on both ends), else `null`.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub dist_m: Option<f64>,
    /// Stalls longer than 100 ms since the run started.
    pub stalls: u32,
    /// The run's keep-alive anchor is active.
    pub anchor_ok: bool,
}

/// Mean of a rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabRate {
    /// Mean over the run.
    pub mean: f64,
}

/// Latency percentiles in milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabPercentiles {
    /// Median, ms.
    pub p50: f64,
    /// 95th percentile, ms.
    pub p95: f64,
    /// 99th percentile, ms.
    pub p99: f64,
}

/// On-device summary of a finished run (mirrors the `labrun/1` `summary` line, §12.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabSummary {
    /// Run length in seconds.
    pub duration_s: f64,
    /// Transmit throughput, kbit/s.
    pub tx_kbps: LabRate,
    /// Probe loss ratio (0..1).
    pub loss: f64,
    /// One-way delay estimate percentiles, `null` without clock estimates.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub owd_ms: Option<LabPercentiles>,
    /// RTT percentiles, `null` without PONGs.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub rtt_ms: Option<LabPercentiles>,
    /// Stalls longer than 100 ms.
    pub stalls: u32,
    /// Verdict against the bench thresholds.
    pub verdict: LabVerdict,
}

/// One stored run in `labListRuns`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabRunInfo {
    /// Run id (`r-YYYYMMDD-HHMM-xxxx`).
    pub run_id: String,
    /// Bench of the run.
    pub bench: LabBench,
    /// Audio path of the run.
    pub audio: LabAudio,
    /// Keep-alive anchor of the run.
    pub anchor: LabAnchor,
    /// ISO 8601 start time stamped by the runtime.
    pub started_at: String,
    /// Run length in seconds, `null` while the summary is missing (interrupted run).
    #[serde(deserialize_with = "crate::common::nullable")]
    pub duration_s: Option<f64>,
    /// Verdict, `null` while the summary is missing.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub verdict: Option<LabVerdict>,
    /// Exported files (labrun/1 JSONL and summary CSV), platform paths.
    pub files: Vec<String>,
}
