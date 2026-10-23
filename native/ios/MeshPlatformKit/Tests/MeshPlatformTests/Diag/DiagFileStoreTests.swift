import Foundation
@testable import MeshPlatform
import XCTest

final class DiagFileStoreTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appending(path: "diag-\(UUID().uuidString)")
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    private func hdr(_ id: String, seg: Int) -> String {
        #"{"t":"hdr","v":1,"schema":"ridelog/1","seg":\#(seg),"sessionId":"\#(id)"}"#
    }

    private func filler(_ n: Int) -> [String] {
        (0..<n).map { #"{"t":"e","ts":\#($0),"src":"plat","ev":"pad","x":"\#(String(repeating: "a", count: 40))"}"# }
    }

    func testAppendCreatesSegmentsStartingWithTheirHeader() throws {
        let s = try DiagFileStore(directory: dir)
        let id = "d-20261022-2004-7k2q"
        try s.append(sessionId: id, seg: 1, lines: [hdr(id, seg: 1), #"{"t":"e","ts":0.1,"src":"plat","ev":"bt"}"#])
        try s.append(sessionId: id, seg: 1, lines: [#"{"t":"mark","ts":2,"label":"lag"}"#])
        try s.append(sessionId: id, seg: 2, lines: [hdr(id, seg: 2)])
        let info = try XCTUnwrap(s.sessions().first)
        XCTAssertEqual(info.sessionId, id)
        XCTAssertEqual(info.segments.map(\.lastPathComponent), ["seg-0001.jsonl", "seg-0002.jsonl"])
        let text = try String(contentsOf: s.segmentURL(sessionId: id, seg: 1), encoding: .utf8)
        XCTAssertEqual(text.split(separator: "\n").count, 3)
        XCTAssertTrue(text.hasPrefix(#"{"t":"hdr""#))
        XCTAssertTrue(text.hasSuffix("\n"))
        // Cleared mid-segment: lines without a header are dropped, never a headless file.
        s.clear()
        XCTAssertTrue(s.sessions().isEmpty)
        try s.append(sessionId: id, seg: 2, lines: [#"{"t":"s","ts":3}"#])
        XCTAssertTrue(s.sessions().isEmpty)
        XCTAssertEqual(s.droppedLines, 1)
        let excluded = try dir.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup
        XCTAssertEqual(excluded, true)
    }

    func testPathsAreValidated() throws {
        let s = try DiagFileStore(directory: dir)
        for bad in ["../x", "d-2026-1", "", "d-20261022-2004-7K2Q", "d-20261022-2004-7k2q/../x"] {
            XCTAssertThrowsError(try s.append(sessionId: bad, seg: 1, lines: ["{}"]))
        }
        XCTAssertThrowsError(try s.append(sessionId: "d-20261022-2004-aaaa", seg: 0, lines: ["{}"]))
        XCTAssertThrowsError(try s.append(sessionId: "d-20261022-2004-aaaa", seg: 1, lines: ["a\nb"]))
    }

    func testBudgetEvictsOldestSessionsButNeverTheCurrentOne() throws {
        let s = try DiagFileStore(directory: dir, maxBytes: 20_000, currentMaxBytes: 12_000)
        let old = ["d-20261020-1000-aaaa", "d-20261021-1000-bbbb", "d-20261022-1000-cccc"]
        for (i, id) in old.enumerated() {
            try s.append(sessionId: id, seg: 1, lines: [hdr(id, seg: 1)] + filler(80))
            let url = try s.segmentURL(sessionId: id, seg: 1)
            try FileManager.default.setAttributes(
                [.modificationDate: Date(timeIntervalSinceNow: Double(i - 10) * 3600)], ofItemAtPath: url.path)
        }
        let cur = "d-20261022-2004-dddd"
        for seg in 1...3 {
            try s.append(sessionId: cur, seg: UInt32(seg), lines: [hdr(cur, seg: seg)] + filler(60))
        }
        s.enforceBudget()
        let ids = s.sessions().map(\.sessionId)
        XCTAssertTrue(ids.contains(cur), "the current session is never evicted")
        XCTAssertFalse(ids.contains(old[0]), "the oldest session goes first")
        XCTAssertLessThanOrEqual(s.usage().bytes, 20_000)
        let curInfo = try XCTUnwrap(s.sessions().first { $0.sessionId == cur })
        XCTAssertLessThanOrEqual(curInfo.bytes, 12_000)
        XCTAssertEqual(curInfo.segments.last?.lastPathComponent, "seg-0003.jsonl", "the open segment stays")
    }

    func testRetentionDropsOldSessions() throws {
        let clock = Date(timeIntervalSince1970: 2_000_000_000)
        let s = try DiagFileStore(directory: dir, retention: 86_400, now: { clock })
        let id = "d-20261001-1000-aaaa"
        try s.append(sessionId: id, seg: 1, lines: [hdr(id, seg: 1)])
        let url = try s.segmentURL(sessionId: id, seg: 1)
        try FileManager.default.setAttributes([.modificationDate: clock.addingTimeInterval(-3 * 86_400)],
                                              ofItemAtPath: url.path)
        let fresh = "d-20261022-1000-bbbb"
        try s.append(sessionId: fresh, seg: 1, lines: [hdr(fresh, seg: 1)])
        try FileManager.default.setAttributes([.modificationDate: clock], ofItemAtPath:
            s.segmentURL(sessionId: fresh, seg: 1).path)
        s.enforceBudget()
        XCTAssertEqual(s.sessions().map(\.sessionId), [fresh])
    }

    func testExportZipsTheLastSessionOrTheRing() throws {
        let s = try DiagFileStore(directory: dir)
        let out = dir.appending(path: "out")
        let empty = try s.exportZip(scope: .last, label: "9f3a71c2-n4711", deviceJSON: "{}",
                                    ring: [hdr("d-20261022-2004-aaaa", seg: 1)], outDir: out)
        XCTAssertEqual(empty.sessions, 0)
        let ringEntries = DiagZip.entries(try Data(contentsOf: empty.url)).map(\.name)
        XCTAssertEqual(ringEntries, ["device.json", "diag/ring.jsonl"])
        for id in ["d-20261021-1000-aaaa", "d-20261022-1000-bbbb"] {
            try s.append(sessionId: id, seg: 1, lines: [hdr(id, seg: 1)])
            Thread.sleep(forTimeInterval: 0.02)
        }
        let last = try s.exportZip(scope: .last, label: "x", deviceJSON: #"{"model":"iPhone15,5"}"#, ring: nil, outDir: out)
        let entries = DiagZip.entries(try Data(contentsOf: last.url))
        XCTAssertEqual(entries.map(\.name), ["device.json", "diag/d-20261022-1000-bbbb/seg-0001.jsonl"])
        XCTAssertEqual(String(decoding: entries[1].data, as: UTF8.self), hdr("d-20261022-1000-bbbb", seg: 1) + "\n")
        XCTAssertEqual(last.url.lastPathComponent, "convlink-diag-x.zip")
        let all = try s.exportZip(scope: .all, label: "../all", deviceJSON: "{}", ring: nil, outDir: out)
        XCTAssertEqual(all.sessions, 2)
        XCTAssertEqual(all.url.lastPathComponent, "convlink-diag-all.zip")
    }

    func testZipCrcMatchesTheStandardCheckValue() {
        XCTAssertEqual(DiagZip.crc32(Data("123456789".utf8)), 0xCBF4_3926)
    }

    func testNotesAreCompactJsonWithoutNames() {
        let n = DiagNote.perm(name: "microphone", state: "granted", via: "check",
                              api: "AVAudioApplication.recordPermission")
        XCTAssertEqual(n.ev, "perm")
        XCTAssertEqual(n.fieldsJSON,
                       #"{"name":"microphone","state":"granted","via":"check","api":"AVAudioApplication.recordPermission"}"#)
        let q = DiagNote("x", [("s", .string("a\"b\n" + String(repeating: "z", count: 100))), ("i", .ints([1, 2]))])
        XCTAssertTrue(q.fieldsJSON.hasPrefix(#"{"s":"a\"b\nzz"#))
        XCTAssertTrue(q.fieldsJSON.hasSuffix(#""i":[1,2]}"#))
        let route = AudioRouteInfo(kind: .hfpWideband, inputRate: 16_000, output: .bluetoothHfp, headsetPresent: true)
        let r = DiagNote.audioRoute(route, prev: nil)
        XCTAssertTrue(r.fieldsJSON.contains(#""headsetClass":"bluetooth""#))
        XCTAssertEqual(DiagNote.audioIo(.interrupted(.otherSession)).ev, "interruption")
    }
}
