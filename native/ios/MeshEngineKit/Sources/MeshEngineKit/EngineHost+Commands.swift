import CMeshAudio
import Foundation
import MeshActivityShared
import MeshPlatform

extension EngineHost {
    // MARK: Plain commands

    /// JSON pass-through of mesh-api `UiCommand` → `CommandResult`. Rejects the ★
    /// gesture-gated commands with `useGestureCommand` (§3.2, §11.1).
    public func send(_ json: String) -> String {
        if GestureCommand.requiresGesture(json) {
            return CommandJSON.error("useGestureCommand")
        }
        let obj = CommandJSON.object(json)
        let type = obj?["type"] as? String
        if type == "ptt", let down = obj?["down"] as? Bool {
            // Also mirrored to the RT flags (§3.7), like the bridge's fast path.
            LiveActivityRtActions.setPttDown(down)
            flagsChanged()
        }
        let result = engineHandle.uiCommand(json: json)
        if CommandJSON.isOk(result), let obj {
            accepted(type: type, command: obj, result: result)
        }
        return result
    }

    /// For the ★ commands (goLive, resumeMic, labStart with audio ≠ off, audioCheck start):
    /// refuses unless the app is foreground-active, then synchronously performs the
    /// OS-level, gesture-bound work (activate the session and start VPIO muted, the
    /// location anchor when requested, the Live Activity) and only then forwards the
    /// command (§3.2). Any other command is forwarded as by `send`.
    ///
    /// Call it from the bridge's call path inside the user's tap, never from the main
    /// thread while it waits on the engine.
    public func gestureCommand(_ json: String) throws(EngineHostError) -> String {
        guard gestureForegroundCheck() else { throw .notForeground }
        guard let gesture = GestureCommand.classify(json) else {
            return forwardAccepted(json)
        }
        if GestureCommand.isRefusedWhileLive(gesture), state.withLock({ $0.rideLive }) {
            // The ride owns the session: the engine refuses (alreadyLive / wrongMode), and
            // re-running the gesture would restart a lost capture as a side effect.
            return forwardAccepted(json)
        }
        let share = MainHop.sync { [mainSide] in mainSide.notify.digest.shareDistance }
        let plan = GesturePlan.make(gesture, shareDistance: share)
        // 1. Audio session + VPIO (muted) inside the gesture.
        let activated: Bool
        do throws(AudioPlatformError) {
            activated = try audio.prepareGesture(plan)
        } catch {
            hooks.log(level: .error, msg: "gesture: audio session not started: \(error)")
            return CommandJSON.error("audioUnavailable")
        }
        // 2. Location anchor and Live Activity, before backgrounding.
        let started = MainHop.sync { [mainSide] () -> (location: Bool, activity: Bool) in
            var location = false
            if plan.locationAnchor, !mainSide.location.isActive {
                mainSide.startLocationAnchor()
                location = true
            }
            var activity = false
            if let a = plan.activity {
                activity = !mainSide.notify.hasActivity
                mainSide.notify.startActivity(a)
            }
            return (location, activity)
        }
        // 3. Only now the command reaches the engine.
        let result = engineHandle.uiCommand(json: json)
        if CommandJSON.isOk(result), let obj = CommandJSON.object(json) {
            accepted(type: obj["type"] as? String, command: obj, result: result)
        } else {
            if activated { audio.rollbackGesture() }
            MainHop.sync { [mainSide] in
                if started.location { mainSide.location.stop() }
                if started.activity { mainSide.notify.endActivity() }
            }
        }
        return result
    }

    /// `gestureCommand({type:"goLive"})` (F3: the explicit consent to capture).
    @discardableResult
    public func goLive() throws(EngineHostError) -> String {
        try gestureCommand(#"{"type":"goLive"}"#)
    }

    /// `gestureCommand({type:"resumeMic"})` (F5, ladder L2 → L0).
    @discardableResult
    public func resumeMic() throws(EngineHostError) -> String {
        try gestureCommand(#"{"type":"resumeMic"}"#)
    }

    // MARK: RT fast paths (§3.6)

    /// PTT: writes the RT flag bit 0 at once (no core round trip; T8) and sends
    /// `{type:"ptt"}`.
    public func ptt(_ down: Bool) {
        LiveActivityRtActions.setPttDown(down)
        _ = engineHandle.uiCommand(json: #"{"type":"ptt","down":\#(down)}"#)
        flagsChanged()
    }

    /// Toggles the PTT latch edge (RT flag bit 3), as the Live Activity Talk button does.
    public func setRtLatchToggle() {
        LiveActivityRtActions.perform(.latchTalk, resumeMic: {})
        flagsChanged()
    }

    /// Installs the Live Activity action handler (mute and latch write RT flags; Resume
    /// opens the app and resumes the mic once it is foreground-active). Call once at
    /// launch (done by `init` unless `registerActivityActions` is false).
    public func registerLiveActivityActions() {
        // Same as `LiveActivityRtActions.register(resumeMic:)`, plus a Live Activity
        // refresh after the flag write.
        LiveActivityActionHandler.register { [weak self] action in
            guard let host = self else { return }
            LiveActivityRtActions.perform(action) { host.resumeFromActivity() }
            host.flagsChanged()
        }
    }

    // MARK: Internals

    private func resumeFromActivity() {
        if gestureForegroundCheck() {
            _ = try? resumeMic()
        } else {
            // The intent opens the app; resume when it becomes foreground-active.
            state.withLock { $0.pendingResume = true }
        }
    }

    func flagsChanged() {
        MainHop.async { [mainSide] in mainSide.notify.flagsChanged() }
    }

    private func forwardAccepted(_ json: String) -> String {
        let result = engineHandle.uiCommand(json: json)
        if CommandJSON.isOk(result), let obj = CommandJSON.object(json) {
            accepted(type: obj["type"] as? String, command: obj, result: result)
        }
        return result
    }

    /// Side effects of accepted commands the host tracks itself.
    private func accepted(type: String?, command: [String: Any], result: String) {
        switch type {
        case "createGroup", "setVehicle":
            if let v = (command["vehicle"] as? String).flatMap(VehicleProfile.init(apiValue:)) {
                state.withLock { $0.vehicle = v }
            }
        case "goLive":
            state.withLock { $0.rideLive = true }
        case "labStart":
            let bench = command["bench"] as? String ?? ""
            let runId = CommandJSON.object(result)?["runId"] as? String
            MainHop.async { [mainSide] in mainSide.notify.labStarted(bench: bench, runId: runId) }
        default:
            break
        }
    }
}
