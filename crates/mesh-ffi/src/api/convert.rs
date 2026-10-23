//! Conversions between the UniFFI types and the mesh-core / mesh-api / mesh-wire types.

use mesh_api::{AudioRouteKind, PeerPlatform, RoutePreference};
use mesh_core::effects::{AdvPayload, AudioSessionKind, AudioSessionProfile};
use mesh_core::{
    AdvData, AnchorState, AudioCommand, AudioIoState, AudioRouteInfo, Discovery, DiscoveryMode,
    GpsFix, InterruptReason, LinkDownReason, LinkInfo, LinkKind, LinkPriority, LinkRole, LinkUnit,
    LogLevel, Notification, OpenVia, Platform, PlatformSample, PromptRequest, RadioAvail,
    RadioCommand, ScanIntensity, Thermal, Transport,
};
use mesh_wire::consts::BleConstants;

use super::outputs::*;
use super::types::*;

// ------------------------------------------------------------------ inputs

impl From<FfiPlatform> for Platform {
    fn from(p: FfiPlatform) -> Self {
        match p {
            FfiPlatform::Ios => Platform::Ios,
            FfiPlatform::Android => Platform::Android,
            FfiPlatform::WebSim => Platform::WebSim,
        }
    }
}

impl From<FfiPeerPlatform> for PeerPlatform {
    fn from(p: FfiPeerPlatform) -> Self {
        match p {
            FfiPeerPlatform::Ios => PeerPlatform::Ios,
            FfiPeerPlatform::Android => PeerPlatform::Android,
        }
    }
}

impl From<FfiLinkInfo> for LinkInfo {
    fn from(i: FfiLinkInfo) -> Self {
        let kind = match i.kind {
            FfiLinkKind::BleL2cap => LinkKind::BleL2cap,
            FfiLinkKind::BleGatt => LinkKind::BleGatt,
            FfiLinkKind::WifiAware => LinkKind::WifiAware,
            FfiLinkKind::Tunnel => LinkKind::Tunnel,
        };
        let role = match i.role {
            FfiLinkRole::Initiator => LinkRole::Initiator,
            FfiLinkRole::Responder => LinkRole::Responder,
        };
        LinkInfo {
            kind,
            role,
            max_frame: i.max_frame,
            unit: match i.unit {
                FfiLinkUnit::Stream => LinkUnit::Stream,
                FfiLinkUnit::Message => LinkUnit::Message,
                FfiLinkUnit::Datagram => LinkUnit::Datagram,
            },
            peer_platform_hint: i.peer_platform_hint.map(Into::into),
            att_mtu: i.att_mtu,
            disc_id: i.disc_id,
        }
    }
}

impl From<FfiLinkDownReason> for LinkDownReason {
    fn from(r: FfiLinkDownReason) -> Self {
        match r {
            FfiLinkDownReason::Timeout => LinkDownReason::Timeout,
            FfiLinkDownReason::PeerClosed => LinkDownReason::PeerClosed,
            FfiLinkDownReason::LocalClose => LinkDownReason::LocalClose,
            FfiLinkDownReason::RadioOff => LinkDownReason::RadioOff,
            FfiLinkDownReason::LimitReached => LinkDownReason::LimitReached,
            FfiLinkDownReason::Error { code } => LinkDownReason::Error(code),
        }
    }
}

impl From<FfiDiscovery> for Discovery {
    fn from(d: FfiDiscovery) -> Self {
        Discovery {
            disc_id: d.disc_id,
            transport: match d.transport {
                FfiTransport::Ble => Transport::Ble,
                FfiTransport::Aware => Transport::Aware,
            },
            rssi: d.rssi,
            adv: match d.adv {
                FfiAdvData::ServiceData { bytes } => AdvData::ServiceData(bytes),
                FfiAdvData::LocalName { name } => AdvData::LocalName(name),
                FfiAdvData::UuidOnly => AdvData::UuidOnly,
            },
            platform_hint: d.platform_hint.map(Into::into),
        }
    }
}

impl From<FfiAudioRouteKind> for AudioRouteKind {
    fn from(k: FfiAudioRouteKind) -> Self {
        match k {
            FfiAudioRouteKind::HfpWideband => AudioRouteKind::HfpWideband,
            FfiAudioRouteKind::HfpNarrowband => AudioRouteKind::HfpNarrowband,
            FfiAudioRouteKind::LeAudio => AudioRouteKind::LeAudio,
            FfiAudioRouteKind::BuiltinSpeaker => AudioRouteKind::BuiltinSpeaker,
            FfiAudioRouteKind::BuiltinReceiver => AudioRouteKind::BuiltinReceiver,
            FfiAudioRouteKind::Wired => AudioRouteKind::Wired,
            FfiAudioRouteKind::CarBluetooth => AudioRouteKind::CarBluetooth,
            FfiAudioRouteKind::None => AudioRouteKind::None,
        }
    }
}

impl From<FfiAudioRoute> for AudioRouteInfo {
    fn from(r: FfiAudioRoute) -> Self {
        AudioRouteInfo {
            kind: r.kind.into(),
            input_rate: r.input_rate,
            output: r.output.into(),
            headset_present: r.headset_present,
        }
    }
}

impl From<FfiAudioIoState> for AudioIoState {
    fn from(s: FfiAudioIoState) -> Self {
        match s {
            FfiAudioIoState::Running => AudioIoState::Running,
            FfiAudioIoState::Interrupted { reason } => AudioIoState::Interrupted {
                reason: match reason {
                    FfiInterruptReason::Call => InterruptReason::Call,
                    FfiInterruptReason::Assistant => InterruptReason::Assistant,
                    FfiInterruptReason::OtherApp => InterruptReason::OtherApp,
                    FfiInterruptReason::RouteChange => InterruptReason::RouteChange,
                    FfiInterruptReason::MediaReset => InterruptReason::MediaReset,
                    FfiInterruptReason::Other => InterruptReason::Other,
                },
            },
            FfiAudioIoState::Restarting => AudioIoState::Restarting,
            FfiAudioIoState::CaptureLost => AudioIoState::CaptureLost,
            FfiAudioIoState::RenderLost => AudioIoState::RenderLost,
            FfiAudioIoState::Restored => AudioIoState::Restored,
            FfiAudioIoState::Stopped => AudioIoState::Stopped,
        }
    }
}

impl From<FfiRadioAvail> for RadioAvail {
    fn from(r: FfiRadioAvail) -> Self {
        match r {
            FfiRadioAvail::On => RadioAvail::On,
            FfiRadioAvail::Off => RadioAvail::Off,
            FfiRadioAvail::Unauthorized => RadioAvail::Unauthorized,
            FfiRadioAvail::Unsupported => RadioAvail::Unsupported,
        }
    }
}

impl FfiPlatformSample {
    /// The core sample (audio counters left at default; the runtime fills them) and the
    /// radio state carried with it.
    pub fn split(self) -> (PlatformSample, (RadioAvail, RadioAvail)) {
        let sample = PlatformSample {
            wall_ms: self.wall_ms,
            gps: self.gps.map(|g| GpsFix {
                lat: g.lat,
                lon: g.lon,
                speed_mps: g.speed_mps,
                acc_m: g.acc_m,
                heading_deg: g.heading_deg,
            }),
            battery: self.battery,
            charging: self.charging,
            thermal: self.thermal.map(|t| match t {
                FfiThermal::Nominal => Thermal::Nominal,
                FfiThermal::Fair => Thermal::Fair,
                FfiThermal::Serious => Thermal::Serious,
                FfiThermal::Critical => Thermal::Critical,
            }),
            background: self.background,
            locked: self.locked,
            on_call: self.on_call,
            audio: Default::default(),
            anchor: AnchorState {
                session: self.anchor_session,
                location: self.anchor_location,
                live_activity: self.anchor_live_activity,
            },
        };
        (sample, (self.ble.into(), self.aware.into()))
    }
}

// ------------------------------------------------------------------ outputs

impl From<DiscoveryMode> for FfiDiscoveryMode {
    fn from(m: DiscoveryMode) -> Self {
        match m {
            DiscoveryMode::Off => FfiDiscoveryMode::Off,
            DiscoveryMode::Forming => FfiDiscoveryMode::Forming,
            DiscoveryMode::Stable => FfiDiscoveryMode::Stable,
        }
    }
}

impl From<ScanIntensity> for FfiScanIntensity {
    fn from(s: ScanIntensity) -> Self {
        match s {
            ScanIntensity::LowLatency => FfiScanIntensity::LowLatency,
            ScanIntensity::Balanced => FfiScanIntensity::Balanced,
            ScanIntensity::LowPower => FfiScanIntensity::LowPower,
        }
    }
}

impl From<AdvPayload> for FfiAdvPayload {
    fn from(a: AdvPayload) -> Self {
        FfiAdvPayload {
            service_data: a.service_data.to_vec(),
            local_name: a.local_name,
        }
    }
}

impl From<RadioCommand> for FfiRadioCommand {
    fn from(c: RadioCommand) -> Self {
        match c {
            RadioCommand::SetDiscovery { mode, adv, scan } => FfiRadioCommand::SetDiscovery {
                mode: mode.into(),
                adv: adv.map(Into::into),
                scan: scan.into(),
            },
            RadioCommand::SetHello { bytes } => FfiRadioCommand::SetHello {
                bytes: bytes.to_vec(),
            },
            RadioCommand::Connect { disc_id } => FfiRadioCommand::Connect { disc_id },
            RadioCommand::OpenLink { disc_id, via } => FfiRadioCommand::OpenLink {
                disc_id,
                via: match via {
                    OpenVia::L2cap { psm } => FfiOpenVia::L2cap { psm },
                    OpenVia::Gatt => FfiOpenVia::Gatt,
                },
            },
            RadioCommand::Abort { disc_id, ignore_s } => {
                FfiRadioCommand::Abort { disc_id, ignore_s }
            }
            RadioCommand::Close { link, reason } => FfiRadioCommand::Close {
                link: link.0,
                reason: reason.map(|r| r as u8),
            },
            RadioCommand::SetLinkParams { link, priority } => FfiRadioCommand::SetLinkParams {
                link: link.0,
                priority: match priority {
                    LinkPriority::High => FfiLinkPriority::High,
                    LinkPriority::Balanced => FfiLinkPriority::Balanced,
                },
            },
            RadioCommand::ArmReconnect { peer_ref, until_s } => {
                FfiRadioCommand::ArmReconnect { peer_ref, until_s }
            }
            RadioCommand::AwareUpgrade { peer, op } => FfiRadioCommand::AwareUpgrade { peer, op },
            RadioCommand::TunnelOpen { peer } => FfiRadioCommand::TunnelOpen { peer },
        }
    }
}

impl From<AudioSessionProfile> for FfiAudioSessionProfile {
    fn from(p: AudioSessionProfile) -> Self {
        FfiAudioSessionProfile {
            kind: match p.kind {
                AudioSessionKind::Ride => FfiAudioSessionKind::Ride,
                AudioSessionKind::LabRender => FfiAudioSessionKind::LabRender,
                AudioSessionKind::LabFull => FfiAudioSessionKind::LabFull,
                AudioSessionKind::Check => FfiAudioSessionKind::Check,
            },
            capture: p.capture,
            mixable: p.mixable,
            location_anchor: p.location_anchor,
            bypass_voice_processing: p.bypass_voice_processing,
        }
    }
}

impl From<AudioCommand> for FfiAudioCommand {
    fn from(c: AudioCommand) -> Self {
        match c {
            AudioCommand::OpenSession { profile } => FfiAudioCommand::OpenSession {
                profile: profile.into(),
            },
            AudioCommand::StartCapture => FfiAudioCommand::StartCapture,
            AudioCommand::StopCapture => FfiAudioCommand::StopCapture,
            AudioCommand::SetRoutePreference(p) => FfiAudioCommand::SetRoutePreference {
                pref: match p {
                    RoutePreference::Auto => FfiRoutePreference::Auto,
                    RoutePreference::Headset => FfiRoutePreference::Headset,
                    RoutePreference::Speaker => FfiRoutePreference::Speaker,
                },
            },
            AudioCommand::OutputOnlyFallback(enabled) => {
                FfiAudioCommand::OutputOnlyFallback { enabled }
            }
            AudioCommand::CloseSession => FfiAudioCommand::CloseSession,
        }
    }
}

impl From<Notification> for FfiNotification {
    fn from(n: Notification) -> Self {
        FfiNotification {
            category: FfiNotificationCategory::Notice,
            key: n.key.as_str().to_owned(),
            params_json: serde_json::to_string(&n.params).unwrap_or_else(|_| "{}".into()),
        }
    }
}

impl From<PromptRequest> for FfiPromptRequest {
    fn from(r: PromptRequest) -> Self {
        FfiPromptRequest {
            key: r.key.as_str().to_owned(),
            text: r.text,
            lang: r.lang.as_str().to_owned(),
        }
    }
}

impl From<LogLevel> for FfiLogLevel {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Debug => FfiLogLevel::Debug,
            LogLevel::Info => FfiLogLevel::Info,
            LogLevel::Warn => FfiLogLevel::Warn,
            LogLevel::Error => FfiLogLevel::Error,
        }
    }
}

impl From<BleConstants> for FfiBleConstants {
    fn from(c: BleConstants) -> Self {
        // Exhaustive destructuring: a new mesh-wire constant fails to compile here.
        let BleConstants {
            service,
            hello,
            rx,
            tx,
            hello_len,
            max_stream_frame,
            max_datagram_frame,
            preferred_att_mtu,
            ble_links_target,
            ble_links_max,
            neighbors_max,
            connect_timeout_ms,
            handshake_timeout_ms,
            backoff_initial_ms,
            backoff_max_ms,
            link_death_ms,
            ping_active_ms,
            ping_idle_ms,
            adv_interval_forming_ms,
            adv_interval_stable_ms,
            scan_restart_ms,
            scan_quota_starts,
            scan_quota_window_ms,
            token_rotation_s,
            pending_reconnect_ms,
            limit_backoff_ms,
            local_name_len,
        } = c;
        FfiBleConstants {
            service_uuid: service.to_canonical(),
            hello_uuid: hello.to_canonical(),
            rx_uuid: rx.to_canonical(),
            tx_uuid: tx.to_canonical(),
            hello_len,
            max_stream_frame,
            max_datagram_frame,
            preferred_att_mtu,
            ble_links_target,
            ble_links_max,
            neighbors_max,
            connect_timeout_ms,
            handshake_timeout_ms,
            backoff_initial_ms,
            backoff_max_ms,
            link_death_ms,
            ping_active_ms,
            ping_idle_ms,
            adv_interval_forming_ms,
            adv_interval_stable_ms,
            scan_restart_ms,
            scan_quota_starts,
            scan_quota_window_ms,
            token_rotation_s,
            pending_reconnect_ms,
            limit_backoff_ms,
            local_name_len,
        }
    }
}
