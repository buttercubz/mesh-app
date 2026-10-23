import Foundation

/// Stable discovery ids per remote device (CoreBluetooth identifier), assigned on first
/// sight and kept for the driver's lifetime so the core can refer to a peer by `disc_id`.
public struct DiscoveryTable: Sendable {
    private var byPeer: [UUID: UInt32] = [:]
    private var byId: [UInt32: UUID] = [:]
    private var next: UInt32

    /// `firstId` lets central and peripheral drivers use disjoint ranges.
    public init(firstId: UInt32 = 1) {
        next = max(1, firstId)
    }

    public mutating func id(for peer: UUID) -> UInt32 {
        if let id = byPeer[peer] { return id }
        let id = next
        next &+= 1
        if next == 0 { next = 1 }
        byPeer[peer] = id
        byId[id] = peer
        return id
    }

    public func existingId(for peer: UUID) -> UInt32? { byPeer[peer] }
    public func peer(for id: UInt32) -> UUID? { byId[id] }
}

/// Connection bookkeeping of the central driver (DESIGN §9.3, §9.5): at most one
/// outstanding connect per transport, the 8 s native timeout (iOS `connect` never times
/// out by itself) unless the core armed a pending reconnect, and `Abort{ignore_s}`.
/// Pure and clock-injected so it is unit-tested without radios.
public struct ConnectTracker: Sendable {
    public struct Attempt: Sendable, Equatable {
        public var discId: UInt32
        public var startedAt: TimeInterval
    }

    public let timeout: TimeInterval
    public private(set) var current: Attempt?
    private var ignoredUntil: [UInt32: TimeInterval] = [:]
    private var armedUntil: [UInt32: TimeInterval] = [:]
    private var pending: Set<UInt32> = []

    public init(timeout: TimeInterval) {
        self.timeout = timeout
    }

    public enum BeginResult: Sendable, Equatable {
        /// Regular attempt: counts as the transport's one outstanding connect, 8 s cap.
        case start
        /// Pending reconnect (armed by the core): left outstanding by iOS until the armed
        /// window ends; does not occupy the one-per-transport slot.
        case startPending
        /// Another connect is outstanding (one per transport).
        case busy(UInt32)
        /// The core aborted this peer and asked to ignore it for a while.
        case ignored
    }

    /// Starts an attempt for `discId` unless another is outstanding or it is ignored.
    public mutating func begin(_ discId: UInt32, now: TimeInterval) -> BeginResult {
        if isIgnored(discId, now: now) { return .ignored }
        if isArmed(discId, now: now) {
            pending.insert(discId)
            return .startPending
        }
        if let c = current, c.discId != discId { return .busy(c.discId) }
        current = Attempt(discId: discId, startedAt: now)
        return .start
    }

    /// The attempt finished (connected, failed or cancelled).
    public mutating func finish(_ discId: UInt32) {
        if current?.discId == discId { current = nil }
        pending.remove(discId)
    }

    /// Pending reconnects whose armed window has closed: cancel their `connect`.
    public mutating func expiredPending(now: TimeInterval) -> [UInt32] {
        let done = pending.filter { !isArmed($0, now: now) }.sorted()
        for d in done { pending.remove(d) }
        return done
    }

    public func isPending(_ discId: UInt32) -> Bool { pending.contains(discId) }

    /// The regular attempt to cancel at `now`, if it exceeded the timeout.
    public mutating func expired(now: TimeInterval) -> UInt32? {
        guard let c = current, now - c.startedAt >= timeout else {
            return nil
        }
        current = nil
        return c.discId
    }

    /// Next instant at which `expired` may return an attempt (for timer scheduling).
    public func nextDeadline(now: TimeInterval) -> TimeInterval? {
        var deadlines = pending.compactMap { armedUntil[$0] }
        if let c = current { deadlines.append(c.startedAt + timeout) }
        return deadlines.min()
    }

    public mutating func ignore(_ discId: UInt32, seconds: UInt32, now: TimeInterval) {
        ignoredUntil[discId] = now + TimeInterval(seconds)
        finish(discId)
    }

    public func isIgnored(_ discId: UInt32, now: TimeInterval) -> Bool {
        (ignoredUntil[discId] ?? 0) > now
    }

    /// `ArmReconnect{peer, until_s}`: keep an outstanding connect without the 8 s cap.
    public mutating func arm(_ discId: UInt32, untilS: UInt32, now: TimeInterval) {
        if untilS == 0 {
            armedUntil.removeValue(forKey: discId)
        } else {
            armedUntil[discId] = now + TimeInterval(untilS)
        }
    }

    public func isArmed(_ discId: UInt32, now: TimeInterval) -> Bool {
        (armedUntil[discId] ?? 0) > now
    }

    /// Armed peers whose window is still open (to re-issue `connect` after a loss).
    public func armedPeers(now: TimeInterval) -> [UInt32] {
        armedUntil.filter { $0.value > now }.map(\.key).sorted()
    }

    public mutating func prune(now: TimeInterval) {
        ignoredUntil = ignoredUntil.filter { $0.value > now }
        armedUntil = armedUntil.filter { $0.value > now }
    }
}
