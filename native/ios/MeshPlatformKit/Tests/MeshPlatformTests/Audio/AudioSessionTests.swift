import AVFAudio
import Foundation
@testable import MeshPlatform
import XCTest

final class AudioSessionConfigTests: XCTestCase {
    func testMotorcycleConfigMatchesDesign() {
        let c = AudioSessionConfig.make(profile: .motorcycle)
        XCTAssertEqual(c.category, .playAndRecord)
        XCTAssertEqual(c.mode, .voiceChat)
        XCTAssertEqual(c.options, [.allowBluetoothHFP, .mixWithOthers])
        XCTAssertEqual(c.preferredSampleRate, 16_000)
        XCTAssertEqual(c.preferredIOBufferDuration, 0.010)
        XCTAssertTrue(c.prefersNoInterruptionsFromSystemAlerts)
        XCTAssertTrue(c.isMixable, "product session mixes with music (2026-10-23 deviation)")
        XCTAssertFalse(c.options.contains(.duckOthers), "VPIO ducks only while someone talks")
        XCTAssertFalse(c.usesHighQualityBluetoothRecording)
    }

    func testCarAddsDefaultToSpeakerAndLabNonMixableToggle() {
        XCTAssertEqual(AudioSessionConfig.make(profile: .car).options,
                       [.allowBluetoothHFP, .defaultToSpeaker, .mixWithOthers])
        let lab = AudioSessionConfig.make(profile: .motorcycle, mixable: false)
        XCTAssertFalse(lab.isMixable)
        XCTAssertTrue(lab.options.contains(.allowBluetoothHFP))
    }
}

final class AudioSessionControllerTests: XCTestCase {
    func testActivateAppliesEveryStepInOrderAndPrefersHfp() throws {
        let port = FakeSessionPort()
        port.s.withLock {
            $0.inputs = [AudioPortInfo(portType: "MicrophoneBuiltIn", uid: "mic"),
                         AudioPortInfo(portType: AVAudioSession.Port.bluetoothHFP.rawValue, uid: "helmet")]
        }
        let c = AudioSessionController(port: port, center: NotificationCenter())
        try c.activate(.make(profile: .motorcycle))
        XCTAssertEqual(port.calls, ["setCategory", "rate:16000", "io:0.01", "noInterruptions:true",
                                    "active:true", "preferredInput"])
        XCTAssertEqual(port.s.withLock { $0.preferredInput }, "helmet")
        XCTAssertTrue(c.isActive)
        c.deactivate()
        XCTAssertFalse(c.isActive)
        XCTAssertEqual(port.calls.last, "active:false")
    }

    func testRouteChangeReassertsHfpInputOnlyWhenLostAndWanted() throws {
        let port = FakeSessionPort()
        let hfp = AVAudioSession.Port.bluetoothHFP.rawValue
        port.s.withLock {
            $0.inputs = [AudioPortInfo(portType: "MicrophoneBuiltIn", uid: "mic"),
                         AudioPortInfo(portType: hfp, uid: "helmet")]
        }
        let c = AudioSessionController(port: port, center: NotificationCenter())
        XCTAssertFalse(c.reassertPreferredInput(), "inactive session: nothing to do")
        try c.activate(.make(profile: .motorcycle))
        // Route fell back to the built-in mic (e.g. music moved the headset to A2DP).
        port.s.withLock { $0.preferredInput = nil }
        XCTAssertTrue(c.reassertPreferredInput())
        XCTAssertEqual(port.s.withLock { $0.preferredInput }, "helmet")
        // Already on HFP: no new request, so the resulting route change cannot loop.
        port.s.withLock { $0.route.inputs = [AudioPortInfo(portType: hfp, uid: "helmet")] }
        XCTAssertFalse(c.reassertPreferredInput())
        // Rider chose the phone mic: never dragged back to the helmet.
        port.s.withLock { $0.route.inputs = [AudioPortInfo(portType: "MicrophoneBuiltIn", uid: "mic")] }
        c.setHfpInputWanted(false)
        XCTAssertFalse(c.reassertPreferredInput())
    }

    func testActivationFailureNamesTheStep() {
        let port = FakeSessionPort()
        port.s.withLock { $0.failOn = "active:true" }
        let c = AudioSessionController(port: port, center: NotificationCenter())
        XCTAssertThrowsError(try c.activate(.make(profile: .car))) { e in
            XCTAssertEqual(e as? AudioPlatformError, .session("setActive", 561_145_187))
        }
        XCTAssertFalse(c.isActive)
        XCTAssertThrowsError(try c.reactivate(), "nothing to reactivate")
    }

    func testLegacyInterruptionParsing() {
        let began: [AnyHashable: Any] = [AVAudioSessionInterruptionTypeKey: NSNumber(value: 1),
                                         AVAudioSessionInterruptionReasonKey: NSNumber(value: 0)]
        XCTAssertEqual(AudioSessionController.legacyInterruptionEvent(began), .interruptionBegan(.otherSession))
        let muted: [AnyHashable: Any] = [AVAudioSessionInterruptionTypeKey: NSNumber(value: 1),
                                         AVAudioSessionInterruptionReasonKey: NSNumber(value: 2)]
        XCTAssertEqual(AudioSessionController.legacyInterruptionEvent(muted), .interruptionBegan(.builtInMicMuted))
        let endedResume: [AnyHashable: Any] = [AVAudioSessionInterruptionTypeKey: NSNumber(value: 0),
                                               AVAudioSessionInterruptionOptionKey: NSNumber(value: 1)]
        XCTAssertEqual(AudioSessionController.legacyInterruptionEvent(endedResume), .interruptionEnded(shouldResume: true))
        let ended: [AnyHashable: Any] = [AVAudioSessionInterruptionTypeKey: NSNumber(value: 0)]
        XCTAssertEqual(AudioSessionController.legacyInterruptionEvent(ended), .interruptionEnded(shouldResume: false))
        XCTAssertNil(AudioSessionController.legacyInterruptionEvent([:]))
    }

    func testIOS27DeactivationParsing() {
        XCTAssertNil(AudioSessionController.deactivationEvent(source: 1, reason: nil), "our own setActive(false)")
        XCTAssertEqual(AudioSessionController.deactivationEvent(source: 2, reason: 4),
                       .interruptionBegan(.routeDisconnected))
        XCTAssertEqual(AudioSessionController.deactivationEvent(source: 2, reason: nil),
                       .interruptionBegan(.otherSession))
        XCTAssertEqual(AudioSessionController.interruptionReason(9), .unknown)
    }

    func testRouteChangeParsingAndObservation() {
        let center = NotificationCenter()
        let c = AudioSessionController(port: FakeSessionPort(), center: center)
        let seen = Recorder<AudioSessionEvent>()
        c.startObserving { seen.add($0) }
        center.post(name: AVAudioSession.routeChangeNotification, object: nil,
                    userInfo: [AVAudioSessionRouteChangeReasonKey: NSNumber(value: 2)])
        center.post(name: AVAudioSession.mediaServicesWereResetNotification, object: nil)
        center.post(name: AVAudioSession.mediaServicesWereLostNotification, object: nil)
        XCTAssertEqual(seen.values, [.routeChanged(reason: 2), .mediaServicesReset, .mediaServicesLost])
        c.stopObserving()
        center.post(name: AVAudioSession.mediaServicesWereResetNotification, object: nil)
        XCTAssertEqual(seen.values.count, 3, "no events after stopObserving")
    }

    func testInterruptionObservationUsesTheOSSpecificPath() {
        let center = NotificationCenter()
        let c = AudioSessionController(port: FakeSessionPort(), center: center)
        let seen = Recorder<AudioSessionEvent>()
        c.startObserving { seen.add($0) }
        var observesIOS27 = false
        #if compiler(>=6.4)
        if #available(iOS 27.0, *) {
            observesIOS27 = true
            // iOS 27: the deprecated notification is not observed (no double events).
            center.post(name: AVAudioSession.interruptionNotification, object: nil,
                        userInfo: [AVAudioSessionInterruptionTypeKey: NSNumber(value: 1)])
            XCTAssertTrue(seen.values.isEmpty)
            center.post(name: AVAudioSession.resumptionRecommendationNotification, object: nil, userInfo: [:])
            XCTAssertEqual(seen.values, [.interruptionEnded(shouldResume: false)], "no context → do not resume")
        }
        #endif
        if !observesIOS27 {
            center.post(name: AVAudioSession.interruptionNotification, object: nil,
                        userInfo: [AVAudioSessionInterruptionTypeKey: NSNumber(value: 1)])
            XCTAssertEqual(seen.values, [.interruptionBegan(.otherSession)])
        }
    }
}

final class RouteMonitorTests: XCTestCase {
    private func route(_ ins: [String], _ outs: [String], _ rate: Double) -> AudioRouteSnapshot {
        AudioRouteSnapshot(inputs: ins.map { AudioPortInfo(portType: $0) },
                           outputs: outs.map { AudioPortInfo(portType: $0) }, sampleRate: rate)
    }

    func testMapping() {
        typealias P = AVAudioSession.Port
        let hfpWB = RouteMonitor.map(route([P.bluetoothHFP.rawValue], [P.bluetoothHFP.rawValue], 16_000))
        XCTAssertEqual(hfpWB, AudioRouteInfo(kind: .hfpWideband, inputRate: 16_000, output: .bluetoothHfp, headsetPresent: true))
        let hfpNB = RouteMonitor.map(route([P.bluetoothHFP.rawValue], [P.bluetoothHFP.rawValue], 8_000))
        XCTAssertEqual(hfpNB.kind, .hfpNarrowband)
        XCTAssertEqual(hfpNB.inputRate, 8_000, "the core encodes NB on CVSD routes")
        XCTAssertEqual(RouteMonitor.map(route([P.builtInMic.rawValue], [P.builtInSpeaker.rawValue], 48_000)),
                       AudioRouteInfo(kind: .builtinSpeaker, inputRate: 48_000, output: .speaker, headsetPresent: false))
        XCTAssertEqual(RouteMonitor.map(route([P.builtInMic.rawValue], [P.builtInReceiver.rawValue], 16_000)).kind,
                       .builtinReceiver)
        XCTAssertEqual(RouteMonitor.map(route([P.bluetoothHFP.rawValue], [P.carAudio.rawValue], 16_000)).kind, .carBluetooth)
        XCTAssertEqual(RouteMonitor.map(route([P.headsetMic.rawValue], [P.headphones.rawValue], 44_100)).kind, .wired)
        XCTAssertEqual(RouteMonitor.map(route([], [P.bluetoothLE.rawValue], 48_000)).kind, .leAudio)
        let renderOnly = RouteMonitor.map(route([], [P.builtInSpeaker.rawValue], 48_000))
        XCTAssertEqual(renderOnly.inputRate, 0, "no input on a render-only route")
        XCTAssertEqual(RouteMonitor.map(route([], [], 0)), .none)
        XCTAssertEqual(RouteMonitor.normalisedRate(15_999.7), 16_000)
    }

    func testRefreshReportsChangesOnce() {
        let port = FakeSessionPort()
        let m = RouteMonitor(port: port)
        XCTAssertNotNil(m.refresh())
        XCTAssertNil(m.refresh())
        port.s.withLock {
            $0.route = AudioRouteSnapshot(inputs: [AudioPortInfo(portType: "BluetoothHFP")],
                                          outputs: [AudioPortInfo(portType: "BluetoothHFP")], sampleRate: 16_000)
        }
        XCTAssertEqual(m.refresh()?.kind, .hfpWideband)
        XCTAssertNil(m.refresh())
    }
}
