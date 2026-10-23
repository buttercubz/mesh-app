@preconcurrency import MeshCoreBindings
import Foundation
import MeshActivityShared
import MeshPlatform
import Synchronization

/// Executes the core's `AudioCommand`s and reports the platform's audio events back
/// (DESIGN §3.2 capture gating, §10.1.2, §10.1.4).
///
/// - `OpenSession` connects the RT sink (render can play; the C host keeps capture muted).
///   The session itself was activated by the gesture (`prepareGesture`); only a
///   foreground-active app may activate it here as a fallback.
/// - `StartCapture` only connects the sink and unmutes: it never activates a session, so a
///   late or duplicated `StartCapture` is harmless.
/// - `OutputOnlyFallback(true)` is idempotent with the platform's own fallback (the mic-loss
///   state machine starts it itself on CAPTURE_LOST).
///
/// Threading: `handle` runs on the engine thread and never blocks on the main thread.
final class AudioAdapter: Sendable {
    struct Callbacks: Sendable {
        /// The vehicle profile for session options (`.car` adds `.defaultToSpeaker`).
        var vehicle: @Sendable () -> VehicleProfile
        /// The core asked for a location anchor with this session (`location_anchor`).
        var wantsLocationAnchor: @Sendable () -> Void
        /// The session was closed (`CloseSession`): anchors and Live Activity end.
        var sessionClosed: @Sendable () -> Void
        /// Every platform audio event (Live Activity, resume notification, logs).
        var event: @Sendable (PlatformAudioEvent) -> Void
        var log: @Sendable (FfiLogLevel, String) -> Void
    }

    let audio: any AudioControl
    private let engine: EngineRef
    private let foreground: any ForegroundProvider
    private let calls: any CallStatePort
    private let cb: Callbacks
    private let lastProfile = Mutex<FfiAudioSessionProfile?>(nil)
    /// Last route reported (diagnostics `audioRoute.prev`).
    private let lastRoute = Mutex<AudioRouteInfo?>(nil)

    init(audio: any AudioControl, engine: EngineRef, foreground: any ForegroundProvider,
         calls: any CallStatePort, callbacks: Callbacks) {
        self.audio = audio
        self.engine = engine
        self.foreground = foreground
        self.calls = calls
        self.cb = callbacks
        audio.setListener { [weak self] e in self?.platformEvent(e) }
    }

    /// The profile of the last `OpenSession` (diagnostics, tests).
    var openedProfile: FfiAudioSessionProfile? { lastProfile.withLock { $0 } }

    // MARK: Gesture-bound start (bridge thread, inside the tap)

    /// The OS-level audio work of a gesture: activate + start VPIO muted. On a session that
    /// is already live the platform turns the same call into a restart of a lost capture
    /// (`resumeMic`). Returns whether this call activated a new session (so a refused
    /// command can roll it back). Throws when the session cannot be activated.
    func prepareGesture(_ plan: GesturePlan) throws(AudioPlatformError) -> Bool {
        guard let path = plan.path else { return false }
        let wasLive = audio.isLive
        do throws(AudioPlatformError) {
            try audio.activateAndStartMuted(profile: cb.vehicle(), path: path, mixable: plan.mixable)
        } catch {
            engine.note(.audioOpen(api: "vpio", path: path.rawValue, ok: false, err: Self.code(error)))
            throw error
        }
        engine.note(.audioOpen(api: "vpio", path: path.rawValue, ok: true, err: wasLive ? "alreadyLive" : nil))
        return !wasLive
    }

    /// Undoes a fresh `prepareGesture` when the engine refused the command.
    func rollbackGesture() {
        audio.close()
    }

    // MARK: Core commands (engine thread)

    func handle(_ cmd: FfiAudioCommand) {
        switch cmd {
        case .openSession(let profile):
            lastProfile.withLock { $0 = profile }
            openSession(profile)
        case .startCapture:
            audio.connectRtSink()
            audio.unmute()
        case .stopCapture:
            audio.mute()
        case .setRoutePreference(let pref):
            audio.setRoutePreference(RoutePreference(pref))
        case .outputOnlyFallback(let on):
            if on, audio.isCaptureLost { return }
            audio.setOutputOnlyFallback(on)
        case .closeSession:
            lastProfile.withLock { $0 = nil }
            audio.close()
            cb.sessionClosed()
        }
    }

    /// The product session (ride, audio check) is always mixable whatever the core asks
    /// (2026-10-23 deviation from §10.1.2, see `AudioSessionConfig.make`); only Lab runs
    /// honour the core's flag (T2 compares both).
    static func sessionMixable(_ p: FfiAudioSessionProfile) -> Bool {
        switch p.kind {
        case .ride, .check: return true
        case .labRender, .labFull: return p.mixable
        }
    }

    private func openSession(_ p: FfiAudioSessionProfile) {
        if !audio.isLive {
            if foreground.isForegroundActive {
                let path: AudioPath = p.capture ? (p.bypassVoiceProcessing ? .t5aBypass : .full) : .renderOnly
                do throws(AudioPlatformError) {
                    try audio.activateAndStartMuted(profile: cb.vehicle(), path: path,
                                                    mixable: Self.sessionMixable(p))
                } catch {
                    engine.note(.audioOpen(api: "vpio", path: path.rawValue, ok: false, err: Self.code(error)))
                    cb.log(.error, "audio: OpenSession could not activate the session: \(error)")
                    report(p.capture ? .captureLost : .renderLost)
                    return
                }
            } else {
                // No gesture prepared it and the app is not foreground-active: iOS refuses to
                // start a recording session from the background. Report it, never fake it.
                engine.note(.audioOpen(api: "vpio", path: p.capture ? "full" : "renderOnly", ok: false,
                                       err: "backgroundWithoutGesture"))
                cb.log(.error, "audio: OpenSession in the background without a gesture")
                report(p.capture ? .captureLost : .renderLost)
                return
            }
        }
        audio.connectRtSink()
        if p.locationAnchor { cb.wantsLocationAnchor() }
    }

    // MARK: Platform events (audio control queue)

    private func platformEvent(_ e: PlatformAudioEvent) {
        switch e {
        case .io(let s):
            report(FfiAudioIoState(s, onCall: calls.onCall))
        case .route(let r):
            engine.engine?.onAudioRoute(r: FfiAudioRoute(r))
        case .ladder, .captureLostNotice, .cue:
            break
        }
        diagnose(e)
        cb.event(e)
    }

    /// The diagnostics note of a platform audio event (ridelog/1, src `plat`).
    private func diagnose(_ e: PlatformAudioEvent) {
        switch e {
        case .io(let s):
            engine.note(.audioIo(s))
        case .route(let r):
            let prev = lastRoute.withLock { p in
                defer { p = r }
                return p
            }
            if prev != r { engine.note(.audioRoute(r, prev: prev)) }
        case .ladder(let l):
            engine.note(.ladder(l.rawValue))
        case .captureLostNotice(let on):
            engine.note(.banner("ride.micPaused", on: on))
        case .cue:
            break
        }
    }

    /// A short code for an audio platform error (no free text).
    static func code(_ e: AudioPlatformError) -> String {
        switch e {
        case .session(let op, let code): return "session:\(op):\(code)"
        case .unit(let st): return "unit:\(st)"
        case .clientFormat: return "clientFormat"
        case .notActive: return "notActive"
        }
    }

    private func report(_ s: FfiAudioIoState) {
        engine.engine?.onAudioIo(s: s)
    }
}
