@preconcurrency import MeshCoreBindings
import Foundation
import MeshActivityShared
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// Phase 1 gesture flows over the real engine (DESIGN §1.3 F3/F5, §3.2, §7.7): Go live in
/// a group lobby, Resume mic after a capture loss, spoken prompts, and the temporary
/// audio-check session of onboarding.
@MainActor
final class EngineHostRideTests: XCTestCase {
    /// A group lobby on one host: `createGroup` (car) + `start(.group)`, UI attached.
    private func lobby(seed: UInt8, shareDistance: Bool) throws -> HostRig {
        let rig = try HostRig(air: FakeAir(), seed: seed)
        rig.host.setUiAttached(true)
        let created = rig.host.send(#"{"type":"createGroup","name":"Sunday ride","vehicle":"car"}"#)
        let hex = try XCTUnwrap(CommandJSON.object(created)?["groupId"] as? String, created)
        try rig.host.start(XCTUnwrap(SessionStart.group(hex: hex)))
        if shareDistance {
            XCTAssertTrue(CommandJSON.isOk(rig.host.send(#"{"type":"setShareDistance","enabled":true}"#)))
        }
        _ = rig.host.send(#"{"type":"requestSnapshot"}"#)
        waitUntil("lobby snapshot reached the host") {
            let d = rig.host.mainSide.notify.digest
            return d.hasSession && d.groupName == "Sunday ride" && d.shareDistance == shareDistance
        }
        return rig
    }

    /// Go live: session + VPIO (muted) + location anchor (share distance) + Live Activity
    /// inside the tap, BEFORE the engine sees the command; the engine's OpenSession then
    /// connects the sink and StartCapture only unmutes.
    func testGoLiveActivatesEverythingBeforeTheEngine() throws {
        let rig = try lobby(seed: 41, shareDistance: true)
        XCTAssertEqual(rig.host.send(#"{"type":"goLive"}"#), #"{"ok":false,"code":"useGestureCommand"}"#)
        let r = try rig.host.goLive()
        XCTAssertTrue(CommandJSON.isOk(r), r)
        let log = rig.log
        let activate = try XCTUnwrap(log.index(of: "audio.activate:full:car:mixable=true"), "\(log.all)")
        let command = try XCTUnwrap(log.index(of: "engine.uiCommand:goLive"))
        XCTAssertLessThan(activate, command)
        XCTAssertEqual(rig.anchor.begun, 1, "share distance: location anchor started in the tap")
        waitUntil("Live Activity requested") { !rig.activityPort.requests.isEmpty }
        XCTAssertEqual(rig.activityPort.requests.first?.0, IntercomAttributes(kind: .ride, title: "Sunday ride"))
        waitUntil("OpenSession + StartCapture") { log.index(of: "audio.unmute") != nil }
        XCTAssertLessThan(command, try XCTUnwrap(log.index(of: "audio.connectRtSink")))
        XCTAssertLessThan(log.index(of: "audio.connectRtSink")!, log.index(of: "audio.unmute")!)
        XCTAssertEqual(log.count("audio.activate"), 1, "StartCapture never activates")

        // While live, a second Go live and an audio check are refused by the engine and
        // never touch the session (no restart of the capture as a side effect).
        let audioCalls = log.count("audio.")
        XCTAssertEqual(CommandJSON.object(try rig.host.goLive())?["code"] as? String, "alreadyLive")
        let check = try rig.host.gestureCommand(#"{"type":"audioCheck","op":"start","mode":"tone"}"#)
        XCTAssertEqual(CommandJSON.object(check)?["code"] as? String, "wrongMode")
        XCTAssertEqual(log.count("audio."), audioCalls, "\(log.all)")
        XCTAssertTrue(rig.audio.isLive, "the ride keeps its session")

        // Engine stop: the session, the anchor and the Live Activity end.
        rig.host.stop()
        waitUntil("session closed") { log.index(of: "audio.close") != nil }
        waitUntil("anchors ended") { rig.activityPort.ends == 1 && rig.anchor.tokens.allSatisfy(\.invalidated) }
        XCTAssertFalse(rig.host.state.withLock { $0.rideLive })
    }

    /// Without share distance, Go live starts no location anchor (the `audio` anchor only).
    func testGoLiveWithoutShareDistanceStartsNoLocation() throws {
        let rig = try lobby(seed: 42, shareDistance: false)
        XCTAssertTrue(CommandJSON.isOk(try rig.host.goLive()))
        XCTAssertEqual(rig.anchor.begun, 0)
        waitUntil("Live Activity requested") { !rig.activityPort.requests.isEmpty }
        rig.host.stop()
    }

    /// Resume mic (F5): after a capture loss the tap restarts capture on the live session
    /// (the platform's resume) before the engine sees the command, and StartCapture
    /// unmutes again. Outside a live ride the engine refuses and the gesture is undone.
    func testResumeMicAfterCaptureLoss() throws {
        let rig = try lobby(seed: 43, shareDistance: false)
        XCTAssertEqual(CommandJSON.object(try rig.host.resumeMic())?["code"] as? String, "notLive")
        XCTAssertFalse(rig.audio.isLive, "the refused resume rolled its fresh session back")
        XCTAssertTrue(CommandJSON.isOk(try rig.host.goLive()))
        waitUntil("capture started") { rig.log.index(of: "audio.unmute") != nil }
        rig.audio.emit(.io(.captureLost))
        XCTAssertNotNil(rig.log.index(of: "engine.onAudioIo:captureLost"))
        XCTAssertEqual(rig.host.send(#"{"type":"resumeMic"}"#), #"{"ok":false,"code":"useGestureCommand"}"#)
        let unmutes = rig.log.count("audio.unmute")
        let r = try rig.host.resumeMic()
        XCTAssertTrue(CommandJSON.isOk(r), r)
        let resume = try XCTUnwrap(rig.log.index(of: "audio.resume"), "\(rig.log.all)")
        // The second resumeMic command (the first was the refused one in the lobby).
        let command = try XCTUnwrap(rig.log.all.lastIndex { $0.hasPrefix("engine.uiCommand:resumeMic") })
        XCTAssertLessThan(resume, command)
        XCTAssertEqual(rig.log.count("engine.uiCommand:resumeMic"), 2)
        waitUntil("StartCapture after resume") { rig.log.count("audio.unmute") == unmutes + 1 }
        XCTAssertTrue(rig.audio.isLive, "a resume never closes the live session")
        rig.host.stop()
    }

    /// Spoken prompts (§7.7): the engine asks for its templates at Go live through
    /// `render_prompts`; the renderer's PCM reaches `on_prompt_pcm`, a failure keeps the
    /// earcon (nothing reaches the engine).
    func testPromptsRenderThroughThePlatformRenderer() throws {
        let rig = try lobby(seed: 44, shareDistance: false)
        let setPrompts = #"{"type":"setPrompts","lang":"es","enabled":true,"templates":{"intercomLive":"Intercom activo, {{n}} motoristas","micBack":"Micrófono de vuelta"}}"#
        XCTAssertTrue(CommandJSON.isOk(rig.host.send(setPrompts)))
        XCTAssertTrue(CommandJSON.isOk(try rig.host.goLive()))
        waitUntil("prompts requested") { rig.prompts.requests.count >= 2 }
        let reqs = rig.prompts.requests
        let live = try XCTUnwrap(reqs.first { $0.key == "intercomLive" }, "\(reqs)")
        XCTAssertTrue(live.text.hasPrefix("Intercom activo, "), live.text)
        XCTAssertFalse(live.text.contains("{{"), "the core fills the template")
        XCTAssertEqual(live.lang, "es")
        XCTAssertTrue(reqs.contains { $0.key == "micBack" && $0.text == "Micrófono de vuelta" })
        // 1 s of 16 kHz PCM from the renderer.
        let pcm = (0..<16_000).map { Int16(truncatingIfNeeded: ($0 % 64) * 256 - 8192) }
        rig.prompts.deliver("intercomLive", pcm)
        XCTAssertNotNil(rig.log.index(of: "engine.onPromptPcm:intercomLive:16000"))
        rig.prompts.fail("micBack")
        XCTAssertNil(rig.log.index(of: "engine.onPromptPcm:micBack"))
        rig.host.stop()
    }

    /// Onboarding headset check (F0.3): a temporary session started inside the tap, with no
    /// Live Activity and no anchor; `audioCheck{stop}` (plain send) closes it.
    func testAudioCheckIsATemporarySession() throws {
        let rig = try HostRig(air: FakeAir(), seed: 45)
        rig.host.setUiAttached(true)
        let r = try rig.host.gestureCommand(#"{"type":"audioCheck","op":"start","mode":"tone"}"#)
        XCTAssertTrue(CommandJSON.isOk(r), r)
        let activate = try XCTUnwrap(rig.log.index(of: "audio.activate:full:motorcycle:mixable=true"))
        XCTAssertLessThan(activate, try XCTUnwrap(rig.log.index(of: "engine.uiCommand:audioCheck")))
        waitUntil("capture connected") { rig.log.index(of: "audio.unmute") != nil }
        XCTAssertEqual(rig.host.info().status, "audioCheck")
        XCTAssertEqual(rig.anchor.begun, 0)
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        XCTAssertTrue(rig.activityPort.requests.isEmpty, "no Live Activity for a check")
        XCTAssertFalse(rig.host.radio.hasStack, "a check never creates radio drivers")
        XCTAssertTrue(CommandJSON.isOk(rig.host.send(#"{"type":"audioCheck","op":"stop"}"#)))
        waitUntil("check session closed") { rig.log.index(of: "audio.close") != nil && !rig.audio.isLive }
        waitUntil("engine back to idle") { rig.host.info().status == "idle" }
    }
}
