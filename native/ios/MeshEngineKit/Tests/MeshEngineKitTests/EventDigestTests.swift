import Foundation
import MeshActivityShared
@testable import MeshEngineKit
import MeshPlatform
import XCTest

final class EventDigestTests: XCTestCase {
    private func fixture(_ name: String) throws -> String {
        let list = try XCTUnwrap(RepoFiles.json("crates/mesh-api/fixtures/events.json") as? [[String: Any]])
        let e = try XCTUnwrap(list.first { $0["name"] as? String == name }?["event"], name)
        return String(decoding: try JSONSerialization.data(withJSONObject: e), as: UTF8.self)
    }

    private func firstSnapshotFixture() throws -> String {
        let list = try XCTUnwrap(RepoFiles.json("crates/mesh-api/fixtures/events.json") as? [[String: Any]])
        let e = try XCTUnwrap(list.first { ($0["event"] as? [String: Any])?["type"] as? String == "snapshot" }?["event"])
        return String(decoding: try JSONSerialization.data(withJSONObject: e), as: UTF8.self)
    }

    /// The riding snapshot fixture becomes the Live Activity content (contract C9).
    func testRideSnapshotBecomesTheLiveActivityContent() throws {
        var d = EventDigest()
        XCTAssertTrue(d.apply(try firstSnapshotFixture()))
        XCTAssertTrue(d.live)
        XCTAssertTrue(d.shareDistance)
        XCTAssertEqual(d.groupName, "Sunday ride")
        XCTAssertFalse(d.speakers.contains("Ana"), "my own talker entry is not a speaker")
        XCTAssertTrue(d.speakers.contains("Carla"))
        XCTAssertGreaterThan(d.total, 0)
        let s = d.contentState(hardMuted: false, pttLatched: false)
        XCTAssertEqual(s.talk, .talking)
        XCTAssertEqual(s.ladder, .l0)
        XCTAssertLessThanOrEqual(s.speakers.count, IntercomAttributes.ContentState.maxSpeakers)
        XCTAssertEqual(d.contentState(hardMuted: true, pttLatched: false).talk, .muted)
        XCTAssertTrue(d.contentState(hardMuted: true, pttLatched: false).muted)
        XCTAssertFalse(d.apply(try firstSnapshotFixture()), "same snapshot: no change")
    }

    func testLabProgressAndFinishFeedTheLabStatus() throws {
        var d = EventDigest()
        d.labStarted(bench: "T0", runId: "r-20261023-0912-ab12")
        XCTAssertEqual(d.lab?.phase, .waiting)
        XCTAssertTrue(d.apply(try fixture("labProgress")))
        XCTAssertEqual(d.lab, LabStatus(bench: "T0", phase: .running, elapsedS: 12, linksUp: 1))
        XCTAssertTrue(d.apply(try fixture("labFinished")))
        XCTAssertEqual(d.lab?.phase, .finished)
        XCTAssertEqual(d.lab?.verdict, "go")
        XCTAssertEqual(d.lab?.elapsedS, 60)
        XCTAssertTrue(d.apply(try fixture("labFinished inconclusive")))
        XCTAssertEqual(d.lab?.phase, .failed)
        XCTAssertEqual(d.contentState(hardMuted: false, pttLatched: false).labStatus?.phase, .failed)
    }

    func testCallNoticesAndUnknownEvents() {
        var d = EventDigest()
        XCTAssertTrue(d.apply(#"{"type":"notice","key":"onCall","params":{}}"#))
        XCTAssertTrue(d.onCall)
        XCTAssertTrue(d.apply(#"{"type":"notice","key":"callEnded","params":{}}"#))
        XCTAssertFalse(d.onCall)
        XCTAssertFalse(d.apply(#"{"type":"routeChanged","route":"wired"}"#))
        XCTAssertFalse(d.apply("nonsense"))
    }

    func testNotificationCatalogSubstitutesParams() {
        let c = NotificationCatalog(title: "convLink Voice", templates: [
            "memberLost": "Out of range ({{count}}): {{names}}",
            "batteryLow": "Battery low ({{percent}} %)",
        ])
        XCTAssertEqual(c.body("memberLost", params: ["count": 2, "names": ["Ana", "Luis"]]),
                       "Out of range (2): Ana, Luis")
        XCTAssertEqual(c.body("batteryLow", params: ["percent": 15]), "Battery low (15 %)")
        XCTAssertNil(c.body("unknownKey", params: [:]))
    }
}
