@preconcurrency import MeshCoreBindings
import Foundation
import MeshActivityShared
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// Capture gating through the host (DESIGN §3.2, §11.1 bridge contract).
@MainActor
final class EngineHostGestureTests: XCTestCase {
    /// `send` rejects every ★ command of the shared fixtures with `useGestureCommand`, and
    /// none of them reaches the engine or touches audio.
    func testSendRejectsEveryGestureCommand() throws {
        let rig = try HostRig(air: FakeAir(), seed: 11)
        for f in try RepoFiles.commandFixtures() where f.gesture {
            XCTAssertEqual(rig.host.send(f.json), #"{"ok":false,"code":"useGestureCommand"}"#, f.name)
        }
        XCTAssertEqual(rig.log.count("engine.uiCommand"), 0)
        XCTAssertEqual(rig.log.count("audio."), 0)
        // A plain command passes through to the engine (works in Idle).
        XCTAssertTrue(CommandJSON.isOk(rig.host.send(#"{"type":"listGroups"}"#)))
        XCTAssertEqual(rig.log.count("engine.uiCommand:listGroups"), 1)
    }

    func testGestureCommandRefusesWhenNotForeground() throws {
        let rig = try HostRig(air: FakeAir(), seed: 12, foregroundActive: false)
        XCTAssertThrowsError(try rig.host.gestureCommand(
            #"{"type":"labStart","bench":"T1","params":{"audio":"renderOnly","anchor":"audio","gps":false}}"#)) {
            XCTAssertEqual($0 as? EngineHostError, .notForeground)
        }
        XCTAssertThrowsError(try rig.host.goLive())
        XCTAssertEqual(rig.log.all, [], "nothing ran: no session, no command")
    }

    /// labStart(renderOnly): the session is activated (VPIO muted) and the Live Activity and
    /// location anchor are started BEFORE the command reaches the engine; the engine's
    /// OpenSession then only connects the sink, and never StartCapture.
    func testLabStartActivatesTheSessionBeforeTheEngine() throws {
        let rig = try HostRig(air: FakeAir(), seed: 13)
        rig.host.vehicle = .car
        try rig.host.start(.lab(code: 4321))
        let r = try rig.host.gestureCommand(
            #"{"type":"labStart","bench":"T1","params":{"audio":"renderOnly","anchor":"audioLocation","gps":false,"durationS":30}}"#)
        XCTAssertTrue(CommandJSON.isOk(r), r)
        let log = rig.log
        let activate = try XCTUnwrap(log.index(of: "audio.activate:renderOnly:car"))
        let command = try XCTUnwrap(log.index(of: "engine.uiCommand:labStart"))
        XCTAssertLessThan(activate, command, "\(log.all)")
        waitUntil("OpenSession reached the platform") { log.index(of: "audio.connectRtSink") != nil }
        XCTAssertLessThan(command, log.index(of: "audio.connectRtSink")!)
        XCTAssertNil(log.index(of: "audio.unmute"), "renderOnly never starts capture")
        XCTAssertEqual(rig.anchor.begun, 1, "audioLocation anchor started in the tap")
        waitUntil("Live Activity requested") { !rig.activityPort.requests.isEmpty }
        XCTAssertEqual(rig.activityPort.requests.first?.0, IntercomAttributes(kind: .lab, title: "T1"))
        XCTAssertEqual(rig.activityPort.requests.first?.1.labStatus?.phase, .waiting)
        // Stopping the run closes the session and ends the anchors.
        XCTAssertTrue(CommandJSON.isOk(rig.host.send(#"{"type":"labStop"}"#)))
        waitUntil("CloseSession") { log.index(of: "audio.close") != nil }
        waitUntil("anchors ended") { rig.activityPort.ends == 1 && rig.anchor.tokens.allSatisfy(\.invalidated) }
        rig.host.stop()
    }

    /// labStart(full): StartCapture connects and unmutes, after the gesture's activation.
    func testLabStartFullStartsCaptureOnlyAfterTheGesture() throws {
        let rig = try HostRig(air: FakeAir(), seed: 14)
        try rig.host.start(.lab(code: 4322))
        let r = try rig.host.gestureCommand(
            #"{"type":"labStart","bench":"T0","params":{"audio":"full","anchor":"audio","gps":false,"durationS":30}}"#)
        XCTAssertTrue(CommandJSON.isOk(r), r)
        waitUntil("StartCapture") { rig.log.index(of: "audio.unmute") != nil }
        XCTAssertEqual(rig.log.all.first { $0.hasPrefix("audio.") }, "audio.activate:full:motorcycle:mixable=true")
        XCTAssertLessThan(rig.log.index(of: "audio.activate")!, rig.log.index(of: "engine.uiCommand:labStart")!)
        XCTAssertLessThan(rig.log.index(of: "engine.uiCommand:labStart")!, rig.log.index(of: "audio.unmute")!)
        XCTAssertEqual(rig.anchor.begun, 0, "anchor audio: no location session")
        XCTAssertTrue(CommandJSON.isOk(rig.host.send(#"{"type":"labStop"}"#)))
        waitUntil("capture stopped and session closed") {
            rig.log.index(of: "audio.mute") != nil && rig.log.index(of: "audio.close") != nil
        }
        rig.host.stop()
    }

    /// A refused command rolls the gesture work back (no session left active).
    func testRefusedGestureCommandIsRolledBack() throws {
        let rig = try HostRig(air: FakeAir(), seed: 15)
        // Idle engine: labStart answers engineNotRunning.
        let r = try rig.host.gestureCommand(
            #"{"type":"labStart","bench":"T1","params":{"audio":"renderOnly","anchor":"audioLocation","gps":false}}"#)
        XCTAssertEqual(CommandJSON.object(r)?["code"] as? String, "engineNotRunning")
        XCTAssertEqual(rig.log.all.first, "audio.activate:renderOnly:motorcycle:mixable=true")
        XCTAssertEqual(rig.log.all.last, "audio.close")
        XCTAssertFalse(rig.audio.isLive)
        XCTAssertTrue(rig.anchor.tokens.allSatisfy(\.invalidated))
        waitUntil("activity ended") { rig.activityPort.ends == rig.activityPort.requests.count }
    }

    /// An audio session that cannot be activated never reaches the engine.
    func testActivationFailureAnswersAudioUnavailable() throws {
        let rig = try HostRig(air: FakeAir(), seed: 16)
        rig.audio.failActivation = true
        try rig.host.start(.lab(code: 4323))
        let r = try rig.host.gestureCommand(
            #"{"type":"labStart","bench":"T0","params":{"audio":"full","anchor":"audio","gps":false}}"#)
        XCTAssertEqual(r, #"{"ok":false,"code":"audioUnavailable"}"#)
        XCTAssertNil(rig.log.index(of: "engine.uiCommand:labStart"))
        rig.host.stop()
    }

    /// Non-gated commands sent through gestureCommand are simply forwarded.
    func testGestureCommandForwardsPlainCommands() throws {
        let rig = try HostRig(air: FakeAir(), seed: 17)
        let r = try rig.host.gestureCommand(#"{"type":"createGroup","name":"Sunday ride","vehicle":"car"}"#)
        XCTAssertTrue(CommandJSON.isOk(r), r)
        XCTAssertEqual(rig.host.vehicle, .car, "tracked from the accepted command")
        XCTAssertEqual(rig.log.count("audio."), 0)
        // The new group was persisted to the (fake) keychain through the persist hook.
        waitUntil("group persisted") { rig.keychain.keys.contains { $0.hasPrefix("group/") } }
    }

    func testInfoReportsTheLinkedEngine() throws {
        let rig = try HostRig(air: FakeAir(), seed: 18)
        let i = rig.host.info()
        XCTAssertEqual(i.engine, "full")
        XCTAssertEqual(i.apiVersion, "1.1")
        XCTAssertEqual(i.protocolVersion, 1)
        XCTAssertEqual(i.status, "idle")
        XCTAssertEqual(i.appVersion, "0.1.0 (12)")
        rig.host.setUiAttached(true)
        XCTAssertTrue(rig.host.info().uiAttached)
    }
}
