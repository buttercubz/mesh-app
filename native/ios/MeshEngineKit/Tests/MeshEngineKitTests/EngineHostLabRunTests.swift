@preconcurrency import MeshCoreBindings
import Foundation
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// Two in-process hosts with fake drivers run Field-test benches end to end over the real
/// Rust engine, link framing and Lab runners (WP-15 acceptance 2).
@MainActor
final class EngineHostLabRunTests: XCTestCase {
    private var air: FakeAir!
    private var a: HostRig!
    private var b: HostRig!

    override func setUp() async throws {
        air = FakeAir()
        a = try HostRig(air: air, seed: 21)
        b = try HostRig(air: air, seed: 22)
        a.host.setUiAttached(true)
        b.host.setUiAttached(true)
        try a.host.start(.lab(code: 2468))
        try b.host.start(.lab(code: 2468))
    }

    override func tearDown() async throws {
        a.host.stop()
        b.host.stop()
        a = nil
        b = nil
        air = nil
    }

    private func linkUp() {
        waitUntil("Lab link up on both phones", timeout: 15) {
            a.snapshotLinks() == 1 && b.snapshotLinks() == 1
        }
    }

    private func finished(_ rig: HostRig, _ runId: String) -> [String: Any]? {
        rig.events(ofType: "labFinished").first { $0["runId"] as? String == runId }
    }

    /// Checks one device file: `run` first with the run's bench/audio/anchor, `summary` last.
    private func checkFile(_ rig: HostRig, _ runId: String, bench: String, audio: String, anchor: String,
                           role: String, file: StaticString = #filePath, line: UInt = #line) {
        let lines = rig.labLines(runId)
        XCTAssertGreaterThanOrEqual(lines.count, 2, "labrun/1 lines", file: file, line: line)
        let run = lines.first ?? [:]
        XCTAssertEqual(run["t"] as? String, "run", file: file, line: line)
        XCTAssertEqual(run["runId"] as? String, runId, file: file, line: line)
        XCTAssertEqual(run["bench"] as? String, bench, file: file, line: line)
        XCTAssertEqual(run["audio"] as? String, audio, file: file, line: line)
        XCTAssertEqual(run["anchor"] as? String, anchor, file: file, line: line)
        XCTAssertEqual(run["role"] as? String, role, file: file, line: line)
        XCTAssertEqual((run["device"] as? [String: Any])?["platform"] as? String, "ios", file: file, line: line)
        XCTAssertEqual(lines.last?["t"] as? String, "summary", file: file, line: line)
        XCTAssertTrue(lines.contains { $0["t"] as? String == "s" }, "per-second samples", file: file, line: line)
    }

    func testLinksFormThroughTheFakeDrivers() throws {
        linkUp()
        // The core drove the drivers: advertising, HELLO, a connect by exactly one side,
        // and an L2CAP open with the PSM published by the peer.
        let all = air.executed(0) + air.executed(1)
        XCTAssertTrue(all.contains { if case .setHello = $0 { return true } else { return false } })
        XCTAssertTrue(all.contains { if case .setDiscovery(.forming, .some) = $0 { return true } else { return false } })
        XCTAssertTrue(all.contains { if case .openLink(_, .l2cap(FakeAir.psm)) = $0 { return true } else { return false } })
        XCTAssertGreaterThan(a.log.count("engine.onLinkUp") + b.log.count("engine.onLinkUp"), 1)
        // Platform samples reach the engine at the configured rate.
        waitUntil("platform samples") { a.host.mainSide.samplesSent >= 2 }
    }

    /// Audio `off`: a plain `send`, no audio command on either phone, one file per phone.
    func testAudioOffRunBetweenTwoHosts() throws {
        linkUp()
        let r = a.host.send(#"{"type":"labStart","bench":"T0","params":{"audio":"off","anchor":"audio","gps":false,"durationS":2,"pattern":"ping"}}"#)
        let runId = try XCTUnwrap(CommandJSON.object(r)?["runId"] as? String, r)
        waitUntil("labFinished on both phones", timeout: 20) { finished(a, runId) != nil && finished(b, runId) != nil }
        checkFile(a, runId, bench: "T0", audio: "off", anchor: "audio", role: "controller")
        checkFile(b, runId, bench: "T0", audio: "off", anchor: "audio", role: "follower")
        XCTAssertEqual(a.log.count("audio."), 0)
        XCTAssertEqual(b.log.count("audio."), 0)
        // The summary CSV was written next to the controller's file, and both files are
        // listed by the engine's run index.
        XCTAssertEqual(try a.labStore.exportURLs(runId: runId).count, 2)
        let runs = CommandJSON.object(a.host.send(#"{"type":"labListRuns"}"#))?["runs"] as? [[String: Any]]
        XCTAssertTrue(runs?.contains { $0["runId"] as? String == runId } ?? false)
        // labDeleteRun removes the files through the lab_delete hook.
        XCTAssertTrue(CommandJSON.isOk(a.host.send(#"{"type":"labDeleteRun","runId":"\#(runId)"}"#)))
        waitUntil("files deleted") { a.labStore.list().allSatisfy { $0.runId != runId } }
    }

    /// Audio `renderOnly`: through `gestureCommand` on the controller; the session is
    /// opened and closed only there, never capture; the peer records without audio.
    func testRenderOnlyRunBetweenTwoHosts() throws {
        linkUp()
        let r = try a.host.gestureCommand(#"{"type":"labStart","bench":"T1","params":{"audio":"renderOnly","anchor":"audio","gps":false,"durationS":2,"streams":1}}"#)
        let runId = try XCTUnwrap(CommandJSON.object(r)?["runId"] as? String, r)
        waitUntil("labFinished on both phones", timeout: 20) { finished(a, runId) != nil && finished(b, runId) != nil }
        checkFile(a, runId, bench: "T1", audio: "renderOnly", anchor: "audio", role: "controller")
        checkFile(b, runId, bench: "T1", audio: "renderOnly", anchor: "audio", role: "follower")
        waitUntil("session closed at the end of the run") { a.log.index(of: "audio.close") != nil }
        let audio = a.log.all.filter { $0.hasPrefix("audio.") }
        XCTAssertEqual(audio, ["audio.activate:renderOnly:motorcycle:mixable=true", "audio.connectRtSink", "audio.close"])
        XCTAssertEqual(b.log.count("audio."), 0)
        // The controller's Live Activity followed the run.
        waitUntil("Live Activity ended") { a.activityPort.ends == 1 }
    }
}
