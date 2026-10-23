//! UniFFI records and enums of contract C4 (DESIGN §3.5): the platform → engine inputs.
//! Engine → platform outputs live in `outputs.rs`.

/// Platform the engine runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPlatform {
    Ios,
    Android,
    /// Host tests and the browser simulator.
    WebSim,
}

/// One persisted key/value written earlier through `PlatformHooks::persist`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiPersistedEntry {
    pub key: String,
    pub value: Vec<u8>,
}

/// Engine configuration handed to `MeshEngine::new`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiEngineConfig {
    pub platform: FfiPlatform,
    /// App build number (CAPS `app_ver`).
    pub app_version: u16,
    /// 32 bytes from the platform CSPRNG (`SecRandomCopyBytes` / `SecureRandom`).
    pub rng_seed: Vec<u8>,
    /// Hardware model (`iPhone15,2`).
    pub device_model: String,
    /// OS name and version (`iOS 27.0`).
    pub device_os: String,
    /// App version string (`0.1.0 (12)`).
    pub app_label: String,
    /// Every key the engine persisted before (`group/<hex>`, `lab/index`, …), loaded from
    /// secure storage; the runtime rebuilds the stored groups and Lab runs from it.
    pub persisted: Vec<FfiPersistedEntry>,
}

/// How to start the engine thread (`MeshEngine::start`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiSessionStart {
    /// Group lobby: radios on, audio off until Go live. `group_id` is 16 bytes.
    Group { group_id: Vec<u8> },
    /// Lab mode with the shared 4-digit code.
    Lab { lab_code: u16 },
    /// Demo ride with simulated riders (runtime-owned, §3.4).
    Demo,
}

/// Transport kind of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiLinkKind {
    BleL2cap,
    BleGatt,
    WifiAware,
    Tunnel,
}

/// Who opened the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiLinkRole {
    Initiator,
    Responder,
}

/// What one `on_link_data` call carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiLinkUnit {
    /// Arbitrary stream chunks.
    Stream,
    /// One GATT value.
    Message,
    /// One datagram.
    Datagram,
}

/// Peer platform hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPeerPlatform {
    Ios,
    Android,
}

/// Link description given on link-up (§9.1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiLinkInfo {
    pub kind: FfiLinkKind,
    pub role: FfiLinkRole,
    /// Largest LinkFrame (clamped by the core to the §4.2 maximum of the kind).
    pub max_frame: u16,
    pub unit: FfiLinkUnit,
    pub peer_platform_hint: Option<FfiPeerPlatform>,
    pub att_mtu: Option<u16>,
    /// The discovery this link came from (initiator side).
    pub disc_id: Option<u32>,
}

/// Why a link went down (or a connect failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiLinkDownReason {
    Timeout,
    PeerClosed,
    LocalClose,
    RadioOff,
    /// iOS `connectionLimitReached`, Android GATT 133/257.
    LimitReached,
    Error {
        code: i32,
    },
}

/// Radio a discovery came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTransport {
    Ble,
    Aware,
}

/// Raw advertising data (the core parses it).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAdvData {
    /// Android scan-response service data after `MESH_SVC`, or the raw AD sequence.
    ServiceData { bytes: Vec<u8> },
    /// iPhone foreground local name.
    LocalName { name: String },
    /// Only the service UUID (iPhone in background).
    UuidOnly,
}

/// A discovered peer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiDiscovery {
    pub disc_id: u32,
    pub transport: FfiTransport,
    pub rssi: Option<i16>,
    pub adv: FfiAdvData,
    pub platform_hint: Option<FfiPeerPlatform>,
}

/// Audio route kind (UI contract `AudioRouteKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAudioRouteKind {
    HfpWideband,
    HfpNarrowband,
    LeAudio,
    BuiltinSpeaker,
    BuiltinReceiver,
    Wired,
    CarBluetooth,
    None,
}

/// Current audio route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiAudioRoute {
    pub kind: FfiAudioRouteKind,
    /// Hardware input rate in Hz (0 = no input).
    pub input_rate: u32,
    pub output: FfiAudioRouteKind,
    pub headset_present: bool,
}

/// Why the audio IO was interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiInterruptReason {
    Call,
    Assistant,
    OtherApp,
    RouteChange,
    MediaReset,
    Other,
}

/// Audio IO state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAudioIoState {
    Running,
    Interrupted { reason: FfiInterruptReason },
    Restarting,
    CaptureLost,
    RenderLost,
    Restored,
    Stopped,
}

/// Radio availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRadioAvail {
    On,
    /// Bluetooth off / airplane mode.
    Off,
    Unauthorized,
    Unsupported,
}

/// A GPS fix (only with consent).
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct FfiGpsFix {
    pub lat: f64,
    pub lon: f64,
    pub speed_mps: Option<f64>,
    pub acc_m: Option<f64>,
    pub heading_deg: Option<f64>,
}

/// Thermal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiThermal {
    Nominal,
    Fair,
    Serious,
    Critical,
}

/// 1 Hz platform sample, including the radio state (forwarded as a `RadioState`
/// event when it changes). Audio counters are added by the runtime from the RT stats.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiPlatformSample {
    /// Wall clock, Unix ms.
    pub wall_ms: Option<u64>,
    pub gps: Option<FfiGpsFix>,
    /// Battery level 0..1.
    pub battery: Option<f64>,
    pub charging: Option<bool>,
    pub thermal: Option<FfiThermal>,
    pub background: bool,
    pub locked: Option<bool>,
    pub on_call: bool,
    /// Keep-alive anchors (§12.1): audio session, location, Live Activity.
    pub anchor_session: Option<bool>,
    pub anchor_location: Option<bool>,
    pub anchor_live_activity: Option<bool>,
    /// Bluetooth availability.
    pub ble: FfiRadioAvail,
    /// Wi-Fi Aware availability.
    pub aware: FfiRadioAvail,
}

/// Diagnostics recorder state (`MeshEngine::diag_status`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiDiagStatus {
    /// Lines are written to files (the RAM ring always records).
    pub enabled: bool,
    /// The open session (`d-YYYYMMDD-HHMM-xxxx`), if any.
    pub session_id: Option<String>,
    /// Ride key of the open session (`9f3a71c2`, `lab-1a2b`, `demo`).
    pub ride_key: Option<String>,
    /// Group node id, once the core reported it.
    pub node: Option<u16>,
    /// Lines waiting for the next `diag_write`.
    pub pending_lines: u32,
    /// Events dropped by the rate limiter (cumulative).
    pub suppressed: u64,
    /// Strings replaced by the redaction filter (cumulative).
    pub redacted: u64,
}

/// Runtime state reported by `MeshEngine::status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRuntimeStatus {
    /// No engine thread: group management only.
    Idle,
    /// Engine thread running a group session or Lab mode.
    Running,
    /// Engine thread running for an audio check without a session.
    AudioCheck,
    /// The last engine thread panicked; the runtime is back in Idle until `start`.
    Crashed,
}

/// Errors of the control plane.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum FfiError {
    /// The configuration is invalid (`detail` names the field).
    InvalidConfig { detail: String },
    /// The session start parameters are invalid (bad group id, Lab code over 9999).
    InvalidStart { detail: String },
    /// No stored group has this id.
    UnknownGroup,
    /// The Demo ride is not available in this build (Phase 1, needs the mesh-sim world).
    DemoUnavailable,
    /// The engine thread could not be spawned.
    ThreadSpawn { detail: String },
    /// The engine thread panicked while handling the request.
    Crashed,
}

impl std::fmt::Display for FfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FfiError::InvalidConfig { detail } => write!(f, "invalid config: {detail}"),
            FfiError::InvalidStart { detail } => write!(f, "invalid start: {detail}"),
            FfiError::UnknownGroup => f.write_str("unknown group"),
            FfiError::DemoUnavailable => f.write_str("demo ride not available in this build"),
            FfiError::ThreadSpawn { detail } => write!(f, "engine thread spawn failed: {detail}"),
            FfiError::Crashed => f.write_str("engine crashed"),
        }
    }
}

impl std::error::Error for FfiError {}
