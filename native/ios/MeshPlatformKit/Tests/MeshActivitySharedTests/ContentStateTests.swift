import Foundation
import MeshActivityShared
import XCTest

final class ContentStateTests: XCTestCase {
    private func full() -> IntercomAttributes.ContentState {
        IntercomAttributes.ContentState(
            live: true, muted: false, talk: .talking, pttLatched: true,
            speakers: ["Ana", "Luis"], inRange: 4, total: 6, ladder: .l1,
            onCall: false, alone: false, bluetoothOff: false,
            labStatus: LabStatus(bench: "T3", phase: .running, elapsedS: 42, linksUp: 2))
    }

    func testRoundTripThroughJSON() throws {
        let s = full()
        let data = try JSONEncoder().encode(s)
        let back = try JSONDecoder().decode(IntercomAttributes.ContentState.self, from: data)
        XCTAssertEqual(back, s)
        XCTAssertEqual(back.hashValue, s.hashValue)
    }

    func testWireShapeUsesCamelCaseAndApiRawValues() throws {
        let data = try JSONEncoder().encode(full())
        let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(Set(obj.keys), ["live", "muted", "talk", "pttLatched", "speakers", "inRange",
                                       "total", "ladder", "onCall", "alone", "bluetoothOff", "labStatus"])
        XCTAssertEqual(obj["talk"] as? String, "talking")
        XCTAssertEqual(obj["ladder"] as? String, "L1")
        let lab = try XCTUnwrap(obj["labStatus"] as? [String: Any])
        XCTAssertEqual(lab["phase"] as? String, "running")
        XCTAssertEqual(lab["elapsedS"] as? Int, 42)
    }

    func testLabStatusIsOptional() throws {
        var s = full()
        s.labStatus = nil
        let back = try JSONDecoder().decode(IntercomAttributes.ContentState.self,
                                            from: JSONEncoder().encode(s))
        XCTAssertNil(back.labStatus)
        let json = Data("""
        {"live":false,"muted":true,"talk":"muted","pttLatched":false,"speakers":[],"inRange":0,
         "total":1,"ladder":"L2","onCall":true,"alone":true,"bluetoothOff":true}
        """.utf8)
        let d = try JSONDecoder().decode(IntercomAttributes.ContentState.self, from: json)
        XCTAssertEqual(d.ladder, .l2)
        XCTAssertTrue(d.bluetoothOff)
        XCTAssertNil(d.labStatus)
    }

    func testSpeakersAreCappedAtTwoAndCountsClamped() throws {
        let s = IntercomAttributes.ContentState(speakers: ["a", "b", "c"], inRange: -3, total: -1)
        XCTAssertEqual(s.speakers, ["a", "b"])
        XCTAssertEqual(s.inRange, 0)
        XCTAssertEqual(s.total, 0)
        let json = Data("""
        {"live":true,"muted":false,"talk":"idle","pttLatched":false,"speakers":["x","y","z"],
         "inRange":1,"total":2,"ladder":"L0","onCall":false,"alone":false,"bluetoothOff":false}
        """.utf8)
        let d = try JSONDecoder().decode(IntercomAttributes.ContentState.self, from: json)
        XCTAssertEqual(d.speakers, ["x", "y"], "decoding applies the same cap")
    }

    func testUnknownEnumValueFailsDecoding() {
        let json = Data("""
        {"live":true,"muted":false,"talk":"shouting","pttLatched":false,"speakers":[],
         "inRange":1,"total":2,"ladder":"L0","onCall":false,"alone":false,"bluetoothOff":false}
        """.utf8)
        XCTAssertThrowsError(try JSONDecoder().decode(IntercomAttributes.ContentState.self, from: json))
    }

    func testAttributesAreCodableAndHashable() throws {
        let a = IntercomAttributes(kind: .lab, title: "T3")
        let back = try JSONDecoder().decode(IntercomAttributes.self, from: JSONEncoder().encode(a))
        XCTAssertEqual(back, a)
        XCTAssertEqual(TalkState.allCases.map(\.rawValue), ["idle", "talking", "blocked", "muted"])
        XCTAssertEqual(LadderLevel.allCases.map(\.rawValue), ["L0", "L1", "L2", "L3"])
    }
}
