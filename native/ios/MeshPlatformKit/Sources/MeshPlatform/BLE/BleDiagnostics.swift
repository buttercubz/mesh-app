import CoreBluetooth
import Foundation

/// An OS error as reported by CoreBluetooth (`CBErrorDomain`, `CBATTErrorDomain`, ...).
public struct BleErrorInfo: Sendable, Equatable {
    public var domain: String
    public var code: Int

    public init(domain: String, code: Int) {
        self.domain = domain
        self.code = code
    }

    public init?(_ error: (any Error)?) {
        guard let error else { return nil }
        let ns = error as NSError
        self.init(domain: ns.domain, code: ns.code)
    }
}

/// Which driver emitted a diagnostics event.
public enum BleRole: UInt8, Sendable {
    case central = 0
    case peripheral = 1
}

/// BLE driver events for the diagnostics recorder (ridelog). They describe what the
/// drivers did and what the OS answered; they are never needed for correctness.
public enum BleDiagEvent: Sendable, Equatable {
    /// `CBManagerState` raw value of the central or peripheral manager.
    case radioState(role: BleRole, state: Int)
    /// Scanning (re)configured: `on == false` means stopped.
    case scan(mode: DiscoveryMode, on: Bool, allowDuplicates: Bool)
    /// Advertising (re)configured.
    case advertising(mode: DiscoveryMode, on: Bool)
    /// `connect` issued (`pending`: armed reconnect without the 8 s cap).
    case connectAttempt(discId: UInt32, pending: Bool)
    /// `connect` skipped: another attempt is outstanding, the peer is ignored, or it is
    /// still disconnecting (`deferred`, issued after the disconnect completes).
    case connectSkipped(discId: UInt32, why: String)
    /// `didConnect` (`error == nil`), `didFailToConnect`, or the native 8 s timeout
    /// (`timedOut`).
    case connectResult(discId: UInt32, ok: Bool, timedOut: Bool, error: BleErrorInfo?)
    /// GATT setup step failed before a link existed (services, characteristics, HELLO).
    case setupFailed(discId: UInt32, step: String, error: BleErrorInfo?)
    case helloRead(discId: UInt32, bytes: Int)
    /// `OpenLink` executed with the transport the core chose.
    case openAttempt(discId: UInt32, via: OpenVia)
    /// The data path could not be opened; reported to the core as `connectFailed`.
    case openFailed(discId: UInt32, via: OpenVia, error: BleErrorInfo?)
    /// A link is up (either role); `mtu` is the ATT MTU when known (GATT and central).
    case linkUp(link: UInt32, discId: UInt32?, kind: LinkKind, role: LinkRole,
                maxFrame: UInt16, mtu: UInt16?)
    case linkDown(link: UInt32, reason: LinkDownReason, error: BleErrorInfo?)
    /// The ACL connection to a peripheral ended (central role).
    case disconnected(discId: UInt32, local: Bool, reconnecting: Bool, error: BleErrorInfo?)
    /// Periodic ATT MTU / RSSI sample of a central link.
    case linkSample(link: UInt32, rssi: Int16?, mtu: UInt16?)
    /// L2CAP listener published (`psm != nil`) or gone; `retryIn` when a retry is scheduled.
    case l2capListener(psm: UInt16?, error: BleErrorInfo?, retryIn: TimeInterval?)
    /// The GATT service was (re)added.
    case serviceAdded(ok: Bool, error: BleErrorInfo?, retryIn: TimeInterval?)
    /// A link refused an outbound unit larger than its ATT value limit (dropped).
    case oversizeUnit(link: UInt32, bytes: Int, max: Int)
}

/// Receives `BleDiagEvent`s. Called on the driver's serial queue: it must return quickly
/// and must not call back into the driver synchronously.
public typealias BleDiagSink = @Sendable (BleDiagEvent) -> Void
