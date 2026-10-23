import Foundation
import MeshActivityShared

// Action execution of `PlatformAudio`; everything here runs on `ctl`.
extension PlatformAudio {
    func emit(_ e: PlatformAudioEvent) {
        listener?(e)
    }

    func process(_ e: AudioSessionEvent) {
        let now = clock()
        switch e {
        case .interruptionBegan(let reason):
            watchdog.disarm()
            run(fsm.handle(.interruptionBegan(reason), now: now))
        case .interruptionEnded(let shouldResume):
            run(fsm.handle(.interruptionEnded(shouldResume: shouldResume), now: now))
        case .routeChanged:
            // Keep the helmet mic: a music app or a reconnect can leave the route on
            // A2DP / built-in while HFP is available (mixable session, §10.1.2 deviation).
            session.reassertPreferredInput()
            if let r = routes.refresh() { emit(.route(r)) }
            let fmt = unit.backend.hardwareFormat
            let changed = fmt != lastFormat
            run(fsm.handle(.routeChanged(formatChanged: changed, unitRunning: unit.backend.isRunning),
                           now: now))
        case .mediaServicesLost:
            watchdog.disarm()
        case .mediaServicesReset:
            run(fsm.handle(.mediaServicesReset, now: now))
        }
    }

    func poll() {
        let now = clock()
        if fsm.state == .running, voiceProfile.hasInput,
           watchdog.check(count: unit.backend.inputCallbacks, now: now) {
            run(fsm.handle(.watchdogStall, now: now))
        }
        run(fsm.handle(.tick, now: now))
    }

    func armWatchdog() {
        if voiceProfile.hasInput, fsm.state == .running {
            watchdog.arm(count: unit.backend.inputCallbacks, now: clock())
        } else {
            watchdog.disarm()
        }
    }

    func startTimer() {
        guard timer == nil else { return }
        let t = DispatchSource.makeTimerSource(queue: ctl)
        t.schedule(deadline: .now() + pollInterval, repeating: pollInterval)
        t.setEventHandler { [weak self] in self?.poll() }
        timer = t
        t.resume()
    }

    /// Executes actions; results are fed back into the state machine until it settles.
    func run(_ initial: [MicLossStateMachine.Action]) {
        var queue = initial
        while !queue.isEmpty {
            let a = queue.removeFirst()
            queue += execute(a)
        }
        let l = fsm.ladder
        if l != lastLadder {
            lastLadder = l
            emit(.ladder(l))
        }
    }

    private func execute(_ a: MicLossStateMachine.Action) -> [MicLossStateMachine.Action] {
        let now = clock()
        switch a {
        case .report(let s):
            emit(.io(s))
            unit.publish(Self.unitState(s))
            return []
        case .restartUnit:
            return fsm.handle(.restartResult(ok: restartVoice(reactivate: true)), now: now)
        case .reconfigureUnit:
            return fsm.handle(.restartResult(ok: restartVoice(reactivate: false)), now: now)
        case .rebuildSessionAndUnit:
            return fsm.handle(.restartResult(ok: restartVoice(reactivate: true)), now: now)
        case .startOutputOnly:
            watchdog.disarm()
            return fsm.handle(.outputOnlyResult(ok: fallback.start()), now: now)
        case .stopOutputOnly:
            fallback.stop()
            return []
        case .notifyCaptureLost:
            emit(.captureLostNotice(true))
            return []
        case .clearCaptureLostNotice:
            emit(.captureLostNotice(false))
            return []
        case .cue(let c):
            emit(.cue(c))
            return []
        }
    }

    /// Uninit, reconfigure, init and start the voice profile again (muted unless capture
    /// was wanted). Returns false on any failure ('!rec' included).
    private func restartVoice(reactivate: Bool) -> Bool {
        fallback.stop()
        if reactivate {
            do { try session.reactivate() } catch { return false }
        }
        unit.backend.stop()
        do { try unit.start(voiceProfile) } catch { return false }
        if captureWanted, voiceProfile.hasInput { unit.unmute() }
        lastFormat = unit.backend.hardwareFormat
        if let r = routes.refresh() { emit(.route(r)) }
        watchdog.arm(count: unit.backend.inputCallbacks, now: clock())
        return true
    }

    static func unitState(_ s: AudioIoState) -> VoiceIOState {
        switch s {
        case .running, .restored: return .running
        case .interrupted: return .interrupted
        case .restarting: return .restarting
        case .captureLost: return .captureLost
        case .renderLost: return .renderLost
        }
    }
}
