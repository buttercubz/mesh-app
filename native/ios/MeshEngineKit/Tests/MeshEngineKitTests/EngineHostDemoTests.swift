@preconcurrency import MeshCoreBindings
import Foundation
import MeshCoreFFI
@testable import MeshEngineKit
import XCTest

/// The single-device Demo ride (DESIGN §3.4, §1.3): `start(.demo)` runs the runtime's
/// demo world (bots on virtual links, no radio drivers), the bots' `demo.*` lines go
/// through `render_prompts` → renderer → `on_prompt_pcm`, and audio starts with
/// `goLive` exactly as in a group ride.
@MainActor
final class EngineHostDemoTests: XCTestCase {
    private static let setPrompts = #"""
    {"type":"setPrompts","lang":"en","enabled":true,"templates":{"demo.line1":"One","demo.line2":"Two",
    "demo.line3":"Three","demo.line4":"Four","demo.line5":"Five","demo.line6":"Six","demo.line7":"Seven",
    "demo.line8":"Eight"}}
    """#

    /// Demo ride plays audio in the simulator: start(.demo) → goLive →
    /// `audio.connectRtSink` / `unmute`, and the bots' voice reaches the render ring.
    func testDemoRidePlaysAudioInTheSimulator() throws {
        let rig = try HostRig(air: FakeAir(), seed: 61)
        rig.host.setUiAttached(true)
        XCTAssertTrue(CommandJSON.isOk(rig.host.send(Self.setPrompts)))
        try rig.host.start(.demo)
        XCTAssertFalse(rig.host.radio.hasStack, "the Demo never creates radio drivers")
        XCTAssertEqual(rig.host.info().status, "running")

        // The bots' lines are rendered by the platform renderer, back into the engine.
        waitUntil("demo lines requested") {
            rig.prompts.requests.filter { $0.key.hasPrefix("demo.") }.count == 8
        }
        let tone = (0..<16_000).map { Int16(truncatingIfNeeded: Int((sin(Double($0) * 0.17) * 8_000).rounded())) }
        for req in rig.prompts.requests where req.key.hasPrefix("demo.") {
            rig.prompts.deliver(req.key, tone)
        }
        XCTAssertNotNil(rig.log.index(of: "engine.onPromptPcm:demo.line1:16000"), "\(rig.log.all)")

        // The bots form the ride; Go live opens audio like a group ride.
        _ = rig.host.send(#"{"type":"requestSnapshot"}"#)
        waitUntil("demo session snapshot") { rig.host.mainSide.notify.digest.hasSession }
        let r = try rig.host.goLive()
        XCTAssertTrue(CommandJSON.isOk(r), r)
        waitUntil("OpenSession + StartCapture") { rig.log.index(of: "audio.unmute") != nil }
        XCTAssertLessThan(try XCTUnwrap(rig.log.index(of: "audio.connectRtSink")),
                          try XCTUnwrap(rig.log.index(of: "audio.unmute")))

        // The bots talk: their decoded voice reaches the RT render ring.
        var pcm = [Int16](repeating: 0, count: 320)
        waitUntil("bot voice rendered", timeout: 30) {
            XCTAssertEqual(mesh_rt_render(&pcm, 320, 0), 320)
            return pcm.contains { abs(Int32($0)) > 1_000 }
        }

        rig.host.stop()
        waitUntil("session closed") { rig.log.index(of: "audio.close") != nil }
        XCTAssertEqual(rig.host.info().status, "idle")
        XCTAssertFalse(rig.host.radio.hasStack, "still no radio after the Demo")

        // The same host runs a second Demo (one host for the life of the process).
        try rig.host.start(.demo)
        rig.host.stop()
    }
}
