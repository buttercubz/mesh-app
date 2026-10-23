import AVFAudio
import Foundation

/// A port of the audio route (value type, so route logic is testable without hardware).
public struct AudioPortInfo: Sendable, Equatable {
    /// `AVAudioSession.Port` raw value (e.g. "BluetoothHFP").
    public var portType: String
    public var uid: String
    public var name: String

    public init(portType: String, uid: String = "", name: String = "") {
        self.portType = portType
        self.uid = uid
        self.name = name
    }
}

/// Snapshot of the current route.
public struct AudioRouteSnapshot: Sendable, Equatable {
    public var inputs: [AudioPortInfo]
    public var outputs: [AudioPortInfo]
    /// Hardware sample rate of the session.
    public var sampleRate: Double

    public init(inputs: [AudioPortInfo], outputs: [AudioPortInfo], sampleRate: Double) {
        self.inputs = inputs
        self.outputs = outputs
        self.sampleRate = sampleRate
    }
}

/// The subset of `AVAudioSession` the platform layer uses; faked in tests (Swift
/// conventions: a protocol in front of every OS service).
public protocol AudioSessionPort: AnyObject, Sendable {
    func setCategory(_ category: AVAudioSession.Category, mode: AVAudioSession.Mode,
                     options: AVAudioSession.CategoryOptions) throws
    func setPreferredSampleRate(_ rate: Double) throws
    func setPreferredIOBufferDuration(_ seconds: TimeInterval) throws
    func setPrefersNoInterruptionsFromSystemAlerts(_ value: Bool) throws
    func setActive(_ active: Bool) throws
    var availableInputs: [AudioPortInfo] { get }
    func setPreferredInput(uid: String?) throws
    var route: AudioRouteSnapshot { get }
}

/// `AVAudioSession.sharedInstance()` behind `AudioSessionPort`.
public final class SystemAudioSession: AudioSessionPort, @unchecked Sendable {
    // AVAudioSession is documented as thread-safe; this wrapper holds no state of its own.
    private let session: AVAudioSession

    public init(session: AVAudioSession = .sharedInstance()) {
        self.session = session
    }

    public func setCategory(_ category: AVAudioSession.Category, mode: AVAudioSession.Mode,
                            options: AVAudioSession.CategoryOptions) throws {
        try session.setCategory(category, mode: mode, options: options)
    }

    public func setPreferredSampleRate(_ rate: Double) throws {
        try session.setPreferredSampleRate(rate)
    }

    public func setPreferredIOBufferDuration(_ seconds: TimeInterval) throws {
        try session.setPreferredIOBufferDuration(seconds)
    }

    public func setPrefersNoInterruptionsFromSystemAlerts(_ value: Bool) throws {
        try session.setPrefersNoInterruptionsFromSystemAlerts(value)
    }

    public func setActive(_ active: Bool) throws {
        try session.setActive(active, options: active ? [] : [.notifyOthersOnDeactivation])
    }

    public var availableInputs: [AudioPortInfo] {
        (session.availableInputs ?? []).map(Self.info)
    }

    public func setPreferredInput(uid: String?) throws {
        let port = uid.flatMap { u in session.availableInputs?.first { $0.uid == u } }
        try session.setPreferredInput(port)
    }

    public var route: AudioRouteSnapshot {
        let r = session.currentRoute
        return AudioRouteSnapshot(inputs: r.inputs.map(Self.info), outputs: r.outputs.map(Self.info),
                                  sampleRate: session.sampleRate)
    }

    static func info(_ p: AVAudioSessionPortDescription) -> AudioPortInfo {
        AudioPortInfo(portType: p.portType.rawValue, uid: p.uid, name: p.portName)
    }
}
