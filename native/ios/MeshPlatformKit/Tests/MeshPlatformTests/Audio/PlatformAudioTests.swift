import AVFAudio
import CMeshAudio
import Foundation
import MeshActivityShared
@testable import MeshPlatform
import XCTest

/// `PlatformAudio` wiring with a mocked session and C host: the gesture-bound start, the
/// StartCapture split and the mic-loss paths end to end.
final class PlatformAudioTests: XCTestCase {
    private var port: FakeSessionPort!
    private var backend: FakeBackend!
    private var clock: ManualClock!
    private var audio: PlatformAudio!
    private var events: Recorder<PlatformAudioEvent>!

    override func setUp() {
        super.setUp()
        port = FakeSessionPort()
        backend = FakeBackend()
        clock = ManualClock()
        audio = PlatformAudio(session: AudioSessionController(port: port, center: NotificationCenter()),
                              unit: VoiceIOUnit(backend: backend), pollInterval: 3600, clock: clock.fn)
        let ev = Recorder<PlatformAudioEvent>()
        events = ev
        audio.setListener { ev.add($0) }
    }

    override func tearDown() {
        audio.close()
        super.tearDown()
    }

    private func goLive(_ path: AudioPath = .full) throws {
        try audio.activateAndStartMuted(profile: .motorcycle, path: path)
    }

    func testGestureStartActivatesSessionThenStartsVpioMuted() throws {
        try goLive()
        XCTAssertTrue(port.calls.contains("active:true"))
        XCTAssertEqual(backend.starts, [.voice])
        XCTAssertTrue(backend.captureMuted, "VPIO starts with the sink muted (§3.2)")
        XCTAssertEqual(audio.state, .running)
        XCTAssertEqual(audio.unit.state, .running)
        XCTAssertTrue(events.values.contains(.io(.running)))
        XCTAssertTrue(events.values.contains { if case .route = $0 { return true }; return false })
    }

    func testStartCaptureOnlyUnmutesAndNeverActivates() throws {
        audio.unmute() // a late/duplicated StartCapture before Go live is harmless
        XCTAssertTrue(port.calls.isEmpty, "StartCapture never touches the session")
        XCTAssertTrue(backend.starts.isEmpty)
        try goLive()
        XCTAssertTrue(backend.captureMuted, "the wish is recorded but the gesture start is muted")
        audio.unmute()
        XCTAssertFalse(backend.captureMuted)
        audio.mute()
        XCTAssertTrue(backend.captureMuted)
    }

    func testRenderOnlyNeverUnmutes() throws {
        try goLive(.renderOnly)
        XCTAssertEqual(backend.starts, [.voiceRenderOnly])
        audio.unmute()
        XCTAssertTrue(backend.captureMuted)
    }

    func testFailedGestureStartDeactivatesAndThrows() {
        backend.failNext(-66637)
        XCTAssertThrowsError(try goLive()) { XCTAssertEqual($0 as? AudioPlatformError, .unit(-66637)) }
        XCTAssertEqual(port.calls.last, "active:false")
        XCTAssertEqual(audio.state, .idle)
    }

    func testInterruptionThenResumeRestoresCaptureAndMuteWish() throws {
        try goLive()
        audio.unmute()
        audio.handle(.interruptionBegan(.otherSession))
        XCTAssertEqual(audio.ladder, .l1)
        XCTAssertEqual(audio.unit.state, .interrupted)
        audio.handle(.interruptionEnded(shouldResume: true))
        XCTAssertEqual(audio.state, .running)
        XCTAssertEqual(backend.starts, [.voice, .voice])
        XCTAssertFalse(backend.captureMuted, "capture was live before the call, so it is live again")
        let v = events.values
        XCTAssertTrue(v.contains(.io(.interrupted(.otherSession))))
        XCTAssertTrue(v.contains(.io(.restored)))
        XCTAssertTrue(v.contains(.cue(.micBack)))
        XCTAssertEqual(v.filter { if case .ladder = $0 { return true }; return false }, [.ladder(.l1), .ladder(.l0)])
    }

    func testRestartFailureFallsBackToOutputOnlyAndNotifies() throws {
        try goLive()
        audio.handle(.interruptionBegan(.otherSession))
        port.s.withLock { $0.failOn = "active:true" } // '!rec': the session will not reactivate
        audio.handle(.interruptionEnded(shouldResume: true))
        XCTAssertEqual(audio.state, .captureLost)
        XCTAssertEqual(backend.starts.last, .outputOnly, "RemoteIO output-only keeps the group audible")
        let v = events.values
        XCTAssertTrue(v.contains(.io(.captureLost)))
        XCTAssertTrue(v.contains(.captureLostNotice(true)))
        XCTAssertTrue(v.contains(.cue(.micPaused)))
        XCTAssertEqual(audio.ladder, .l2)
        // Foreground tap: the mic comes back and the notice is cleared.
        port.s.withLock { $0.failOn = nil }
        audio.appDidBecomeActive()
        XCTAssertEqual(audio.state, .running)
        XCTAssertEqual(backend.starts.last, .voice)
        XCTAssertTrue(events.values.contains(.captureLostNotice(false)))
    }

    func testMusicAppRouteChangeKeepsCaptureAndReclaimsHelmetMic() throws {
        let hfp = AVAudioSession.Port.bluetoothHFP.rawValue
        port.s.withLock {
            $0.inputs = [AudioPortInfo(portType: hfp, uid: "helmet")]
            $0.route = AudioRouteSnapshot(inputs: [AudioPortInfo(portType: hfp, uid: "helmet")],
                                          outputs: [AudioPortInfo(portType: hfp, uid: "helmet")],
                                          sampleRate: 16_000)
        }
        try goLive()
        XCTAssertTrue(port.s.withLock { $0.options.contains(.mixWithOthers) }, "mixable product session")
        audio.unmute()
        // Spotify starts: the headset falls to A2DP output and the built-in mic.
        port.s.withLock {
            $0.preferredInput = nil
            $0.route = AudioRouteSnapshot(inputs: [AudioPortInfo(portType: "MicrophoneBuiltIn")],
                                          outputs: [AudioPortInfo(portType: "BluetoothA2DPOutput")],
                                          sampleRate: 16_000)
        }
        audio.handle(.routeChanged(reason: 3)) // categoryChange
        XCTAssertEqual(port.s.withLock { $0.preferredInput }, "helmet", "HFP input asked for again")
        XCTAssertEqual(audio.state, .running, "no interruption, capture keeps running")
        XCTAssertFalse(backend.captureMuted)
    }

    func testOutputOnlyFailureIsL3() throws {
        try goLive()
        backend.failNext(1, 2) // restart fails, then output-only fails
        audio.handle(.routeChanged(reason: 2))
        // Unchanged format and running unit: nothing happens yet.
        XCTAssertEqual(audio.state, .running)
        backend.s.withLock { $0.format.inputRate = 8_000 } // HFP CVSD: format changed
        audio.handle(.routeChanged(reason: 1))
        XCTAssertEqual(audio.state, .renderLost)
        XCTAssertEqual(audio.ladder, .l3)
        XCTAssertTrue(events.values.contains(.io(.renderLost)))
    }

    func testWatchdogStallRestartsTheUnit() throws {
        try goLive()
        backend.tick(100)
        audio.pollNow()
        clock.advance(1.0)
        backend.tick(100)
        audio.pollNow()
        clock.advance(1.6) // no callbacks for 1.6 s
        audio.pollNow()
        XCTAssertEqual(backend.starts, [.voice, .voice], "silent capture stop → restart")
        XCTAssertEqual(audio.state, .running)
    }

    func testMediaServicesResetRebuildsSessionAndUnit() throws {
        try goLive()
        let activations = port.calls.filter { $0 == "active:true" }.count
        audio.handle(.mediaServicesReset)
        XCTAssertEqual(port.calls.filter { $0 == "active:true" }.count, activations + 1)
        XCTAssertEqual(backend.starts.count, 2)
        XCTAssertEqual(audio.state, .running)
    }

    func testCloseStopsEverythingAndClearsOnlyOurSink() throws {
        try goLive()
        audio.connectSink(push: nil, pull: nil, flags: nil, stats: nil)
        XCTAssertTrue(cmesh_has_external_sink())
        audio.close()
        XCTAssertFalse(cmesh_has_external_sink())
        XCTAssertEqual(audio.state, .idle)
        XCTAssertEqual(audio.unit.state, .stopped)
        XCTAssertEqual(port.calls.last, "active:false")
    }

    func testSecondGestureWhileLiveIsAResume() throws {
        try goLive()
        backend.failNext(1) // the media-reset rebuild fails → CAPTURE_LOST
        audio.handle(.mediaServicesReset)
        XCTAssertEqual(audio.state, .captureLost)
        try goLive() // resumeMic arrives through gestureCommand: same entry point
        XCTAssertEqual(audio.state, .running)
        XCTAssertEqual(backend.starts.last, .voice)
    }
}

final class VoiceIOUnitTests: XCTestCase {
    func testStatePublisherAndRestartKeepsMute() throws {
        let b = FakeBackend()
        let u = VoiceIOUnit(backend: b)
        let seen = Recorder<VoiceIOState>()
        let token = u.subscribe { seen.add($0) }
        try u.start(.voice)
        u.unmute()
        try u.restart()
        XCTAssertFalse(b.captureMuted)
        u.publish(.restarting)
        u.publish(.restarting)
        u.unsubscribe(token)
        u.publish(.captureLost)
        XCTAssertEqual(seen.values, [.running, .restarting])
        u.stop()
        XCTAssertTrue(b.captureMuted)
        XCTAssertNil(u.profile)
        XCTAssertThrowsError(try u.restart())
    }

    func testStatesAsyncStream() async throws {
        let u = VoiceIOUnit(backend: FakeBackend())
        let stream = u.states
        try u.start(.voiceBypass)
        u.publish(.interrupted)
        var it = stream.makeAsyncIterator()
        let first = await it.next()
        let second = await it.next()
        XCTAssertEqual([first, second], [.running, .interrupted])
        XCTAssertEqual(VoiceIOProfile(path: .t5aBypass), .voiceBypass)
        XCTAssertTrue(VoiceIOProfile.voiceBypass.hasInput)
        XCTAssertFalse(VoiceIOProfile.outputOnly.hasInput)
    }
}
