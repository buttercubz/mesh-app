import CMeshAudio
import Foundation
import MeshActivityShared

/// Events for the engine host (MeshEngineKit maps them to `on_audio_io`, `on_audio_route`,
/// Live Activity / notifications and cues).
public enum PlatformAudioEvent: Sendable, Equatable {
    case io(AudioIoState)
    case route(AudioRouteInfo)
    case ladder(LadderLevel)
    /// Show (true) or clear (false) "Mic paused — Resume" (Live Activity + local notification).
    case captureLostNotice(Bool)
    case cue(MicLossStateMachine.Cue)
}

/// iOS audio orchestration (DESIGN §3.2 capture gating, §10.1.2, §10.1.4).
///
/// The start API is split: `activateAndStartMuted` is the OS-level, gesture-bound step the
/// bridge performs synchronously inside the tap (activate the session, start VPIO with the
/// sink muted); `connectSink` + `unmute` are what `StartCapture` does and never activate
/// anything, so a late or duplicated `StartCapture` is harmless. VPIO is never stopped
/// during a ride; talk modes gate transmission after capture.
///
/// Threading: all state is confined to the serial `ctl` queue.
public final class PlatformAudio: @unchecked Sendable {
    public typealias Listener = @Sendable (PlatformAudioEvent) -> Void

    public let session: AudioSessionController
    public let unit: VoiceIOUnit
    public let routes: RouteMonitor
    let fallback: OutputOnlyFallback
    let clock: @Sendable () -> TimeInterval
    let ctl = DispatchQueue(label: "mesh.audio.ctl", qos: .userInitiated)
    let pollInterval: TimeInterval

    // Confined to `ctl`.
    var fsm = MicLossStateMachine()
    var watchdog = MuteWatchdog()
    var voiceProfile: VoiceIOProfile = .voice
    var captureWanted = false
    var lastFormat: HardwareFormat?
    var lastLadder: LadderLevel = .l0
    var listener: Listener?
    var timer: DispatchSourceTimer?

    public init(session: AudioSessionController = AudioSessionController(),
                unit: VoiceIOUnit = VoiceIOUnit(), pollInterval: TimeInterval = 0.25,
                clock: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
        self.session = session
        self.unit = unit
        self.routes = RouteMonitor(port: session.port)
        self.fallback = OutputOnlyFallback(unit: unit)
        self.pollInterval = pollInterval
        self.clock = clock
    }

    public func setListener(_ l: Listener?) {
        ctl.sync { listener = l }
    }

    public var state: MicLossStateMachine.State { ctl.sync { fsm.state } }
    public var ladder: LadderLevel { ctl.sync { fsm.ladder } }

    // MARK: Gesture-bound start (bridge, inside the tap)

    /// Activates the session and starts VPIO with the capture sink muted. Must be called
    /// synchronously inside a foreground gesture (`gestureCommand`).
    public func activateAndStartMuted(profile: VehicleProfile, path: AudioPath,
                                      mixable: Bool = true) throws(AudioPlatformError) {
        let result: Result<Void, AudioPlatformError> = ctl.sync {
            if fsm.state != .idle {
                // Already live (e.g. resumeMic): a gesture restarts a lost capture.
                run(fsm.handle(.resumeRequested, now: clock()))
                return .success(())
            }
            do throws(AudioPlatformError) {
                try session.activate(AudioSessionConfig.make(profile: profile, mixable: mixable))
                voiceProfile = VoiceIOProfile(path: path)
                try unit.start(voiceProfile)
            } catch {
                session.deactivate()
                return .failure(error)
            }
            captureWanted = false
            lastFormat = unit.backend.hardwareFormat
            session.startObserving { [weak self] e in
                guard let self else { return }
                self.ctl.async { [weak self] in self?.process(e) }
            }
            run(fsm.handle(.started, now: clock()))
            if let r = routes.refresh() { emit(.route(r)) }
            armWatchdog()
            startTimer()
            return .success(())
        }
        try result.get()
    }

    // MARK: StartCapture / StopCapture (core commands; never activate anything)

    /// Registers the RT sink (the core's `mesh_rt_*`, signature-identical to the cmesh
    /// function types). `stats` is `mesh_rt_stats` cast with `unsafeBitCast`.
    public func connectSink(push: cmesh_push_fn?, pull: cmesh_pull_fn?, flags: cmesh_flags_fn?,
                            stats: cmesh_stats_fn?) {
        cmesh_set_sink(push, pull, flags, stats, sinkToken)
    }

    public func disconnectSink() {
        _ = cmesh_clear_sink(sinkToken)
    }

    var sinkToken: UnsafeMutableRawPointer { Unmanaged.passUnretained(self).toOpaque() }

    /// Lets capture reach the sink. Harmless when not live (it only records the wish).
    public func unmute() {
        ctl.sync {
            captureWanted = true
            if fsm.state == .running, voiceProfile.hasInput { unit.unmute() }
        }
    }

    public func mute() {
        ctl.sync {
            captureWanted = false
            unit.mute()
        }
    }

    /// `AudioCommand::OutputOnlyFallback(bool)` from the core.
    public func setOutputOnlyFallback(_ on: Bool) {
        ctl.sync {
            if on {
                if !fallback.start() { run(fsm.handle(.outputOnlyResult(ok: false), now: clock())) }
            } else {
                fallback.stop()
            }
        }
    }

    /// Resume mic (gesture-bound foreground tap or the Resume button).
    public func resume() {
        ctl.sync { run(fsm.handle(.resumeRequested, now: clock())) }
    }

    /// `CloseSession`: stops everything and deactivates the session.
    public func close() {
        ctl.sync {
            timer?.cancel()
            timer = nil
            session.stopObserving()
            fallback.stop()
            unit.stop()
            session.deactivate()
            _ = cmesh_clear_sink(sinkToken)
            watchdog.disarm()
            captureWanted = false
            run(fsm.handle(.stopped, now: clock()))
            lastLadder = .l0
        }
    }

    /// App returned to the foreground: a lost capture restarts (allowed in the foreground).
    public func appDidBecomeActive() {
        ctl.sync {
            switch fsm.state {
            case .captureLost, .renderLost: run(fsm.handle(.resumeRequested, now: clock()))
            default: break
            }
        }
    }

    // MARK: Test and timer entry points (on `ctl`)

    /// Feeds one session event synchronously (tests; notifications arrive via `ctl.async`).
    public func handle(_ e: AudioSessionEvent) {
        ctl.sync { process(e) }
    }

    /// Runs one watchdog / retry poll synchronously (tests; the timer calls `poll`).
    public func pollNow() {
        ctl.sync { poll() }
    }
}
