import Foundation
import Synchronization

/// The OS side of a GATT data path: central → write-without-response on `RX`, peripheral
/// → notify on `TX` (DESIGN §9.3). `trySend` must not block; it returns false when the OS
/// has no room (the writable signal follows through `GattLink.readyToSend`).
public protocol GattTxPort: AnyObject, Sendable {
    func trySend(_ data: Data) -> Bool
    /// Largest ATT value the OS accepts for this path.
    var maxUnit: Int { get }
    /// Releases OS resources (cancel the connection or forget the central).
    func shutdown()
}

/// A GATT fallback link. One `send` = one ATT value; the core produces chunks of
/// `hdr u8 ‖ chunk` sized to `maxFrame` and reassembles inbound values (§4.2), so this
/// class moves opaque values only.
public final class GattLink: MeshLink, @unchecked Sendable {
    public let id: UInt32
    public let kind: LinkKind = .bleGatt
    public let role: LinkRole

    private struct State {
        var closed = false
        var wantWritable = false
    }

    // `state` is the only mutable state (Mutex); `port` and `registry` are Sendable.
    private let state = Mutex(State())
    private let port: any GattTxPort
    private let registry: LinkRegistry

    public init(id: UInt32, role: LinkRole, port: any GattTxPort, registry: LinkRegistry) {
        self.id = id
        self.role = role
        self.port = port
        self.registry = registry
    }

    public var maxFrame: UInt16 { UInt16(clamping: port.maxUnit) }

    public func send(_ bytes: Data) -> SendOutcome {
        if state.withLock({ $0.closed }) { return .noLink }
        if port.trySend(bytes) { return .accepted }
        state.withLock { $0.wantWritable = true }
        return .wouldBlock
    }

    public func close(reason: UInt8) {
        down(.localClose)
    }

    /// OS says there is room again (`peripheralIsReady(toSendWriteWithoutResponse:)` or
    /// `peripheralManagerIsReady(toUpdateSubscribers:)`).
    public func readyToSend() {
        let notify = state.withLock { s -> Bool in
            guard !s.closed, s.wantWritable else { return false }
            s.wantWritable = false
            return true
        }
        if notify { registry.writable(id) }
    }

    /// One inbound ATT value (notification on TX or write to RX).
    public func received(_ value: Data) {
        guard !state.withLock({ $0.closed }) else { return }
        registry.data(id, value)
    }

    /// Tears the link down once and reports `linkDown`.
    public func down(_ reason: LinkDownReason) {
        let first = state.withLock { s -> Bool in
            let was = s.closed
            s.closed = true
            return !was
        }
        guard first else { return }
        port.shutdown()
        registry.down(id, reason: reason)
    }

    public var isClosed: Bool { state.withLock { $0.closed } }
}
