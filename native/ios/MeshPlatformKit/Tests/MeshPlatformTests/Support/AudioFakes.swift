import AVFAudio
import Foundation
@testable import MeshPlatform
import Synchronization

/// Records every AVAudioSession call; can be told to fail one operation.
final class FakeSessionPort: AudioSessionPort {
    struct State {
        var calls: [String] = []
        var failOn: String?
        var inputs: [AudioPortInfo] = []
        var route = AudioRouteSnapshot(inputs: [AudioPortInfo(portType: "MicrophoneBuiltIn")],
                                       outputs: [AudioPortInfo(portType: "Receiver")], sampleRate: 16_000)
        var category: AVAudioSession.Category?
        var mode: AVAudioSession.Mode?
        var options: AVAudioSession.CategoryOptions = []
        var preferredInput: String?
        var active = false
    }

    let s = Mutex(State())
    var calls: [String] { s.withLock { $0.calls } }

    private func call(_ name: String) throws {
        let fail = s.withLock { st -> Bool in
            st.calls.append(name)
            return st.failOn == name
        }
        if fail { throw NSError(domain: "fake.session", code: 561_145_187) } // '!rec'
    }

    func setCategory(_ category: AVAudioSession.Category, mode: AVAudioSession.Mode,
                     options: AVAudioSession.CategoryOptions) throws {
        try call("setCategory")
        s.withLock {
            $0.category = category
            $0.mode = mode
            $0.options = options
        }
    }
    func setPreferredSampleRate(_ rate: Double) throws { try call("rate:\(Int(rate))") }
    func setPreferredIOBufferDuration(_ seconds: TimeInterval) throws { try call("io:\(seconds)") }
    func setPrefersNoInterruptionsFromSystemAlerts(_ value: Bool) throws { try call("noInterruptions:\(value)") }
    func setActive(_ active: Bool) throws {
        try call("active:\(active)")
        s.withLock { $0.active = active }
    }
    var availableInputs: [AudioPortInfo] { s.withLock { $0.inputs } }
    func setPreferredInput(uid: String?) throws {
        try call("preferredInput")
        s.withLock { $0.preferredInput = uid }
    }
    var route: AudioRouteSnapshot { s.withLock { $0.route } }
}

/// The C host replaced by a fake: start results, running flag, callback counter.
final class FakeBackend: VoiceIOBackend {
    struct State {
        var starts: [VoiceIOProfile] = []
        var stops = 0
        var running = false
        var muted = true
        var callbacks: UInt64 = 0
        var nextResults: [Int32] = []
        var format = HardwareFormat(inputRate: 16_000, inputChannels: 1, outputRate: 16_000, outputChannels: 1)
    }

    let s = Mutex(State())

    func start(_ profile: VoiceIOProfile) -> Int32 {
        s.withLock { st in
            st.starts.append(profile)
            let r = st.nextResults.isEmpty ? 0 : st.nextResults.removeFirst()
            st.running = r == 0
            return r
        }
    }
    func stop() { s.withLock { $0.stops += 1; $0.running = false } }
    var isRunning: Bool { s.withLock { $0.running } }
    func setCaptureMuted(_ muted: Bool) { s.withLock { $0.muted = muted } }
    var captureMuted: Bool { s.withLock { $0.muted } }
    var inputCallbacks: UInt64 { s.withLock { $0.callbacks } }
    var hardwareFormat: HardwareFormat? { s.withLock { $0.running ? $0.format : nil } }

    var starts: [VoiceIOProfile] { s.withLock { $0.starts } }
    func failNext(_ statuses: Int32...) { s.withLock { $0.nextResults += statuses } }
    func tick(_ n: UInt64 = 1) { s.withLock { $0.callbacks += n } }
}
