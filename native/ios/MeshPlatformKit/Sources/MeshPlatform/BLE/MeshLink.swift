import Foundation

// Platform link interface (contract C9, DESIGN §9.1). The core sees links as data and
// events; this layer owns every OS object and never parses or frames bytes (all framing
// is in Rust, mesh-wire). The Kotlin mirror lives in WP-16.

/// Transport kind of a link (mirrors the core's `LinkKind`).
public enum LinkKind: UInt8, Sendable, CaseIterable {
    case bleL2cap = 0
    case bleGatt = 1
    case wifiAware = 2
    case tunnel = 3
}

/// Which side opened the link.
public enum LinkRole: UInt8, Sendable {
    case initiator = 0
    case responder = 1
}

/// What one `linkData` call carries (DESIGN §4.2).
public enum LinkUnit: UInt8, Sendable {
    /// Arbitrary stream chunks (L2CAP, Tunnel).
    case stream = 0
    /// One ATT value per call (GATT).
    case message = 1
    /// One datagram per call (Wi-Fi Aware).
    case datagram = 2
}

/// Pre-connection hint about the peer's OS.
public enum PeerPlatform: UInt8, Sendable {
    case unknown = 0
    case ios = 1
    case android = 2
}

/// Result of a non-blocking `MeshLink.send` (`FfiSendOutcome`).
public enum SendOutcome: UInt8, Sendable {
    /// The whole unit was taken; the link may accept more.
    case accepted = 0
    /// Nothing was taken; `linkWritable` follows when there is room again.
    case wouldBlock = 1
    /// The link is gone.
    case noLink = 2
}

/// Why a link went down (`FfiLinkDownReason`).
public enum LinkDownReason: Sendable, Equatable {
    case timeout
    case peerClosed
    case localClose
    case radioOff
    /// iOS `CBError.connectionLimitReached`: the core lowers this device's BLE max (§3.2).
    case limitReached
    case error(Int32)
}

/// Link description passed with `linkUp` (`LinkInfo`, §9.1).
public struct LinkInfo: Sendable, Equatable {
    public var id: UInt32
    public var kind: LinkKind
    public var role: LinkRole
    /// Largest unit the core may hand to `send` (L2CAP: 2 + 1024; GATT: ATT max write).
    public var maxFrame: UInt16
    public var unit: LinkUnit
    public var peerPlatformHint: PeerPlatform
    /// Discovery that led to this link (initiator side), if any.
    public var discId: UInt32?

    public init(id: UInt32, kind: LinkKind, role: LinkRole, maxFrame: UInt16, unit: LinkUnit,
                peerPlatformHint: PeerPlatform, discId: UInt32? = nil) {
        self.id = id
        self.kind = kind
        self.role = role
        self.maxFrame = maxFrame
        self.unit = unit
        self.peerPlatformHint = peerPlatformHint
        self.discId = discId
    }
}

/// Discovery transport.
public enum DiscoveryTransport: UInt8, Sendable {
    case ble = 0
    case wifiAware = 1
}

/// Advertising tokens readable before connecting (§9.4). Every field is optional because
/// a backgrounded iPhone advertises only its service UUID.
public struct AdvTokens: Sendable, Equatable {
    /// `group_tag[0..2]` from an iPhone local name, or the full 4 bytes from Android.
    public var groupTag: Data
    /// Top 14 bits of the node token (iPhone local name or derived from Android data).
    public var tokenPrefix14: UInt16
    /// Full node token (Android scan response only).
    public var nodeToken: Data?
    /// Android service-data flags (b0 android, b1 lab).
    public var android: Bool
    public var lab: Bool

    public init(groupTag: Data, tokenPrefix14: UInt16, nodeToken: Data? = nil,
                android: Bool = false, lab: Bool = false) {
        self.groupTag = groupTag
        self.tokenPrefix14 = tokenPrefix14
        self.nodeToken = nodeToken
        self.android = android
        self.lab = lab
    }
}

/// One discovered peer (`Discovery`, §3.2).
public struct Discovery: Sendable, Equatable {
    public var discId: UInt32
    public var transport: DiscoveryTransport
    public var rssi: Int16?
    public var tokens: AdvTokens?
    public var platformHint: PeerPlatform

    public init(discId: UInt32, transport: DiscoveryTransport, rssi: Int16?, tokens: AdvTokens?,
                platformHint: PeerPlatform) {
        self.discId = discId
        self.transport = transport
        self.rssi = rssi
        self.tokens = tokens
        self.platformHint = platformHint
    }
}

/// Discovery intensity chosen by the core (§5.7, §9.4).
public enum DiscoveryMode: UInt8, Sendable {
    /// No scanning, no advertising.
    case off = 0
    /// Under target or members missing: aggressive scan (duplicates in the foreground).
    case forming = 1
    /// At target: low-duty scanning.
    case stable = 2
}

/// What this node advertises (§9.4). The platform turns it into OS-specific payloads.
public struct AdvConfig: Sendable, Equatable {
    public var groupTag: Data
    public var nodeToken: Data
    public var lab: Bool
    /// Accept incoming connections (peripheral role active).
    public var connectable: Bool

    public init(groupTag: Data, nodeToken: Data, lab: Bool, connectable: Bool = true) {
        self.groupTag = groupTag
        self.nodeToken = nodeToken
        self.lab = lab
        self.connectable = connectable
    }
}

/// How to open a link after HELLO.
public enum OpenVia: Sendable, Equatable {
    case l2cap(psm: UInt16)
    case gatt
}

/// Per-link connection priority hint (Android uses it; iOS records it for Lab).
public enum LinkPriority: UInt8, Sendable {
    case balanced = 0
    case high = 1
    case lowPower = 2
}

/// Radio commands executed by the platform (`RadioCommand`, §3.2). Every decision is made
/// by the core; drivers only execute.
public enum RadioCommand: Sendable, Equatable {
    case setDiscovery(mode: DiscoveryMode, adv: AdvConfig?)
    case setHello(Data)
    case connect(discId: UInt32)
    case openLink(discId: UInt32, via: OpenVia)
    case abort(discId: UInt32, ignoreS: UInt32)
    case close(link: UInt32, reason: UInt8)
    case setLinkParams(link: UInt32, priority: LinkPriority)
    /// Keep an outstanding `connect` to a lost peer (T3 variant, §9.5).
    case armReconnect(discId: UInt32, untilS: UInt32)
    /// Phase 4 (WP-24).
    case awareUpgrade(peer: UInt32, op: UInt8)
    /// v2 hook; no implementation in v1.
    case tunnelOpen(rendezvous: Data, peerHint: Data)
}

/// A live link owned by a driver.
public protocol MeshLink: AnyObject, Sendable {
    var id: UInt32 { get }
    var kind: LinkKind { get }
    /// Non-blocking: either takes the whole unit or returns `.wouldBlock` / `.noLink`.
    func send(_ bytes: Data) -> SendOutcome
    func close(reason: UInt8)
}

/// Executes radio commands.
public protocol LinkDriver: AnyObject, Sendable {
    func apply(_ cmd: RadioCommand)
}

/// Receives link events; implemented by MeshEngineKit's `EngineHost`.
public protocol LinkSink: AnyObject, Sendable {
    func linkUp(_ link: any MeshLink, info: LinkInfo)
    func linkDown(_ id: UInt32, reason: LinkDownReason)
    func linkData(_ id: UInt32, _ bytes: Data)
    func linkWritable(_ id: UInt32)
    func linkMetrics(_ id: UInt32, rssi: Int16?, attMtu: UInt16?)
    func discovered(_ d: Discovery)
    func helloRead(_ discId: UInt32, _ bytes: Data)
    /// A `Connect{disc_id}` failed before any link existed (e.g. `connectionLimitReached`
    /// → `.limitReached`, which lowers this device's BLE max, §3.2). Default: ignored.
    func connectFailed(_ discId: UInt32, reason: LinkDownReason)
}

extension LinkSink {
    public func connectFailed(_ discId: UInt32, reason: LinkDownReason) {}
}

/// v2 cellular hook (§9.8): designed, not built. No implementation ships in v1.
public protocol TunnelProvider: AnyObject, Sendable {
    func open(rendezvous: Data, peerHint: Data) -> (any MeshLink)?
}
