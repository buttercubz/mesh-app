import Foundation

/// Vehicle profile; selects the session options (DESIGN §1.4, §10.1.2).
public enum VehicleProfile: String, Sendable, CaseIterable {
    case motorcycle
    case car
}

/// What the VPIO unit runs (Lab `audio` + T5a, DESIGN §12.1).
public enum AudioPath: String, Sendable, CaseIterable {
    /// Capture and render (Go live, Lab `full`).
    case full
    /// Render only (Lab `renderOnly`: keeps the `audio` anchor, plays bench tones).
    case renderOnly
    /// Capture and render with `BypassVoiceProcessing = 1` (T5a only).
    case t5aBypass
}

/// Why the session was interrupted.
public enum InterruptionReason: String, Sendable, Equatable {
    /// Another session was activated (phone call, Siri, an alarm, another app).
    case otherSession
    case appWasSuspended
    case builtInMicMuted
    case routeDisconnected
    case unknown
}

/// Audio IO state reported to the core (`AudioIoState`, §3.2).
public enum AudioIoState: Sendable, Equatable {
    case running
    case interrupted(InterruptionReason)
    case restarting
    case captureLost
    case renderLost
    case restored
}

/// Audio route kind; raw values match mesh-api `AudioRouteKind`.
public enum AudioRouteKind: String, Sendable, CaseIterable {
    case hfpWideband
    case hfpNarrowband
    case leAudio
    case builtinSpeaker
    case builtinReceiver
    case wired
    case carBluetooth
    case none
}

/// Output side of the route.
public enum AudioOutputKind: String, Sendable {
    case bluetoothHfp
    case bluetoothLe
    case speaker
    case receiver
    case wired
    case carAudio
    case airplay
    case none
}

/// `AudioRouteInfo` (§3.2): kind, real capture rate, output kind, headset present.
public struct AudioRouteInfo: Sendable, Equatable {
    public var kind: AudioRouteKind
    /// Capture bandwidth of the route in Hz (8000 on CVSD HFP; the core then encodes NB).
    public var inputRate: UInt32
    public var output: AudioOutputKind
    public var headsetPresent: Bool

    public init(kind: AudioRouteKind, inputRate: UInt32, output: AudioOutputKind, headsetPresent: Bool) {
        self.kind = kind
        self.inputRate = inputRate
        self.output = output
        self.headsetPresent = headsetPresent
    }

    public static let none = AudioRouteInfo(kind: .none, inputRate: 0, output: .none, headsetPresent: false)
}

/// Errors from the audio platform layer.
public enum AudioPlatformError: Error, Equatable, Sendable {
    /// An AVAudioSession call failed (operation name, NSError code).
    case session(String, Int)
    /// The AudioUnit failed to start (OSStatus).
    case unit(Int32)
    /// The VPIO client format could not be set to 16 kHz mono Int16.
    case clientFormat
    /// Operation requires an active session.
    case notActive
}
