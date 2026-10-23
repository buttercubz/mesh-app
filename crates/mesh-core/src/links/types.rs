//! Link vocabulary shared with the platform (DESIGN §3.2, §9.1).

use mesh_api::PeerPlatform;
use mesh_wire::frame::RejectReason;
use mesh_wire::link::{MAX_DATAGRAM_FRAME, MAX_STREAM_FRAME};

/// Transport kind of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkKind {
    /// BLE L2CAP connection-oriented channel (stream).
    BleL2cap,
    /// BLE GATT data path (one ATT value per call).
    BleGatt,
    /// Wi-Fi Aware UDP (datagrams).
    WifiAware,
    /// v2 tunnel (stream).
    Tunnel,
}

impl LinkKind {
    /// `true` for the two BLE kinds (they share one radio and the BLE link max).
    pub const fn is_ble(self) -> bool {
        matches!(self, LinkKind::BleL2cap | LinkKind::BleGatt)
    }

    /// The natural unit of this kind.
    pub const fn unit(self) -> LinkUnit {
        match self {
            LinkKind::BleL2cap | LinkKind::Tunnel => LinkUnit::Stream,
            LinkKind::BleGatt => LinkUnit::Message,
            LinkKind::WifiAware => LinkUnit::Datagram,
        }
    }

    /// mesh-routing kind (BLE L2CAP and GATT are both BLE).
    pub const fn routing(self) -> mesh_routing::LinkKind {
        match self {
            LinkKind::BleL2cap | LinkKind::BleGatt => mesh_routing::LinkKind::Ble,
            LinkKind::WifiAware => mesh_routing::LinkKind::Aware,
            LinkKind::Tunnel => mesh_routing::LinkKind::Tunnel,
        }
    }

    /// labrun/1 link kind.
    pub const fn lab(self) -> mesh_lab::LinkKind {
        match self {
            LinkKind::BleL2cap => mesh_lab::LinkKind::BleL2cap,
            LinkKind::BleGatt => mesh_lab::LinkKind::BleGatt,
            LinkKind::WifiAware => mesh_lab::LinkKind::Aware,
            LinkKind::Tunnel => mesh_lab::LinkKind::Tunnel,
        }
    }

    /// Largest LinkFrame on this kind (§4.2).
    pub const fn max_frame(self) -> usize {
        match self {
            LinkKind::WifiAware => MAX_DATAGRAM_FRAME,
            _ => MAX_STREAM_FRAME,
        }
    }
}

/// Who opened the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkRole {
    /// This side connected (BLE central / Aware subscriber).
    Initiator,
    /// The peer connected.
    Responder,
}

/// What one `handle_packet` call carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkUnit {
    /// Arbitrary stream chunks (`len u16` framing).
    Stream,
    /// One GATT value (`hdr u8` + chunk).
    Message,
    /// One datagram (one frame).
    Datagram,
}

/// Link description given by the platform on `Up` (§9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkInfo {
    pub kind: LinkKind,
    pub role: LinkRole,
    /// Largest LinkFrame for this link (≤ 1024 B stream/GATT, ≤ 1200 B datagram); the
    /// core clamps it to the §4.2 maximum of the kind.
    pub max_frame: u16,
    pub unit: LinkUnit,
    pub peer_platform_hint: Option<PeerPlatform>,
    /// Negotiated ATT MTU (GATT links), if known at link-up.
    pub att_mtu: Option<u16>,
    /// The discovery this link came from (initiator side), if any.
    pub disc_id: Option<u32>,
}

impl LinkInfo {
    /// A link of `kind` with its natural unit and frame size.
    pub const fn new(kind: LinkKind, role: LinkRole) -> Self {
        LinkInfo {
            kind,
            role,
            max_frame: kind.max_frame() as u16,
            unit: kind.unit(),
            peer_platform_hint: None,
            att_mtu: None,
            disc_id: None,
        }
    }
}

/// Why a link went down (§3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkDownReason {
    /// No inbound frame for the link-death time (or a connect timeout).
    Timeout,
    /// The peer closed the link without telling why (platform close).
    PeerClosed,
    /// The peer closed the link with a reason: a REJECT frame (§4.3) or a CLOSE record
    /// (§4.4, which adds `Redundant` and `Leaving`). Engine-generated, never reported by
    /// the platform.
    PeerRejected(RejectReason),
    /// This side closed it.
    LocalClose,
    /// Bluetooth was switched off (airplane mode, Settings).
    RadioOff,
    /// The OS connection limit was reached (iOS `connectionLimitReached`, Android
    /// GATT 133/257).
    LimitReached,
    /// Platform error code.
    Error(i32),
}

impl LinkDownReason {
    /// labrun/1 `linkDown` reason string.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkDownReason::Timeout => "timeout",
            LinkDownReason::PeerClosed | LinkDownReason::PeerRejected(_) => "peerClosed",
            LinkDownReason::LocalClose => "closed",
            LinkDownReason::RadioOff => "radioOff",
            LinkDownReason::LimitReached => "limitReached",
            LinkDownReason::Error(_) => "error",
        }
    }

    /// `true` when the peer closed the link (with or without a reason).
    pub const fn peer_closed(self) -> bool {
        matches!(
            self,
            LinkDownReason::PeerClosed | LinkDownReason::PeerRejected(_)
        )
    }

    /// The reason the peer gave (REJECT frame or CLOSE record), if any.
    pub const fn peer_reason(self) -> Option<RejectReason> {
        match self {
            LinkDownReason::PeerRejected(r) => Some(r),
            _ => None,
        }
    }
}

impl core::hash::Hash for LinkDownReason {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            LinkDownReason::PeerRejected(r) => (*r as u8).hash(state),
            LinkDownReason::Error(code) => code.hash(state),
            _ => {}
        }
    }
}

/// Radio a discovery came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Transport {
    Ble,
    Aware,
}

/// Raw advertising data handed over by the platform; the core parses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvData {
    /// Android scan response: the service data bytes after `MESH_SVC` (9 B), or the
    /// whole raw AD sequence (the core finds the service data in it).
    ServiceData(Vec<u8>),
    /// iPhone foreground local name (6 base32 chars).
    LocalName(String),
    /// Only the service UUID was seen (iPhone in background).
    UuidOnly,
}

/// A discovered peer (§3.2 `Discovered`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// Platform id of this advertiser, valid until `Abort` or a connect outcome.
    pub disc_id: u32,
    pub transport: Transport,
    pub rssi: Option<i16>,
    pub adv: AdvData,
    pub platform_hint: Option<PeerPlatform>,
}

/// Radio availability (§3.2 `RadioState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RadioAvail {
    On,
    /// Bluetooth off / airplane mode.
    Off,
    Unauthorized,
    Unsupported,
}

impl RadioAvail {
    /// mesh-lab Bluetooth state.
    pub const fn lab(self) -> mesh_lab::BtState {
        match self {
            RadioAvail::On => mesh_lab::BtState::On,
            RadioAvail::Off => mesh_lab::BtState::Off,
            RadioAvail::Unauthorized => mesh_lab::BtState::Unauthorized,
            RadioAvail::Unsupported => mesh_lab::BtState::Unsupported,
        }
    }

    /// Snapshot Bluetooth state.
    pub const fn ui(self) -> mesh_api::BluetoothState {
        match self {
            RadioAvail::On => mesh_api::BluetoothState::On,
            RadioAvail::Off | RadioAvail::Unsupported => mesh_api::BluetoothState::Off,
            RadioAvail::Unauthorized => mesh_api::BluetoothState::Unauthorized,
        }
    }
}
