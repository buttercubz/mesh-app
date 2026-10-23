import AVFAudio
import Foundation

/// The exact session configuration of DESIGN §10.1.2, as data so it is unit-tested.
public struct AudioSessionConfig: Sendable, Equatable {
    public var category: AVAudioSession.Category
    public var mode: AVAudioSession.Mode
    public var options: AVAudioSession.CategoryOptions
    /// Hint only: the VPIO client format guarantees 16 kHz to the core.
    public var preferredSampleRate: Double
    public var preferredIOBufferDuration: TimeInterval
    public var prefersNoInterruptionsFromSystemAlerts: Bool
    /// Prefer an HFP input when one is available (helmet headset).
    public var preferHfpInput: Bool

    /// - Parameters:
    ///   - profile: `.car` adds `.defaultToSpeaker`.
    ///   - mixable: `.mixWithOthers`. **Product default since the 2026-10-23 field test**
    ///     (deviation from DESIGN §10.1.2, which shipped non-mixable): riders always play
    ///     music, and a non-mixable session is interrupted as soon as a music app
    ///     (Spotify) activates its own session; a background app cannot restart recording,
    ///     so the intercom collapsed. Mixable, the music keeps playing over the same
    ///     HFP route and the VPIO unit ducks it only while someone talks (advanced ducking,
    ///     minimum level, `cmesh_unit.c`), Cardo-like. `.duckOthers` is deliberately not
    ///     set: it would duck the music for the whole ride.
    ///     Trade-off (DTS): a mixable recording session cannot (re)activate in the
    ///     background after a phone call ends; that case keeps the existing mic-loss
    ///     ladder (retries, output-only fallback, CaptureLost notification and the
    ///     gesture-bound "tap to resume"). `false` survives only as the Lab T2 comparison.
    public static func make(profile: VehicleProfile, mixable: Bool = true) -> AudioSessionConfig {
        var options: AVAudioSession.CategoryOptions = [.allowBluetoothHFP]
        if profile == .car { options.insert(.defaultToSpeaker) }
        if mixable { options.insert(.mixWithOthers) }
        return AudioSessionConfig(
            category: .playAndRecord,
            mode: .voiceChat,
            options: options,
            preferredSampleRate: 16_000,
            preferredIOBufferDuration: 0.010,
            prefersNoInterruptionsFromSystemAlerts: true,
            preferHfpInput: true
        )
    }

    /// `bluetoothHighQualityRecording` is never used (not for real time, §10.1.2).
    public var usesHighQualityBluetoothRecording: Bool {
        options.contains(.bluetoothHighQualityRecording)
    }

    public var isMixable: Bool { options.contains(.mixWithOthers) }
}
