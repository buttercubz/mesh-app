@preconcurrency import MeshCoreBindings
import AVFAudio
import CMeshAudio
import Foundation
@testable import MeshEngineKit
import MeshPlatform
import Synchronization
import XCTest

/// AVAudioSession replaced by a recorder.
private final class RecordingSessionPort: AudioSessionPort, @unchecked Sendable {
    private let lock = NSLock()
    private(set) var calls: [String] = []
    private(set) var options: AVAudioSession.CategoryOptions = []

    private func call(_ s: String) { lock.withLock { calls.append(s) } }

    func setCategory(_ category: AVAudioSession.Category, mode: AVAudioSession.Mode,
                     options: AVAudioSession.CategoryOptions) throws {
        lock.withLock { self.options = options }
        call("category:\(category.rawValue):\(mode.rawValue)")
    }
    func setPreferredSampleRate(_ rate: Double) throws { call("rate:\(Int(rate))") }
    func setPreferredIOBufferDuration(_ seconds: TimeInterval) throws { call("io") }
    func setPrefersNoInterruptionsFromSystemAlerts(_ value: Bool) throws { call("noAlerts") }
    func setActive(_ active: Bool) throws { call("active:\(active)") }
    var availableInputs: [AudioPortInfo] { [AudioPortInfo(portType: AVAudioSession.Port.bluetoothHFP.rawValue, uid: "hfp")] }
    func setPreferredInput(uid: String?) throws { call("input:\(uid ?? "nil")") }
    var route: AudioRouteSnapshot {
        AudioRouteSnapshot(inputs: [AudioPortInfo(portType: AVAudioSession.Port.bluetoothHFP.rawValue)],
                           outputs: [AudioPortInfo(portType: AVAudioSession.Port.bluetoothHFP.rawValue)],
                           sampleRate: 16_000)
    }
}

/// The C audio host replaced by a recorder.
private final class RecordingBackend: VoiceIOBackend, @unchecked Sendable {
    private let lock = NSLock()
    private(set) var starts: [VoiceIOProfile] = []
    private var running = false
    private var muted = true

    func start(_ profile: VoiceIOProfile) -> Int32 {
        lock.withLock {
            starts.append(profile)
            running = true
        }
        return 0
    }
    func stop() { lock.withLock { running = false } }
    var isRunning: Bool { lock.withLock { running } }
    func setCaptureMuted(_ m: Bool) { lock.withLock { muted = m } }
    var captureMuted: Bool { lock.withLock { muted } }
    var inputCallbacks: UInt64 { 0 }
    var hardwareFormat: HardwareFormat? { nil }
}

/// The adapter over the real `PlatformAudio` orchestration (WP-14's split between the
/// gesture start and `StartCapture`), with the OS session and the C unit faked.
final class PlatformAudioBridgeTests: XCTestCase {
    func testGestureThenStartCaptureThroughPlatformAudio() throws {
        let port = RecordingSessionPort()
        let backend = RecordingBackend()
        let audio = PlatformAudio(session: AudioSessionController(port: port, center: NotificationCenter()),
                                  unit: VoiceIOUnit(backend: backend), pollInterval: 60)
        let events = Mutex<[PlatformAudioEvent]>([])
        let engine = EngineRef()
        let adapter = AudioAdapter(audio: audio, engine: engine, foreground: ForegroundState(true),
                                   calls: FakeCallState(),
                                   callbacks: .init(vehicle: { .motorcycle }, wantsLocationAnchor: {},
                                                    sessionClosed: {},
                                                    event: { e in events.withLock { $0.append(e) } },
                                                    log: { _, _ in }))
        // 1. Gesture: session active, VPIO started muted, capture not connected yet.
        XCTAssertTrue(try adapter.prepareGesture(GesturePlan.make(.goLive, shareDistance: false)))
        XCTAssertTrue(port.calls.contains("active:true"))
        XCTAssertTrue(port.options.contains(.allowBluetoothHFP))
        XCTAssertTrue(port.options.contains(.mixWithOthers), "mixable product session (2026-10-23 deviation)")
        XCTAssertEqual(backend.starts, [.voice])
        XCTAssertTrue(backend.captureMuted, "the unit starts muted")
        XCTAssertTrue(audio.isLive)
        XCTAssertTrue(events.withLock { $0.contains(.io(.running)) })
        // 2. Core: OpenSession connects the RT sink; StartCapture unmutes; nothing activates.
        let activations = port.calls.filter { $0 == "active:true" }.count
        adapter.handle(.openSession(profile: FfiAudioSessionProfile(kind: .ride, capture: true, mixable: false,
                                                                    locationAnchor: false,
                                                                    bypassVoiceProcessing: false)))
        XCTAssertTrue(cmesh_has_external_sink(), "mesh_rt_* registered through cmesh_set_sink")
        adapter.handle(.startCapture)
        XCTAssertFalse(backend.captureMuted)
        adapter.handle(.startCapture)
        XCTAssertEqual(port.calls.filter { $0 == "active:true" }.count, activations, "StartCapture never activates")
        XCTAssertEqual(backend.starts, [.voice])
        // 3. StopCapture mutes; CloseSession deactivates and unregisters the sink.
        adapter.handle(.stopCapture)
        XCTAssertTrue(backend.captureMuted)
        adapter.handle(.closeSession)
        XCTAssertTrue(port.calls.contains("active:false"))
        XCTAssertFalse(cmesh_has_external_sink())
        XCTAssertFalse(audio.isLive)
    }

    func testRenderOnlyLabPathStartsTheRenderOnlyUnit() throws {
        let port = RecordingSessionPort()
        let backend = RecordingBackend()
        let audio = PlatformAudio(session: AudioSessionController(port: port, center: NotificationCenter()),
                                  unit: VoiceIOUnit(backend: backend), pollInterval: 60)
        let adapter = AudioAdapter(audio: audio, engine: EngineRef(), foreground: ForegroundState(true),
                                   calls: FakeCallState(),
                                   callbacks: .init(vehicle: { .car }, wantsLocationAnchor: {}, sessionClosed: {},
                                                    event: { _ in }, log: { _, _ in }))
        let plan = GesturePlan.make(.labStart(.init(bench: "T2", audio: "renderOnly", anchor: "audio", mixable: true)),
                                    shareDistance: false)
        XCTAssertTrue(try adapter.prepareGesture(plan))
        XCTAssertEqual(backend.starts, [.voiceRenderOnly])
        XCTAssertTrue(port.options.contains(.mixWithOthers), "Lab mixable toggle")
        XCTAssertTrue(port.options.contains(.defaultToSpeaker), "car profile")
        adapter.handle(.closeSession)
        XCTAssertFalse(audio.isLive)
    }
}
