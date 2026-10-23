import Foundation
import Synchronization

/// Actions the Live Activity buttons can trigger (contract C9).
public enum LiveActivityAction: String, Sendable, CaseIterable {
    /// Toggle the hard mute (RT flag bit 1).
    case toggleMute
    /// Toggle the PTT latch (RT flag bit 3 edge).
    case latchTalk
    /// Restart capture after a loss; the intent opens the app first (capture cannot start
    /// in the background).
    case resumeMic
}

/// Registration point for the app's Live Activity action handler.
///
/// The app registers one handler at launch (MeshEngineKit/WP-15 through the iOS bridge,
/// WP-19). `ToggleMuteIntent` and `LatchTalkIntent` run in the app process, which already
/// captures, and the handler only writes `mesh_rt_set_flags` bits, so they are legal from
/// the lock screen (DESIGN §10.1.5).
public enum LiveActivityActionHandler {
    public typealias Handler = @Sendable (LiveActivityAction) -> Void

    private static let slot = Mutex<Handler?>(nil)

    /// Installs (or replaces) the handler. Pass `nil` to remove it.
    public static func register(_ handler: Handler?) {
        slot.withLock { $0 = handler }
    }

    public static var isRegistered: Bool {
        slot.withLock { $0 != nil }
    }

    /// Calls the registered handler. Returns `false` when none is registered (the intent
    /// then reports failure instead of silently doing nothing).
    @discardableResult
    public static func handle(_ action: LiveActivityAction) -> Bool {
        guard let h = slot.withLock({ $0 }) else { return false }
        h(action)
        return true
    }
}

/// Thrown by an intent whose action has no registered handler.
public struct LiveActivityHandlerMissing: Error, Sendable, Equatable {
    public let action: LiveActivityAction
    public init(action: LiveActivityAction) { self.action = action }
}
