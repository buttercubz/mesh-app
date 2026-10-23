@preconcurrency import MeshCoreBindings
import CMeshAudio
import Foundation
import MeshActivityShared
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// Host plumbing around the engine: event stream, RT fast paths, Live Activity actions,
/// notifications, platform audio events and samples.
@MainActor
final class EngineHostPlatformTests: XCTestCase {
    override func tearDown() async throws {
        LiveActivityActionHandler.register(nil)
        cmesh_set_flags(0)
    }

    func testEventsStreamCarriesTheEnginesUiEvents() async throws {
        let rig = try HostRig(air: FakeAir(), seed: 31)
        let stream = rig.host.events
        rig.host.setUiAttached(true)
        try rig.host.start(.lab(code: 1357))
        _ = rig.host.send(#"{"type":"requestSnapshot"}"#)
        var got: String?
        for await e in stream where e.contains(#""snapshot""#) {
            got = e
            break
        }
        let s = try XCTUnwrap(CommandJSON.object(try XCTUnwrap(got))?["s"] as? [String: Any])
        XCTAssertNotNil(s["engine"])
        rig.host.stop()
    }

    func testPttWritesTheRtFlagAndTheCommand() throws {
        let rig = try HostRig(air: FakeAir(), seed: 32)
        cmesh_set_flags(0)
        rig.host.ptt(true)
        XCTAssertEqual(cmesh_flags() & CMESH_FLAG_PTT_DOWN, CMESH_FLAG_PTT_DOWN)
        XCTAssertEqual(rig.log.count("engine.uiCommand:ptt"), 1)
        rig.host.ptt(false)
        XCTAssertEqual(cmesh_flags() & CMESH_FLAG_PTT_DOWN, 0)
        // The JSON pass-through mirrors the flag too (§3.7).
        _ = rig.host.send(#"{"type":"ptt","down":true}"#)
        XCTAssertEqual(cmesh_flags() & CMESH_FLAG_PTT_DOWN, CMESH_FLAG_PTT_DOWN)
        let before = cmesh_flags() & CMESH_FLAG_PTT_LATCH_TOGGLE
        rig.host.setRtLatchToggle()
        XCTAssertNotEqual(cmesh_flags() & CMESH_FLAG_PTT_LATCH_TOGGLE, before, "latch edge = toggled bit 3")
    }

    /// Live Activity buttons: mute and latch only write RT flags; Resume runs the
    /// gesture path once the app is foreground-active.
    func testLiveActivityActions() throws {
        let rig = try HostRig(air: FakeAir(), seed: 33, foregroundActive: false)
        rig.host.registerLiveActivityActions()
        cmesh_set_flags(0)
        XCTAssertTrue(LiveActivityActionHandler.handle(.toggleMute))
        XCTAssertEqual(cmesh_flags() & CMESH_FLAG_HARD_MUTE, CMESH_FLAG_HARD_MUTE)
        XCTAssertTrue(LiveActivityActionHandler.handle(.latchTalk))
        XCTAssertEqual(cmesh_flags() & CMESH_FLAG_PTT_LATCH_TOGGLE, CMESH_FLAG_PTT_LATCH_TOGGLE)
        XCTAssertEqual(rig.log.count("engine.uiCommand"), 0, "flag writes never go through the engine")
        // Resume while the app is still opening: deferred until it is foreground-active.
        XCTAssertTrue(LiveActivityActionHandler.handle(.resumeMic))
        XCTAssertNil(rig.log.index(of: "engine.uiCommand:resumeMic"))
        rig.foreground.set(true)
        rig.host.mainSide.lifecycle.receive(.visibility(.foreground))
        XCTAssertNotNil(rig.log.index(of: "engine.uiCommand:resumeMic"))
        XCTAssertLessThan(rig.log.index(of: "audio.activate")!, rig.log.index(of: "engine.uiCommand:resumeMic")!)
    }

    func testNotificationsUseTheUiCatalog() throws {
        let rig = try HostRig(air: FakeAir(), seed: 34)
        // Without a catalog nothing is posted (never English fallbacks from native code).
        rig.host.hooks.notify(n: FfiNotification(category: .notice, key: "bluetoothOff", paramsJson: "{}"))
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        XCTAssertTrue(rig.notifications.specs.isEmpty)
        rig.host.setNotificationCatalog(NotificationCatalog(title: "convLink Voice", templates: [
            "memberLost": "Fuera de alcance ({{count}}): {{names}}",
            "captureLost": "Micrófono en pausa. Toque Reanudar para volver a hablar.",
            "engineCrashed": "El motor se detuvo",
        ]))
        rig.host.hooks.notify(n: FfiNotification(category: .notice, key: "memberLost",
                                                 paramsJson: #"{"count":1,"names":"Ana"}"#))
        rig.host.hooks.notify(n: FfiNotification(category: .notice, key: "captureLost", paramsJson: "{}"))
        rig.host.hooks.notify(n: FfiNotification(category: .error, key: "engineCrashed", paramsJson: "{}"))
        waitUntil("three notifications") { rig.notifications.specs.count == 3 }
        let specs = rig.notifications.specs
        XCTAssertTrue(specs.contains { $0.id == "mesh.notice.memberLost" && $0.body == "Fuera de alcance (1): Ana" && !$0.timeSensitive })
        XCTAssertTrue(specs.contains { $0.id == ResumeNotifier.notificationId && $0.timeSensitive })
        XCTAssertTrue(specs.contains { $0.id == "mesh.error.engineCrashed" && $0.timeSensitive })
        rig.host.hooks.notify(n: FfiNotification(category: .notice, key: "captureRestored", paramsJson: "{}"))
        waitUntil("resume notification cleared") { rig.notifications.removedIds.contains(ResumeNotifier.notificationId) }
    }

    /// Platform audio events reach the engine (`on_audio_io`, `on_audio_route`).
    func testPlatformAudioEventsReachTheEngine() throws {
        let rig = try HostRig(air: FakeAir(), seed: 35)
        try rig.host.start(.lab(code: 2244))
        rig.calls.onCall = true
        rig.audio.emit(.io(.interrupted(.otherSession)))
        rig.audio.emit(.io(.captureLost))
        rig.audio.emit(.route(AudioRouteInfo(kind: .hfpWideband, inputRate: 16_000, output: .bluetoothHfp,
                                             headsetPresent: true)))
        XCTAssertTrue(rig.log.all.contains { $0.hasPrefix("engine.onAudioIo:interrupted") && $0.hasSuffix("call)") },
                      "a call interruption is reported as `call`: \(rig.log.all)")
        XCTAssertNotNil(rig.log.index(of: "engine.onAudioIo:captureLost"))
        rig.host.stop()
    }

    /// No retain cycle between the host, its adapters and the main side: releasing the
    /// host releases the engine (its thread joins) and every adapter.
    func testHostIsReleasedWithItsAdapters() throws {
        weak var host: EngineHost?
        weak var radio: RadioAdapter?
        weak var main: HostMainSide?
        do {
            let rig = try HostRig(air: FakeAir(), seed: 37)
            try rig.host.start(.lab(code: 1111))
            rig.host.stop()
            host = rig.host
            radio = rig.host.radio
            main = rig.host.mainSide
        }
        waitUntil("host, adapters and main side released") { host == nil && radio == nil && main == nil }
    }

    /// The Demo ride needs no radio drivers (no Bluetooth prompt); until WP-12's Phase 1
    /// world exists the engine answers DemoUnavailable.
    func testDemoRideNeverCreatesRadioDrivers() throws {
        let rig = try HostRig(air: FakeAir(), seed: 38)
        do {
            try rig.host.start(.demo)
            rig.host.stop()
        } catch {
            XCTAssertEqual(error, .engine("\(FfiError.DemoUnavailable)"))
        }
        XCTAssertFalse(rig.host.radio.hasStack)
    }

    func testPlatformSamplesDescribeTheDevice() throws {
        let rig = try HostRig(air: FakeAir(), seed: 36)
        let s = rig.host.mainSide.sample()
        XCTAssertEqual(try XCTUnwrap(s.battery), 0.8, accuracy: 1e-6)
        XCTAssertEqual(s.charging, false)
        XCTAssertEqual(s.thermal, .nominal)
        XCTAssertFalse(s.background)
        XCTAssertEqual(s.locked, false)
        XCTAssertEqual(s.anchorSession, false)
        XCTAssertEqual(s.anchorLocation, false)
        XCTAssertEqual(s.ble, .on)
        XCTAssertEqual(s.aware, .unsupported)
        XCTAssertNil(s.gps, "no location without the anchor")
        rig.host.mainSide.lifecycle.receive(.visibility(.background))
        XCTAssertTrue(rig.host.mainSide.sample().background)
        XCTAssertFalse(rig.foreground.isForegroundActive)
    }
}
