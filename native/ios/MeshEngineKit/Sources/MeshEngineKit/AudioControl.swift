import AVFAudio
import CMeshAudio
import Foundation
import MeshCoreFFI
import MeshPlatform

/// The audio orchestration the adapter drives (`PlatformAudio` in production, a recorder
/// in tests).
public protocol AudioControl: AnyObject, Sendable {
    /// Gesture-bound: activate the session and start VPIO with the capture sink muted.
    func activateAndStartMuted(profile: VehicleProfile, path: AudioPath, mixable: Bool) throws(AudioPlatformError)
    /// Registers the core's `mesh_rt_*` as the C audio host's sink (never activates).
    func connectRtSink()
    func disconnectRtSink()
    /// Lets capture reach the sink (`StartCapture`). Harmless when not live.
    func unmute()
    func mute()
    func setOutputOnlyFallback(_ on: Bool)
    func setRoutePreference(_ pref: RoutePreference)
    /// Resume after a capture loss (gesture-bound foreground tap).
    func resume()
    /// `CloseSession`: stops the unit and deactivates the session.
    func close()
    func appDidBecomeActive()
    /// A session was activated and has not been closed.
    var isLive: Bool { get }
    /// Capture (or render) is lost and the output-only fallback ran.
    var isCaptureLost: Bool { get }
    func setListener(_ listener: (@Sendable (PlatformAudioEvent) -> Void)?)
}

extension PlatformAudio: AudioControl {
    public func connectRtSink() {
        // The cmesh function types are signature-identical to mesh_rt_* (contract C5);
        // MeshRtStats and CMeshRtStats share one layout (sizeof 48, tested in WP-14).
        connectSink(push: mesh_rt_capture, pull: mesh_rt_render, flags: mesh_rt_set_flags,
                    stats: unsafeBitCast(mesh_rt_stats as @convention(c) (UnsafeMutablePointer<MeshRtStats>?) -> Void,
                                         to: cmesh_stats_fn.self))
    }

    public func disconnectRtSink() {
        disconnectSink()
    }

    public var isLive: Bool { state != .idle }

    public var isCaptureLost: Bool {
        switch state {
        case .captureLost, .renderLost: return true
        default: return false
        }
    }

    public func setRoutePreference(_ pref: RoutePreference) {
        let port = session.port
        let inputs = port.availableInputs
        let hfp = inputs.first { $0.portType == AVAudioSession.Port.bluetoothHFP.rawValue }
        let builtIn = inputs.first { $0.portType == AVAudioSession.Port.builtInMic.rawValue }
        session.setHfpInputWanted(pref != .speaker)
        switch pref {
        case .auto, .headset:
            try? port.setPreferredInput(uid: hfp?.uid)
        case .speaker:
            try? port.setPreferredInput(uid: builtIn?.uid)
        }
        // The output override exists only on the real session (no-op on test ports).
        if port is SystemAudioSession, session.isActive {
            try? AVAudioSession.sharedInstance().overrideOutputAudioPort(pref == .speaker ? .speaker : .none)
        }
    }
}
