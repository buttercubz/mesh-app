import AVFAudio
import Foundation
import Synchronization

/// Session-level events that drive the mic-loss state machine (DESIGN §10.1.4).
public enum AudioSessionEvent: Sendable, Equatable {
    case interruptionBegan(InterruptionReason)
    /// Interruption over; `shouldResume` from iOS 26 `.shouldResume` or iOS 27
    /// `resumptionRecommendation == .shouldResume`.
    case interruptionEnded(shouldResume: Bool)
    /// `AVAudioSession.RouteChangeReason` raw value.
    case routeChanged(reason: UInt)
    case mediaServicesLost
    case mediaServicesReset
}

/// Applies the §10.1.2 configuration and turns session notifications into
/// `AudioSessionEvent`s: iOS 27 `didBecomeInactive` / `resumptionRecommendation`, the
/// iOS 26 `interruptionNotification`, route changes and media-services loss/reset.
public final class AudioSessionController: Sendable {
    public typealias Listener = @Sendable (AudioSessionEvent) -> Void

    /// Observer tokens are opaque and only passed back to the (thread-safe) center.
    private struct Tokens: @unchecked Sendable {
        var list: [any NSObjectProtocol] = []
    }

    private struct State {
        var observers = Tokens()
        var listener: Listener?
        var active = false
        var config: AudioSessionConfig?
        /// The rider's input preference (`SetRoutePreference`): false while "phone mic"
        /// is chosen, so route-change handling never drags the input back to the helmet.
        var hfpWanted = true
    }

    public let port: any AudioSessionPort
    private let center: NotificationCenter
    private let state = Mutex(State())

    public init(port: any AudioSessionPort = SystemAudioSession(), center: NotificationCenter = .default) {
        self.port = port
        self.center = center
    }

    deinit {
        stopObserving()
    }

    public var isActive: Bool { state.withLock { $0.active } }
    public var config: AudioSessionConfig? { state.withLock { $0.config } }

    /// Configures and activates the session (gesture-bound: called inside the Go-live /
    /// Lab-start tap by the bridge, never from `StartCapture`).
    public func activate(_ config: AudioSessionConfig) throws(AudioPlatformError) {
        try step("setCategory") { try port.setCategory(config.category, mode: config.mode, options: config.options) }
        try step("setPreferredSampleRate") { try port.setPreferredSampleRate(config.preferredSampleRate) }
        try step("setPreferredIOBufferDuration") {
            try port.setPreferredIOBufferDuration(config.preferredIOBufferDuration)
        }
        try step("setPrefersNoInterruptions") {
            try port.setPrefersNoInterruptionsFromSystemAlerts(config.prefersNoInterruptionsFromSystemAlerts)
        }
        try step("setActive") { try port.setActive(true) }
        if config.preferHfpInput,
           let hfp = port.availableInputs.first(where: { $0.portType == AVAudioSession.Port.bluetoothHFP.rawValue }) {
            // Not fatal: the route stays on whatever iOS picked.
            do {
                try port.setPreferredInput(uid: hfp.uid)
            } catch {
                PlatformLog.audio.error("setPreferredInput(HFP) failed: \(error.localizedDescription)")
            }
        }
        state.withLock {
            $0.active = true
            $0.config = config
        }
    }

    /// Records the rider's input preference (`SetRoutePreference`: auto/headset → true,
    /// speaker → false) for `reassertPreferredInput`.
    public func setHfpInputWanted(_ wanted: Bool) {
        state.withLock { $0.hfpWanted = wanted }
    }

    /// After a route change: when an HFP input (helmet) is available but the route lost it
    /// (helmet reconnected, or a music app moved the headset to A2DP, which has no mic),
    /// asks for it again so capture keeps a headset mic. Returns true when it asked. No-op
    /// when the route already uses HFP, so the resulting route change does not loop.
    @discardableResult
    public func reassertPreferredInput() -> Bool {
        let (active, wanted) = state.withLock { ($0.active, ($0.config?.preferHfpInput ?? false) && $0.hfpWanted) }
        guard active, wanted else { return false }
        let hfp = AVAudioSession.Port.bluetoothHFP.rawValue
        guard !port.route.inputs.contains(where: { $0.portType == hfp }),
              let target = port.availableInputs.first(where: { $0.portType == hfp }) else { return false }
        do {
            try port.setPreferredInput(uid: target.uid)
        } catch {
            PlatformLog.audio.error("re-preferring HFP input failed: \(error.localizedDescription)")
            return false
        }
        return true
    }

    /// Re-applies the last configuration (media services reset, restart after a loss).
    public func reactivate() throws(AudioPlatformError) {
        guard let c = config else { throw .notActive }
        try activate(c)
    }

    public func deactivate() {
        do {
            try port.setActive(false)
        } catch {
            PlatformLog.audio.error("setActive(false) failed: \(error.localizedDescription)")
        }
        state.withLock { $0.active = false }
    }

    private func step(_ op: String, _ body: () throws -> Void) throws(AudioPlatformError) {
        do {
            try body()
        } catch {
            throw .session(op, (error as NSError).code)
        }
    }

    // MARK: Notifications

    public func startObserving(_ listener: @escaping Listener) {
        stopObserving()
        var tokens = Tokens()
        let emit: @Sendable (AudioSessionEvent?) -> Void = { [weak self] e in
            guard let e, let l = self?.state.withLock({ $0.listener }) else { return }
            l(e)
        }
        if #available(iOS 27.0, *) {
            tokens.list.append(center.addObserver(forName: AVAudioSession.didBecomeInactiveNotification,
                                             object: nil, queue: nil) { n in
                let ctx = n.userInfo?[AVAudioSession.deactivationContextKey] as? AVAudioSession.DeactivationContext
                emit(Self.deactivationEvent(source: ctx?.source.rawValue ?? 0,
                                            reason: ctx?.interruptionContext?.reason.rawValue))
            })
            tokens.list.append(center.addObserver(forName: AVAudioSession.resumptionRecommendationNotification,
                                             object: nil, queue: nil) { n in
                let ctx = n.userInfo?[AVAudioSession.resumptionContextKey] as? AVAudioSession.ResumptionContext
                emit(.interruptionEnded(shouldResume: ctx?.recommendation == .shouldResume))
            })
        } else {
            tokens.list.append(center.addObserver(forName: AVAudioSession.interruptionNotification,
                                             object: nil, queue: nil) { n in
                emit(Self.legacyInterruptionEvent(n.userInfo ?? [:]))
            })
        }
        tokens.list.append(center.addObserver(forName: AVAudioSession.routeChangeNotification,
                                         object: nil, queue: nil) { n in
            emit(Self.routeChangeEvent(n.userInfo ?? [:]))
        })
        tokens.list.append(center.addObserver(forName: AVAudioSession.mediaServicesWereLostNotification,
                                         object: nil, queue: nil) { _ in emit(.mediaServicesLost) })
        tokens.list.append(center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification,
                                         object: nil, queue: nil) { _ in emit(.mediaServicesReset) })
        state.withLock {
            $0.listener = listener
            $0.observers = tokens
        }
    }

    public func stopObserving() {
        let tokens = state.withLock { s -> Tokens in
            let t = s.observers
            s.observers = Tokens()
            s.listener = nil
            return t
        }
        for t in tokens.list { center.removeObserver(t) }
    }

    // MARK: Pure parsers (unit-tested)

    /// iOS 26 `interruptionNotification` userInfo → event.
    public static func legacyInterruptionEvent(_ info: [AnyHashable: Any]) -> AudioSessionEvent? {
        guard let typeRaw = (info[AVAudioSessionInterruptionTypeKey] as? NSNumber)?.uintValue else { return nil }
        switch typeRaw {
        case 1: // .began
            let reason = (info[AVAudioSessionInterruptionReasonKey] as? NSNumber)?.uintValue
            return .interruptionBegan(interruptionReason(reason))
        case 0: // .ended
            let opts = (info[AVAudioSessionInterruptionOptionKey] as? NSNumber)?.uintValue ?? 0
            return .interruptionEnded(shouldResume: opts & 1 != 0) // .shouldResume
        default:
            return nil
        }
    }

    /// iOS 27 `didBecomeInactive` → event. Source 1 (app) is our own deactivation and is
    /// ignored; source 2 (system) is an interruption.
    public static func deactivationEvent(source: Int, reason: UInt?) -> AudioSessionEvent? {
        source == 2 ? .interruptionBegan(interruptionReason(reason)) : nil
    }

    public static func interruptionReason(_ raw: UInt?) -> InterruptionReason {
        switch raw {
        case 0: return .otherSession
        case 1: return .appWasSuspended
        case 2: return .builtInMicMuted
        case 4: return .routeDisconnected
        default: return raw == nil ? .otherSession : .unknown
        }
    }

    public static func routeChangeEvent(_ info: [AnyHashable: Any]) -> AudioSessionEvent? {
        guard let r = (info[AVAudioSessionRouteChangeReasonKey] as? NSNumber)?.uintValue else { return nil }
        return .routeChanged(reason: r)
    }
}
