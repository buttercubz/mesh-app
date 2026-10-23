import Foundation
import MeshActivityShared

/// Interruption and mic-loss state machine (DESIGN §10.1.4, R2 mitigation). Pure and
/// clock-injected: it only decides; `PlatformAudio` executes the returned actions and
/// feeds their results back.
///
/// ```
/// RUNNING ─began─► INTERRUPTED (L1) ─ended+shouldResume─► RESTARTING ─ok─► RUNNING (L0, "mic back")
///                                                                     └fail─► CAPTURE_LOST (L2)
/// INTERRUPTED ─no "ended" 5 s─► retry every 5 s for 2 min, then every 30 s
/// RUNNING ─route change, format unchanged & running─► nothing
/// RUNNING ─route change, format changed / not running─► reconfigure ─fail─► CAPTURE_LOST
/// RUNNING ─watchdog stall > 1.5 s─► restart ─fail─► CAPTURE_LOST
/// any live state ─media services reset─► rebuild session + VPIO ─fail─► CAPTURE_LOST
/// CAPTURE_LOST: report CaptureLost, output-only fallback, notify, cue; resume → restart
/// output-only fallback fails too ─► RENDER_LOST (L3, relay-only)
/// ```
public struct MicLossStateMachine: Sendable {
    public enum State: Sendable, Equatable {
        case idle
        case running
        case interrupted(since: TimeInterval)
        case restarting(Origin)
        case captureLost
        case renderLost
    }

    /// What triggered a restart; decides where a failure goes.
    public enum Origin: Sendable, Equatable {
        case interruptionEnded
        case retry(since: TimeInterval)
        case routeChange
        case watchdog
        case mediaReset
        case userResume
    }

    public enum Event: Sendable, Equatable {
        /// The gesture-bound start succeeded (session active, VPIO running muted).
        case started
        case interruptionBegan(InterruptionReason)
        case interruptionEnded(shouldResume: Bool)
        case routeChanged(formatChanged: Bool, unitRunning: Bool)
        case watchdogStall
        case mediaServicesReset
        /// Result of the last restart / reconfigure / rebuild action.
        case restartResult(ok: Bool)
        /// Result of starting the output-only fallback.
        case outputOnlyResult(ok: Bool)
        /// Foreground tap or Resume mic (gesture-bound).
        case resumeRequested
        case tick
        case stopped
    }

    public enum Cue: Sendable, Equatable {
        case micPaused
        case micBack
    }

    public enum Action: Sendable, Equatable {
        case report(AudioIoState)
        case restartUnit
        case reconfigureUnit
        case rebuildSessionAndUnit
        case startOutputOnly
        case stopOutputOnly
        /// Live Activity "Mic paused — Resume" + time-sensitive local notification.
        case notifyCaptureLost
        case clearCaptureLostNotice
        case cue(Cue)
    }

    public static let firstRetryDelay: TimeInterval = 5
    public static let fastRetryInterval: TimeInterval = 5
    public static let fastRetryWindow: TimeInterval = 120
    public static let slowRetryInterval: TimeInterval = 30

    public private(set) var state: State = .idle
    private var lastRetry: TimeInterval?

    public init() {}

    /// Degradation ladder level of the current state (§7.8).
    public var ladder: LadderLevel {
        switch state {
        case .idle, .running: return .l0
        case .interrupted, .restarting: return .l1
        case .captureLost: return .l2
        case .renderLost: return .l3
        }
    }

    /// When the next `tick` may act (retry schedule while interrupted).
    public func nextDeadline() -> TimeInterval? {
        guard case .interrupted(let since) = state else { return nil }
        guard let last = lastRetry else { return since + Self.firstRetryDelay }
        let interval = last - since < Self.fastRetryWindow ? Self.fastRetryInterval : Self.slowRetryInterval
        return last + interval
    }

    public mutating func handle(_ event: Event, now: TimeInterval) -> [Action] {
        switch (state, event) {
        case (_, .stopped):
            state = .idle
            lastRetry = nil
            return []
        case (.idle, .started):
            state = .running
            return [.report(.running)]
        case (.running, .interruptionBegan(let reason)),
             (.restarting, .interruptionBegan(let reason)):
            state = .interrupted(since: now)
            lastRetry = nil
            return [.report(.interrupted(reason))]
        case (.interrupted, .interruptionEnded(let shouldResume)):
            guard shouldResume else { return [] } // keep retrying on the schedule
            state = .restarting(.interruptionEnded)
            return [.report(.restarting), .restartUnit]
        case (.interrupted(let since), .tick):
            guard let due = nextDeadline(), now >= due else { return [] }
            lastRetry = now
            state = .restarting(.retry(since: since))
            return [.report(.restarting), .restartUnit]
        case (.running, .routeChanged(let formatChanged, let unitRunning)):
            guard formatChanged || !unitRunning else { return [] }
            state = .restarting(.routeChange)
            return [.report(.restarting), .reconfigureUnit]
        case (.running, .watchdogStall):
            state = .restarting(.watchdog)
            return [.report(.restarting), .restartUnit]
        case (.running, .mediaServicesReset), (.interrupted, .mediaServicesReset),
             (.restarting, .mediaServicesReset), (.captureLost, .mediaServicesReset),
             (.renderLost, .mediaServicesReset):
            state = .restarting(.mediaReset)
            return [.report(.restarting), .rebuildSessionAndUnit]
        case (.restarting(let origin), .restartResult(let ok)):
            return restartFinished(origin: origin, ok: ok, now: now)
        case (.captureLost, .outputOnlyResult(let ok)):
            guard !ok else { return [] }
            state = .renderLost
            return [.report(.renderLost)]
        case (.captureLost, .resumeRequested), (.renderLost, .resumeRequested),
             (.interrupted, .resumeRequested):
            let wasLost = state == .captureLost || state == .renderLost
            state = .restarting(.userResume)
            return (wasLost ? [.stopOutputOnly] : []) + [.report(.restarting), .restartUnit]
        default:
            return []
        }
    }

    private mutating func restartFinished(origin: Origin, ok: Bool, now: TimeInterval) -> [Action] {
        if ok {
            state = .running
            lastRetry = nil
            var a: [Action] = [.report(.restored), .report(.running), .cue(.micBack)]
            if origin == .userResume || origin == .mediaReset { a.insert(.clearCaptureLostNotice, at: 0) }
            return a
        }
        if case .retry(let since) = origin {
            // The OS still holds the session (call in progress): keep waiting.
            state = .interrupted(since: since)
            return []
        }
        state = .captureLost
        return [.report(.captureLost), .startOutputOnly, .notifyCaptureLost, .cue(.micPaused)]
    }
}
