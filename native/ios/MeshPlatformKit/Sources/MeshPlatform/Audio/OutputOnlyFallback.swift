import Foundation
import Synchronization

/// Ladder L2 (DESIGN §10.1.4 CAPTURE_LOST step 2): when capture cannot be restarted in the
/// background, a RemoteIO output-only unit keeps the rider hearing the group. It replaces
/// the VPIO unit (one unit at a time) and is stopped before capture restarts.
public final class OutputOnlyFallback: Sendable {
    private let unit: VoiceIOUnit
    private let active = Mutex(false)

    public init(unit: VoiceIOUnit) {
        self.unit = unit
    }

    public var isActive: Bool { active.withLock { $0 } }

    /// Starts the output-only unit. Returns false when render failed too (ladder L3).
    @discardableResult
    public func start() -> Bool {
        unit.backend.stop()
        do {
            try unit.start(.outputOnly)
        } catch {
            active.withLock { $0 = false }
            return false
        }
        active.withLock { $0 = true }
        return true
    }

    public func stop() {
        let was = active.withLock { a -> Bool in
            let w = a
            a = false
            return w
        }
        if was { unit.backend.stop() }
    }
}
