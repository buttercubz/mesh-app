import AVFAudio
import CMeshAudio
import Foundation
import XCTest

/// AudioUnit lifecycle. The loopback smoke opens the simulator's (Mac's) microphone, so
/// it runs only when asked for (manual smoke, see docs in the WP-14 BITACORA entry):
///
///     TEST_RUNNER_MESH_AUDIO_SMOKE=1 xcodebuild -scheme MeshPlatformKit-Package \
///       -destination "id=$(../../../scripts/pick-sim.sh)" test \
///       -only-testing:CMeshAudioTests/CMeshUnitSmokeTests
final class CMeshUnitSmokeTests: XCTestCase {
    override func setUp() {
        super.setUp()
        resetCMesh()
    }

    override func tearDown() {
        _ = cmesh_stop()
        super.tearDown()
    }

    func testNoUnitReportsNotRunning() {
        _ = cmesh_stop()
        XCTAssertFalse(cmesh_has_unit())
        XCTAssertFalse(cmesh_is_running())
        var r1 = 0.0, r2 = 0.0, c1: UInt32 = 0, c2: UInt32 = 0
        XCTAssertFalse(cmesh_hw_format(&r1, &c1, &r2, &c2))
        XCTAssertFalse(cmesh_client_format_ok())
    }

    private var smokeEnabled: Bool {
        ProcessInfo.processInfo.environment["MESH_AUDIO_SMOKE"] == "1"
    }

    /// VPIO + loopback sink with the Mac microphone: callbacks run, the client format is
    /// 16 kHz mono Int16, the unit reports running, and stop disposes it.
    func testLoopbackSmokeWithMacMicrophone() throws {
        try XCTSkipUnless(smokeEnabled, "manual smoke: set TEST_RUNNER_MESH_AUDIO_SMOKE=1")
        // VPIO needs a recording session (DESIGN §10.1.2) and microphone permission.
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP])
        try session.setPreferredSampleRate(16_000)
        try session.setPreferredIOBufferDuration(0.010)
        try session.setActive(true)
        defer { try? session.setActive(false) }
        let granted = expectation(description: "record permission")
        let ok = LockedFlag()
        AVAudioApplication.requestRecordPermission { g in
            ok.set(g)
            granted.fulfill()
        }
        wait(for: [granted], timeout: 30)
        try XCTSkipUnless(ok.value, "microphone permission not granted to the test runner")
        cmesh_use_builtin(CMESH_BUILTIN_LOOPBACK)
        let st = cmesh_start(CMESH_PROFILE_VOICE)
        XCTAssertEqual(st, 0, "cmesh_start OSStatus \(st)")
        XCTAssertTrue(cmesh_client_format_ok())
        cmesh_set_capture_muted(false)
        Thread.sleep(forTimeInterval: 2)
        XCTAssertTrue(cmesh_is_running())
        var c = CMeshIoCounters()
        cmesh_io_counters(&c)
        XCTAssertGreaterThan(c.input_callbacks, 50, "≈ 100 callbacks/s at 10 ms")
        XCTAssertGreaterThan(c.render_callbacks, 50)
        XCTAssertGreaterThan(c.captured_frames, 16_000)
        XCTAssertEqual(c.input_render_errors, 0, "last status \(c.last_input_status)")
        var inRate = 0.0, outRate = 0.0, inCh: UInt32 = 0, outCh: UInt32 = 0
        XCTAssertTrue(cmesh_hw_format(&inRate, &inCh, &outRate, &outCh))
        XCTAssertGreaterThan(outRate, 0)
        XCTAssertEqual(cmesh_stop(), 0)
        XCTAssertFalse(cmesh_has_unit())
    }

    /// RemoteIO output-only (OutputOnlyFallback) needs no microphone; runs in every test
    /// pass with the silence sink (no audible output).
    func testOutputOnlyUnitRunsInTheSimulator() throws {
        cmesh_use_builtin(CMESH_BUILTIN_SILENCE)
        XCTAssertEqual(cmesh_start(CMESH_PROFILE_OUTPUT_ONLY), 0)
        XCTAssertTrue(cmesh_client_format_ok(), "client format read back as 16 kHz mono Int16")
        XCTAssertEqual(cmesh_profile(), CMESH_PROFILE_OUTPUT_ONLY)
        Thread.sleep(forTimeInterval: 0.5)
        var c = CMeshIoCounters()
        cmesh_io_counters(&c)
        XCTAssertGreaterThan(c.render_callbacks, 10)
        XCTAssertEqual(c.input_callbacks, 0)
        XCTAssertTrue(cmesh_is_running())
        XCTAssertEqual(cmesh_stop(), 0)
        XCTAssertFalse(cmesh_is_running())
    }

    /// VPIO render-only (Lab `renderOnly`): input bus disabled.
    func testVoiceRenderOnlyUnit() throws {
        cmesh_use_builtin(CMESH_BUILTIN_SILENCE)
        let st = cmesh_start(CMESH_PROFILE_VOICE_RENDER_ONLY)
        XCTAssertEqual(st, 0, "OSStatus \(st)")
        Thread.sleep(forTimeInterval: 0.5)
        var c = CMeshIoCounters()
        cmesh_io_counters(&c)
        XCTAssertGreaterThan(c.render_callbacks, 10)
        XCTAssertEqual(c.input_callbacks, 0)
    }
}

/// Thread-safe Bool for permission callbacks.
final class LockedFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var v = false
    func set(_ x: Bool) { lock.lock(); v = x; lock.unlock() }
    var value: Bool { lock.lock(); defer { lock.unlock() }; return v }
}
