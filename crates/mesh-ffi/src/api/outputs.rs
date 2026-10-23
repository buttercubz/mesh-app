//! UniFFI records and enums of contract C4 (DESIGN §3.5): engine → platform outputs
//! carried by `PlatformHooks`, and `ble_constants()`.

/// Result of a non-blocking `link_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiSendOutcome {
    /// Written (or queued in a bounded OS buffer).
    Accepted,
    /// The link cannot take the unit now; the core re-sends it first after
    /// `on_link_writable`.
    WouldBlock,
    /// The link does not exist (any more).
    NoLink,
}

/// Scan/advertise mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiDiscoveryMode {
    Off,
    Forming,
    Stable,
}

/// Scan duty the platform should use (§5.7, §9.5). Android: `SCAN_MODE_LOW_LATENCY`
/// / `SCAN_MODE_BALANCED` / `SCAN_MODE_LOW_POWER`; iOS: `AllowDuplicates` only at
/// `LowLatency`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiScanIntensity {
    /// Continuous scanning: only while no BLE link is up.
    LowLatency,
    /// ~25 % duty: hunting for a member with at least one link up.
    Balanced,
    /// ~10 % duty: every member reachable (`Stable`).
    LowPower,
}

/// Advertising payload built by the core (§9.4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiAdvPayload {
    /// Android scan response service data (9 B after `MESH_SVC`).
    pub service_data: Vec<u8>,
    /// iPhone foreground local name (6 base32 chars).
    pub local_name: String,
}

/// How to open the data path after a HELLO check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiOpenVia {
    L2cap { psm: u16 },
    Gatt,
}

/// Connection priority hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiLinkPriority {
    High,
    Balanced,
}

/// Radio decisions executed by the platform (§3.2).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRadioCommand {
    SetDiscovery {
        mode: FfiDiscoveryMode,
        adv: Option<FfiAdvPayload>,
        scan: FfiScanIntensity,
    },
    /// New 16-byte HELLO value.
    SetHello {
        bytes: Vec<u8>,
    },
    Connect {
        disc_id: u32,
    },
    OpenLink {
        disc_id: u32,
        via: FfiOpenVia,
    },
    Abort {
        disc_id: u32,
        ignore_s: u16,
    },
    /// `reason` is the CLOSE/REJECT reason byte already sent in band, if any.
    Close {
        link: u32,
        reason: Option<u8>,
    },
    SetLinkParams {
        link: u32,
        priority: FfiLinkPriority,
    },
    ArmReconnect {
        peer_ref: u32,
        until_s: u16,
    },
    AwareUpgrade {
        peer: u16,
        op: u8,
    },
    TunnelOpen {
        peer: u16,
    },
}

/// Kind of audio session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAudioSessionKind {
    Ride,
    LabRender,
    LabFull,
    Check,
}

/// Audio session profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiAudioSessionProfile {
    pub kind: FfiAudioSessionKind,
    pub capture: bool,
    pub mixable: bool,
    pub location_anchor: bool,
    pub bypass_voice_processing: bool,
}

/// Output route preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRoutePreference {
    Auto,
    Headset,
    Speaker,
}

/// Audio decisions executed by the platform (§3.2). `StartCapture` only connects the
/// RT sink and unmutes; it never activates a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAudioCommand {
    OpenSession { profile: FfiAudioSessionProfile },
    StartCapture,
    StopCapture,
    SetRoutePreference { pref: FfiRoutePreference },
    OutputOnlyFallback { enabled: bool },
    CloseSession,
}

/// Which i18n catalog a notification key belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiNotificationCategory {
    /// `notices.<key>`.
    Notice,
    /// `errors.<key>` (e.g. `engineCrashed`).
    Error,
}

/// Local notification / Live Activity / FGS notification update.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiNotification {
    pub category: FfiNotificationCategory,
    /// i18n key (never prose).
    pub key: String,
    /// Interpolation params as a JSON object (`Record<string, string | number>`).
    pub params_json: String,
}

/// A spoken prompt to render with platform TTS; the result comes back through
/// `on_prompt_pcm(key, pcm)` (16 kHz mono).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiPromptRequest {
    pub key: String,
    pub text: String,
    /// `es` | `en`.
    pub lang: String,
}

/// Log severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum)]
pub enum FfiLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// BLE UUIDs and timings: the single source of truth for Swift and Kotlin (§9.3-§9.5).
/// UUIDs are uppercase canonical strings.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiBleConstants {
    pub service_uuid: String,
    pub hello_uuid: String,
    pub rx_uuid: String,
    pub tx_uuid: String,
    pub hello_len: u16,
    pub max_stream_frame: u16,
    pub max_datagram_frame: u16,
    pub preferred_att_mtu: u16,
    pub ble_links_target: u8,
    pub ble_links_max: u8,
    pub neighbors_max: u8,
    pub connect_timeout_ms: u32,
    pub handshake_timeout_ms: u32,
    pub backoff_initial_ms: u32,
    pub backoff_max_ms: u32,
    pub link_death_ms: u32,
    pub ping_active_ms: u32,
    pub ping_idle_ms: u32,
    pub adv_interval_forming_ms: u32,
    pub adv_interval_stable_ms: u32,
    pub scan_restart_ms: u32,
    pub scan_quota_starts: u8,
    pub scan_quota_window_ms: u32,
    pub token_rotation_s: u32,
    pub pending_reconnect_ms: u32,
    pub limit_backoff_ms: u32,
    pub local_name_len: u8,
}
