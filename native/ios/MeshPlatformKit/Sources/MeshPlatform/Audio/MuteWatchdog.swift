import Foundation

/// Detects a silent capture stop (DESIGN §10.1.4): if the capture callback counter has not
/// advanced for more than `stall` seconds while capture should be running, it reports a
/// stall once, then re-arms when the counter moves again. Pure; the caller polls `check`.
public struct MuteWatchdog: Sendable {
    public static let defaultStall: TimeInterval = 1.5

    public let stall: TimeInterval
    private var lastCount: UInt64?
    private var lastChange: TimeInterval = 0
    private var fired = false

    public init(stall: TimeInterval = MuteWatchdog.defaultStall) {
        self.stall = stall
    }

    /// Starts watching from `now` (unit just started or restarted).
    public mutating func arm(count: UInt64, now: TimeInterval) {
        lastCount = count
        lastChange = now
        fired = false
    }

    /// Stops watching (unit stopped, capture intentionally off).
    public mutating func disarm() {
        lastCount = nil
        fired = false
    }

    public var isArmed: Bool { lastCount != nil }

    /// Feeds the current callback counter; returns true exactly once per stall.
    public mutating func check(count: UInt64, now: TimeInterval) -> Bool {
        guard let last = lastCount else { return false }
        if count != last {
            lastCount = count
            lastChange = now
            fired = false
            return false
        }
        guard !fired, now - lastChange > stall else { return false }
        fired = true
        return true
    }
}
