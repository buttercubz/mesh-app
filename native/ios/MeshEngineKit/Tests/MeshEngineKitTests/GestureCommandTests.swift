import Foundation
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// Repository files read by the tests (the shared mesh-api fixtures, contract C1).
enum RepoFiles {
    static let root = URL(filePath: #filePath)
        .deletingLastPathComponent() // MeshEngineKitTests
        .deletingLastPathComponent() // Tests
        .deletingLastPathComponent() // MeshEngineKit
        .deletingLastPathComponent() // ios
        .deletingLastPathComponent() // native
        .deletingLastPathComponent() // repo

    static func json(_ path: String) throws -> Any {
        try JSONSerialization.jsonObject(with: Data(contentsOf: root.appending(path: path)))
    }

    /// `{name, requiresGesture, command}` entries of `crates/mesh-api/fixtures/commands.json`.
    static func commandFixtures() throws -> [(name: String, gesture: Bool, json: String)] {
        let list = try XCTUnwrap(json("crates/mesh-api/fixtures/commands.json") as? [[String: Any]])
        return try list.map { e in
            let cmd = try XCTUnwrap(e["command"])
            let data = try JSONSerialization.data(withJSONObject: cmd, options: [.sortedKeys])
            return (e["name"] as? String ?? "?", e["requiresGesture"] as? Bool ?? false,
                    String(decoding: data, as: UTF8.self))
        }
    }
}

final class GestureCommandTests: XCTestCase {
    /// The Swift gate equals mesh-api `GESTURE_COMMANDS` on every shared fixture.
    func testClassificationMatchesTheSharedFixtures() throws {
        let fixtures = try RepoFiles.commandFixtures()
        XCTAssertGreaterThan(fixtures.count, 20)
        XCTAssertTrue(fixtures.contains { $0.gesture }, "fixtures must cover ★ commands")
        for f in fixtures {
            XCTAssertEqual(GestureCommand.requiresGesture(f.json), f.gesture, f.name)
        }
    }

    func testLabStartGatingFollowsTheAudioParam() {
        XCTAssertNil(GestureCommand.classify(#"{"type":"labStart","bench":"T0","params":{"audio":"off","anchor":"audio","gps":false}}"#))
        XCTAssertNil(GestureCommand.classify(#"{"type":"labStart","bench":"T0","params":{"anchor":"audio"}}"#),
                     "a missing audio field is not gated (NotEquals needs the field); the engine refuses it")
        XCTAssertEqual(GestureCommand.classify(#"{"type":"labStart","bench":"T2","params":{"audio":"full","anchor":"none","gps":false,"mixable":true}}"#),
                       .labStart(.init(bench: "T2", audio: "full", anchor: "none", mixable: true)))
        XCTAssertNil(GestureCommand.classify(#"{"type":"audioCheck","op":"stop"}"#))
        XCTAssertEqual(GestureCommand.classify(#"{"type":"audioCheck","op":"start","mode":"echo"}"#),
                       .audioCheckStart(mode: "echo"))
        XCTAssertNil(GestureCommand.classify("not json"))
    }

    func testPlansCoverTheGestureBoundWork() {
        XCTAssertEqual(GesturePlan.make(.goLive, shareDistance: true),
                       GesturePlan(path: .full, mixable: true, locationAnchor: true, activity: .ride, resume: false))
        XCTAssertTrue(GesturePlan.make(.resumeMic, shareDistance: false).mixable, "product session mixes with music")
        XCTAssertEqual(GesturePlan.make(.goLive, shareDistance: false).locationAnchor, false)
        XCTAssertTrue(GesturePlan.make(.resumeMic, shareDistance: false).resume)
        let render = GesturePlan.make(.labStart(.init(bench: "T1", audio: "renderOnly", anchor: "audioLocation",
                                                      mixable: false)), shareDistance: false)
        XCTAssertEqual(render, GesturePlan(path: .renderOnly, mixable: false, locationAnchor: true,
                                           activity: .lab(bench: "T1"), resume: false))
        let t5a = GesturePlan.make(.labStart(.init(bench: "T5a", audio: "full", anchor: "audio", mixable: false)),
                                   shareDistance: true)
        XCTAssertEqual(t5a.path, .t5aBypass)
        XCTAssertFalse(t5a.locationAnchor, "Lab anchors come from the run params, not the session")
        let check = GesturePlan.make(.audioCheckStart(mode: "tone"), shareDistance: true)
        XCTAssertEqual(check.path, .full)
        XCTAssertTrue(check.mixable)
        let labDefault = GestureCommand.classify(#"{"type":"labStart","bench":"T1","params":{"audio":"full"}}"#)
        XCTAssertEqual(labDefault, .labStart(.init(bench: "T1", audio: "full", anchor: "audio", mixable: true)),
                       "Lab runs default to the product (mixable) session; T2 can opt out")
        XCTAssertNil(check.activity)
        XCTAssertFalse(check.locationAnchor)
    }

    func testResultHelpers() {
        XCTAssertEqual(CommandJSON.error("useGestureCommand"), #"{"ok":false,"code":"useGestureCommand"}"#)
        XCTAssertTrue(CommandJSON.isOk(#"{"ok":true,"runId":"r-1"}"#))
        XCTAssertFalse(CommandJSON.isOk(#"{"ok":false,"code":"x"}"#))
        XCTAssertFalse(CommandJSON.isOk("garbage"))
    }
}
