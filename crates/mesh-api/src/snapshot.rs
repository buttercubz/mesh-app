//! `Snapshot`: the full engine state pushed to the UI (≤ 10 Hz, only when changed and
//! the UI is attached; DESIGN §3.7). Snapshots are the UI's source of truth.

use serde::{Deserialize, Serialize};

use crate::common::{AudioRouteKind, MemberRole, PeerPlatform, TalkMode};
use crate::macros::string_enum;

string_enum! {
    /// Engine lifecycle.
    pub enum EngineState {
        /// No engine thread (group management only).
        Idle = "idle",
        /// Engine thread and radios starting.
        Starting = "starting",
        /// Running normally.
        Running = "running",
        /// Running with a degraded subsystem (audio ladder ≥ L1, radio off).
        Degraded = "degraded",
        /// The engine thread panicked and was stopped (§3.4).
        Crashed = "crashed",
    }
}

string_enum! {
    /// Local talk state.
    pub enum TalkState {
        /// Not transmitting.
        Idle = "idle",
        /// Transmitting.
        Talking = "talking",
        /// Wants to talk but the floor is full (busy tone).
        Blocked = "blocked",
        /// Transmit disabled.
        Muted = "muted",
    }
}

string_enum! {
    /// Ride phase of the session.
    pub enum RidePhase {
        /// Pre-ride lobby.
        Lobby = "lobby",
        /// Ride started.
        Riding = "riding",
    }
}

string_enum! {
    /// Audio degradation ladder level (DESIGN §7.8).
    pub enum LadderLevel {
        /// Full capture and render.
        L0 = "L0",
        /// Restarting.
        L1 = "L1",
        /// Listen-only.
        L2 = "L2",
        /// Relay-only.
        L3 = "L3",
    }
}

string_enum! {
    /// How a member is reached.
    pub enum MemberStatus {
        /// This device.
        Me = "me",
        /// Direct link.
        Direct = "direct",
        /// Through relays.
        Relayed = "relayed",
        /// We hear them, they do not hear us.
        OneWay = "oneWay",
        /// Not heard for 6 s.
        Lost = "lost",
        /// JOIN_REQ has not reached the leader yet.
        Unconfirmed = "unconfirmed",
    }
}

string_enum! {
    /// Transport kind of a link.
    pub enum LinkKind {
        /// Bluetooth LE.
        Ble = "ble",
        /// Wi-Fi Aware.
        Aware = "aware",
        /// v2 cellular tunnel (designed, not built).
        Tunnel = "tunnel",
    }
}

string_enum! {
    /// Whether this member's voice reaches us within the voice budget (§5.5).
    pub enum VoiceReach {
        /// Within the KPI path (≤ 2 BLE hops).
        Kpi = "kpi",
        /// 3-4 hops, within the age cap.
        BestEffort = "bestEffort",
        /// Out of voice reach.
        No = "no",
    }
}

string_enum! {
    /// Member condition flags (from PRESENCE).
    pub enum MemberFlag {
        /// On a phone call.
        OnCall = "onCall",
        /// Capture lost (ladder L2/L3).
        MicLost = "micLost",
        /// No headset; TX muted.
        NoHeadset = "noHeadset",
        /// Bluetooth was off (PRESENCE `radio_off`).
        RadioOff = "radioOff",
    }
}

string_enum! {
    /// Radio profile currently used by the mesh.
    pub enum MeshProfile {
        /// BLE only.
        Ble = "ble",
        /// BLE with reduced capacity.
        BleDegraded = "bleDegraded",
        /// Wi-Fi Aware.
        Wifi = "wifi",
    }
}

string_enum! {
    /// OS thermal state.
    pub enum ThermalState {
        /// Nominal.
        Nominal = "nominal",
        /// Fair.
        Fair = "fair",
        /// Serious.
        Serious = "serious",
        /// Critical.
        Critical = "critical",
    }
}

string_enum! {
    /// Bluetooth radio availability.
    pub enum BluetoothState {
        /// Powered on and authorized.
        On = "on",
        /// Off or airplane mode.
        Off = "off",
        /// Permission denied.
        Unauthorized = "unauthorized",
    }
}

/// The open group session (`null` in Idle and Lab modes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionInfo {
    /// Group id.
    pub group_id: String,
    /// Group name (user content).
    pub group_name: String,
    /// This device's role.
    pub role: MemberRole,
    /// This device's node id.
    pub my_node_id: u16,
    /// Go live done (capture running or muted until the ride starts).
    pub live: bool,
    /// Talk mode.
    pub mode: TalkMode,
    /// Talk state.
    pub talk: TalkState,
    /// Ride phase.
    pub ride: RidePhase,
    /// Degradation ladder level.
    pub ladder: LadderLevel,
    /// Share my distance is on.
    pub share_distance: bool,
}

/// Local audio state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioInfo {
    /// Current route.
    pub route: AudioRouteKind,
    /// Capture running.
    pub capture_ok: bool,
    /// Render running.
    pub render_ok: bool,
    /// Input level (0..1, post-AGC peak).
    pub input_level: f64,
    /// TX muted because only the built-in mic is available (§1.4).
    pub tx_muted_no_headset: bool,
}

/// An active talker (≤ 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Talker {
    /// Node id.
    pub node_id: u16,
    /// Display name (user content).
    pub name: String,
    /// Hops from this device.
    pub hops: u8,
    /// Playout level (0..1).
    pub level: f64,
    /// Burst duration so far, ms.
    pub since_ms: u32,
}

/// Per-member readiness (lobby ready checks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberReady {
    /// Go live done.
    pub live: bool,
    /// Capture running.
    pub mic: bool,
    /// Audio route, `null` before Go live.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub route: Option<AudioRouteKind>,
}

/// One member of the group (this device included, status `me`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Member {
    /// Node id.
    pub node_id: u16,
    /// Display name (user content).
    pub name: String,
    /// Role.
    pub role: MemberRole,
    /// Operating system.
    pub platform: PeerPlatform,
    /// Reachability.
    pub status: MemberStatus,
    /// Hops from this device (0 for `me`).
    pub hops: u8,
    /// Path quality (0..100; `Q_GOOD` = 60).
    pub quality: u8,
    /// Kinds of the links on the best path.
    pub link_kinds: Vec<LinkKind>,
    /// They hear us ("who hears me", §5.8).
    pub hears_me: bool,
    /// Voice reach.
    pub voice: VoiceReach,
    /// Time since last heard, ms.
    pub last_heard_ms: u32,
    /// Battery (0..1), `null` when unknown.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub battery: Option<f64>,
    /// Ready checks.
    pub ready: MemberReady,
    /// Condition flags.
    pub flags: Vec<MemberFlag>,
    /// Distance in metres (both share), else `null`.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub distance_m: Option<f64>,
    /// Bearing in degrees (0 = north), else `null`.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub bearing_deg: Option<f64>,
}

/// Mesh summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeshInfo {
    /// Links up.
    pub links: u8,
    /// Direct neighbours.
    pub neighbors: u8,
    /// Islands (partitions) seen.
    pub islands: u8,
    /// Members reachable.
    pub reachable: u8,
    /// Members in the roster.
    pub total: u8,
    /// Simultaneous-talker cap on BLE paths.
    pub cap_ble: u8,
    /// Simultaneous-talker cap on Wi-Fi paths.
    pub cap_wifi: u8,
    /// Radio profile.
    pub profile: MeshProfile,
}

/// Device state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceInfo {
    /// Battery (0..1), `null` when unknown.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub battery: Option<f64>,
    /// Thermal state.
    pub thermal: ThermalState,
    /// App in the background.
    pub background: bool,
    /// Bluetooth radio state.
    pub bluetooth: BluetoothState,
}

/// Built-in latency estimate (§12.6), excluding headset legs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LatencyInfo {
    /// Median capture→ear, ms.
    pub p50_ms: f64,
    /// 95th percentile, ms.
    pub p95_ms: f64,
    /// p95 per hop count (index 0 = 1 hop), ms.
    pub by_hops: Vec<f64>,
}

/// Full engine state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    /// Engine lifecycle.
    pub engine: EngineState,
    /// Open session, `null` in Idle and Lab modes.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub session: Option<SessionInfo>,
    /// Local audio.
    pub audio: AudioInfo,
    /// Active talkers (≤ 4).
    pub talkers: Vec<Talker>,
    /// Members (≤ 32).
    pub members: Vec<Member>,
    /// Mesh summary.
    pub mesh: MeshInfo,
    /// Device state.
    pub device: DeviceInfo,
    /// Latency estimate, `null` before enough voice frames.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub latency: Option<LatencyInfo>,
}
