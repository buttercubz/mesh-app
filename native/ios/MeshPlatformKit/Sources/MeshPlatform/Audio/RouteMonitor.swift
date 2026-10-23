import AVFAudio
import Foundation
import Synchronization

/// Maps the session route to `AudioRouteInfo` (DESIGN §7.8: the platform reports every
/// route change; the core switches encoder bandwidth and the route chip) and reports each
/// distinct route once.
public final class RouteMonitor: Sendable {
    private let port: any AudioSessionPort
    private let last = Mutex<AudioRouteInfo?>(nil)

    public init(port: any AudioSessionPort) {
        self.port = port
    }

    /// The current route, mapped.
    public func current() -> AudioRouteInfo {
        Self.map(port.route)
    }

    /// Returns the route if it differs from the last one reported, else nil.
    public func refresh() -> AudioRouteInfo? {
        let r = current()
        return last.withLock { prev in
            guard prev != r else { return nil }
            prev = r
            return r
        }
    }

    public func reset() {
        last.withLock { $0 = nil }
    }

    /// Pure mapping (unit-tested with synthetic routes).
    public static func map(_ route: AudioRouteSnapshot) -> AudioRouteInfo {
        typealias P = AVAudioSession.Port
        let ins = Set(route.inputs.map(\.portType))
        let outs = Set(route.outputs.map(\.portType))
        guard !outs.isEmpty || !ins.isEmpty else { return .none }
        let rate = normalisedRate(route.sampleRate)

        let output: AudioOutputKind
        if outs.contains(P.carAudio.rawValue) {
            output = .carAudio
        } else if outs.contains(P.bluetoothHFP.rawValue) {
            output = .bluetoothHfp
        } else if outs.contains(P.bluetoothLE.rawValue) {
            output = .bluetoothLe
        } else if outs.contains(P.headphones.rawValue) || outs.contains(P.usbAudio.rawValue) {
            output = .wired
        } else if outs.contains(P.builtInSpeaker.rawValue) {
            output = .speaker
        } else if outs.contains(P.builtInReceiver.rawValue) {
            output = .receiver
        } else if outs.contains(P.airPlay.rawValue) {
            output = .airplay
        } else {
            output = .none
        }

        let kind: AudioRouteKind
        if outs.contains(P.carAudio.rawValue) {
            kind = .carBluetooth
        } else if ins.contains(P.bluetoothHFP.rawValue) || outs.contains(P.bluetoothHFP.rawValue) {
            kind = rate >= 16_000 ? .hfpWideband : .hfpNarrowband
        } else if ins.contains(P.bluetoothLE.rawValue) || outs.contains(P.bluetoothLE.rawValue) {
            kind = .leAudio
        } else if ins.contains(P.headsetMic.rawValue) || outs.contains(P.headphones.rawValue)
                    || ins.contains(P.usbAudio.rawValue) {
            kind = .wired
        } else if outs.contains(P.builtInReceiver.rawValue) {
            kind = .builtinReceiver
        } else if outs.contains(P.builtInSpeaker.rawValue) {
            kind = .builtinSpeaker
        } else {
            kind = .none
        }

        let headset = [AudioRouteKind.hfpWideband, .hfpNarrowband, .leAudio, .wired, .carBluetooth]
            .contains(kind)
        return AudioRouteInfo(kind: kind, inputRate: ins.isEmpty ? 0 : rate, output: output,
                              headsetPresent: headset)
    }

    /// Rounds a hardware rate to the nearest of 8/16/24/32/44.1/48 kHz (0 when unknown).
    static func normalisedRate(_ r: Double) -> UInt32 {
        guard r > 0 else { return 0 }
        let rates: [UInt32] = [8_000, 16_000, 24_000, 32_000, 44_100, 48_000]
        return rates.min { abs(Double($0) - r) < abs(Double($1) - r) } ?? 0
    }
}
