import CoreBluetooth
import Foundation
import Synchronization

/// Radio availability reported to the core (`RadioAvail`, DESIGN §3.2).
public enum RadioState: UInt8, Sendable, Equatable {
    case on = 0
    case off = 1
    case unauthorized = 2
    case unsupported = 3
}

/// Maps CoreBluetooth manager states and errors to the core's radio vocabulary and
/// reports each distinct BLE state once (`on_platform_sample` / `RadioState{ble}`).
///
/// `poweredOff` (Bluetooth off, airplane mode) → `.off`: the core puts discovery on hold,
/// marks BLE links down with `RadioOff` and raises `bluetoothOff`. `unauthorized` →
/// `bluetoothDenied`. `CBError.connectionLimitReached` → `LinkDownReason.limitReached`.
public final class RadioStateMonitor: Sendable {
    public typealias Listener = @Sendable (RadioState) -> Void

    private struct State {
        var central: RadioState?
        var peripheral: RadioState?
        var reported: RadioState?
        var listener: Listener?
    }

    private let state = Mutex(State())

    public init(listener: Listener? = nil) {
        state.withLock { $0.listener = listener }
    }

    public func setListener(_ l: Listener?) {
        state.withLock { $0.listener = l }
    }

    /// `nil` for transitional states (`unknown`, `resetting`): nothing is reported.
    public static func map(_ s: CBManagerState) -> RadioState? {
        switch s {
        case .poweredOn: return .on
        case .poweredOff: return .off
        case .unauthorized: return .unauthorized
        case .unsupported: return .unsupported
        case .unknown, .resetting: return nil
        @unknown default: return nil
        }
    }

    public enum Role: Sendable { case central, peripheral }

    /// Feed a manager state; the combined state is the worse of the two roles (both
    /// managers share one radio, so they normally agree).
    public func update(_ role: Role, _ s: CBManagerState) {
        guard let mapped = Self.map(s) else { return }
        let (changed, listener): (RadioState?, Listener?) = state.withLock { st in
            switch role {
            case .central: st.central = mapped
            case .peripheral: st.peripheral = mapped
            }
            let combined = Self.combine(st.central, st.peripheral)
            guard combined != st.reported else { return (nil, nil) }
            st.reported = combined
            return (combined, st.listener)
        }
        if let changed, let listener { listener(changed) }
    }

    public var current: RadioState? { state.withLock { $0.reported } }

    static func combine(_ a: RadioState?, _ b: RadioState?) -> RadioState? {
        let known = [a, b].compactMap { $0 }
        guard !known.isEmpty else { return nil }
        // Severity order: unsupported > unauthorized > off > on.
        let rank: (RadioState) -> Int = { s in
            switch s {
            case .on: return 0
            case .off: return 1
            case .unauthorized: return 2
            case .unsupported: return 3
            }
        }
        return known.max { rank($0) < rank($1) }
    }

    /// Link-down reason for a CoreBluetooth disconnect / failure.
    public static func downReason(for error: (any Error)?, cancelledLocally: Bool) -> LinkDownReason {
        guard let error else { return cancelledLocally ? .localClose : .peerClosed }
        let ns = error as NSError
        if ns.domain == CBErrorDomain, let code = CBError.Code(rawValue: ns.code) {
            switch code {
            case .connectionLimitReached, .tooManyLEPairedDevices: return .limitReached
            case .connectionTimeout: return .timeout
            case .peripheralDisconnected: return .peerClosed
            case .operationCancelled: return .localClose
            default: return .error(Int32(clamping: ns.code))
            }
        }
        return .error(Int32(clamping: ns.code))
    }
}
