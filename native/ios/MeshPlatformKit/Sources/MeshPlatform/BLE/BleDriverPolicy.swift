import Foundation

/// Scan intensity override. The core decides the discovery mode; within `forming` the
/// driver only spends 100 % duty (duplicates, every advertisement reported) when it has
/// no link at all or is hunting a lost member (field test 1: continuous aggressive
/// scanning starved connection events and partitioned the group).
public enum ScanHint: UInt8, Sendable {
    /// Driver rule: duplicates while forming in the foreground with 0 BLE links, or while
    /// a reconnect to a lost peer is armed.
    case auto = 0
    /// The core is hunting an unreachable member: duplicates while forming in the foreground.
    case hunt = 1
    /// Never request duplicates.
    case quiet = 2
}

/// Pure scan decision of the central driver (unit-tested without radios).
public struct ScanPolicy: Sendable, Equatable {
    public var scan: Bool
    public var allowDuplicates: Bool

    public init(scan: Bool, allowDuplicates: Bool) {
        self.scan = scan
        self.allowDuplicates = allowDuplicates
    }

    /// - Parameters:
    ///   - bleLinks: live BLE links of this node, both roles.
    ///   - armedReconnects: peers with an armed `ArmReconnect` window (a lost member).
    public static func decide(mode: DiscoveryMode, foreground: Bool, bleLinks: Int,
                              armedReconnects: Int, hint: ScanHint) -> ScanPolicy {
        switch mode {
        case .off:
            return ScanPolicy(scan: false, allowDuplicates: false)
        case .stable:
            return ScanPolicy(scan: true, allowDuplicates: false)
        case .forming:
            let hunting: Bool
            switch hint {
            case .auto: hunting = bleLinks == 0 || armedReconnects > 0
            case .hunt: hunting = true
            case .quiet: hunting = false
            }
            return ScanPolicy(scan: true, allowDuplicates: foreground && hunting)
        }
    }

    /// Restart period of a filtered (no-duplicates) scan. Without duplicates CoreBluetooth
    /// reports each peripheral about once per scan session, so a peer that stays in range
    /// stops refreshing its candidate (the core expires candidates after 10 s and treats
    /// adverts older than 3 s as stale) and a peer that left at the range edge and came
    /// back is never reported again. Restarting the same scan every few seconds re-reports
    /// every peer in range (fresh RSSI, reappearance) at the filtered scan's radio cost,
    /// like bitchat's 5 s scan duty cycle; the 100 % duty that starved links in field
    /// test 1 stays off.
    public static let refreshInterval: TimeInterval = 4

    /// `true` when a filtered scan started at `startedAt` should be restarted at `now`.
    public func needsRefresh(startedAt: TimeInterval, now: TimeInterval) -> Bool {
        scan && !allowDuplicates && now - startedAt >= Self.refreshInterval
    }
}

/// Retry schedule for OS registrations that can fail transiently (L2CAP listener, GATT
/// service): 1, 2, 4, 8, 16, then every 30 s while the radio is on.
public struct RepublishBackoff: Sendable, Equatable {
    public private(set) var failures = 0
    /// Bumped on every reset so stale scheduled retries can recognise themselves.
    public private(set) var generation: UInt64 = 0

    public init() {}

    /// Records a failure and returns the delay before the next attempt.
    public mutating func failed() -> TimeInterval {
        failures += 1
        return min(30, pow(2, Double(failures - 1)))
    }

    public mutating func succeeded() {
        failures = 0
    }

    /// Radio off or the registration was replaced: pending retries become stale.
    public mutating func reset() {
        failures = 0
        generation &+= 1
    }
}

/// L2CAP listener lifecycle of the peripheral driver. The PSM reported to the core (and
/// therefore put in HELLO) is non-nil only while CoreBluetooth confirms a live listener;
/// any failure or unexpected unpublish reports nil (peers then use GATT) and schedules
/// a re-publish.
public struct L2capListenerState: Sendable, Equatable {
    public enum Action: Sendable, Equatable {
        /// Call `publishL2CAPChannel(withEncryption: false)` now.
        case publish
        /// Schedule `publish` after this delay, tagged with the backoff generation.
        case retry(after: TimeInterval, generation: UInt64)
        case none
    }

    public private(set) var psm: UInt16?
    public private(set) var publishing = false
    public private(set) var backoff = RepublishBackoff()

    public init() {}

    /// Radio on (or a retry fired): publish unless a listener exists or one is pending.
    public mutating func wantListener() -> Action {
        guard psm == nil, !publishing else { return .none }
        publishing = true
        return .publish
    }

    /// A scheduled retry fired; ignored when it belongs to an older generation.
    public mutating func retryFired(generation: UInt64) -> Action {
        guard generation == backoff.generation else { return .none }
        return wantListener()
    }

    /// `didPublishL2CAPChannel`.
    public mutating func published(psm newPsm: UInt16?, failed: Bool) -> Action {
        publishing = false
        if !failed, let newPsm, newPsm != 0 {
            psm = newPsm
            backoff.succeeded()
            return .none
        }
        psm = nil
        return .retry(after: backoff.failed(), generation: backoff.generation)
    }

    /// `didUnpublishL2CAPChannel` that this driver did not request.
    public mutating func unpublished() -> Action {
        psm = nil
        publishing = false
        return .retry(after: backoff.failed(), generation: backoff.generation)
    }

    /// Radio off / unauthorized: the listener is gone and pending retries are stale.
    public mutating func radioLost() {
        psm = nil
        publishing = false
        backoff.reset()
    }
}

/// Why the central cannot run `OpenLink` on a peer right now.
public enum OpenCheck: Sendable, Equatable {
    case ok
    /// The ACL connection is gone (or never completed).
    case notConnected
    /// The peer's `MESH_SVC` lacks `RX` (write without response) or `TX` (notify).
    case missingGattCharacteristics
    /// The HELLO advertised no L2CAP listener.
    case noPsm

    public static func check(via: OpenVia, connected: Bool, hasRx: Bool, hasTx: Bool) -> OpenCheck {
        guard connected else { return .notConnected }
        switch via {
        case .l2cap(let psm):
            return psm == 0 ? .noPsm : .ok
        case .gatt:
            return hasRx && hasTx ? .ok : .missingGattCharacteristics
        }
    }
}
