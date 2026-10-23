import CMeshAudio
import Foundation
import Synchronization

/// What the C audio host runs (maps to `cmesh_profile_t`).
public enum VoiceIOProfile: Sendable, Equatable {
    case voice
    case voiceRenderOnly
    case voiceBypass
    case outputOnly

    init(path: AudioPath) {
        switch path {
        case .full: self = .voice
        case .renderOnly: self = .voiceRenderOnly
        case .t5aBypass: self = .voiceBypass
        }
    }

    var c: cmesh_profile_t {
        switch self {
        case .voice: return CMESH_PROFILE_VOICE
        case .voiceRenderOnly: return CMESH_PROFILE_VOICE_RENDER_ONLY
        case .voiceBypass: return CMESH_PROFILE_VOICE_BYPASS
        case .outputOnly: return CMESH_PROFILE_OUTPUT_ONLY
        }
    }

    public var hasInput: Bool { self == .voice || self == .voiceBypass }
}

/// Hardware-side formats of the running unit, compared across route changes.
public struct HardwareFormat: Sendable, Equatable {
    public var inputRate: Double
    public var inputChannels: UInt32
    public var outputRate: Double
    public var outputChannels: UInt32
}

/// The C audio host behind a protocol, so the orchestration is unit-tested with a fake.
public protocol VoiceIOBackend: AnyObject, Sendable {
    /// Returns 0 or the failing OSStatus.
    func start(_ profile: VoiceIOProfile) -> Int32
    func stop()
    var isRunning: Bool { get }
    func setCaptureMuted(_ muted: Bool)
    var captureMuted: Bool { get }
    /// OS input callbacks so far (MuteWatchdog input).
    var inputCallbacks: UInt64 { get }
    var hardwareFormat: HardwareFormat? { get }
}

/// `CMeshAudio` as a `VoiceIOBackend` (process-global C state: one unit at a time).
public final class CMeshAudioBackend: VoiceIOBackend {
    public init() {}

    public func start(_ profile: VoiceIOProfile) -> Int32 { cmesh_start(profile.c) }
    public func stop() { _ = cmesh_stop() }
    public var isRunning: Bool { cmesh_is_running() }
    public func setCaptureMuted(_ muted: Bool) { cmesh_set_capture_muted(muted) }
    public var captureMuted: Bool { cmesh_capture_muted() }

    public var inputCallbacks: UInt64 {
        var c = CMeshIoCounters()
        cmesh_io_counters(&c)
        return c.input_callbacks
    }

    public var hardwareFormat: HardwareFormat? {
        var ir = 0.0, orr = 0.0
        var ic: UInt32 = 0, oc: UInt32 = 0
        guard cmesh_hw_format(&ir, &ic, &orr, &oc) else { return nil }
        return HardwareFormat(inputRate: ir, inputChannels: ic, outputRate: orr, outputChannels: oc)
    }
}

/// Lifecycle states published by `VoiceIOUnit` (WP-14 interface: Running / Interrupted /
/// Restarting / CaptureLost / RenderLost).
public enum VoiceIOState: Sendable, Equatable {
    case stopped
    case running
    case interrupted
    case restarting
    case captureLost
    case renderLost
}

/// Swift wrapper over the C VPIO host with a state publisher. The unit always starts with
/// the capture sink muted; only `unmute` (on `StartCapture`) lets capture reach the sink.
public final class VoiceIOUnit: Sendable {
    public let backend: any VoiceIOBackend
    private let st = Mutex<(state: VoiceIOState, profile: VoiceIOProfile?, listeners: [UUID: @Sendable (VoiceIOState) -> Void])>(
        (.stopped, nil, [:]))

    public init(backend: any VoiceIOBackend = CMeshAudioBackend()) {
        self.backend = backend
    }

    public var state: VoiceIOState { st.withLock { $0.state } }
    public var profile: VoiceIOProfile? { st.withLock { $0.profile } }

    /// Starts `profile` muted. Throws the OSStatus on failure (state unchanged).
    public func start(_ profile: VoiceIOProfile) throws(AudioPlatformError) {
        backend.setCaptureMuted(true)
        let status = backend.start(profile)
        guard status == 0 else { throw .unit(status) }
        st.withLock { $0.profile = profile }
        publish(.running)
    }

    /// Stops and restarts the current profile, keeping the mute state.
    public func restart() throws(AudioPlatformError) {
        guard let p = profile else { throw .notActive }
        let muted = backend.captureMuted
        backend.stop()
        let status = backend.start(p)
        backend.setCaptureMuted(muted)
        guard status == 0 else { throw .unit(status) }
    }

    public func stop() {
        backend.stop()
        backend.setCaptureMuted(true)
        st.withLock { $0.profile = nil }
        publish(.stopped)
    }

    public func unmute() { backend.setCaptureMuted(false) }
    public func mute() { backend.setCaptureMuted(true) }
    public var isMuted: Bool { backend.captureMuted }

    /// Sets the published state (driven by `PlatformAudio`'s state machine).
    public func publish(_ s: VoiceIOState) {
        let ls: [@Sendable (VoiceIOState) -> Void]? = st.withLock { v in
            guard v.state != s else { return nil }
            v.state = s
            return Array(v.listeners.values)
        }
        ls?.forEach { $0(s) }
    }

    /// Subscribes to state changes; returns a token for `unsubscribe`.
    @discardableResult
    public func subscribe(_ l: @escaping @Sendable (VoiceIOState) -> Void) -> UUID {
        let id = UUID()
        st.withLock { $0.listeners[id] = l }
        return id
    }

    public func unsubscribe(_ id: UUID) {
        st.withLock { _ = $0.listeners.removeValue(forKey: id) }
    }

    /// State changes as an async sequence.
    public var states: AsyncStream<VoiceIOState> {
        AsyncStream { cont in
            let id = subscribe { cont.yield($0) }
            cont.onTermination = { [weak self] _ in self?.unsubscribe(id) }
        }
    }
}
