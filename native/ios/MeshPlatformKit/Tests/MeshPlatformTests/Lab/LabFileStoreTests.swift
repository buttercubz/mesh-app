import Foundation
@testable import MeshPlatform
import XCTest

final class LabFileStoreTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appending(path: "lab-\(UUID().uuidString)")
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
    }

    func testAppendListSummaryExportDelete() throws {
        let s = try LabFileStore(directory: dir)
        let run = "r-20261023-0912-ab12"
        try s.append(runId: run, line: #"{"t":"run","v":1,"runId":"r-20261023-0912-ab12","bench":"T0"}"#)
        try s.append(runId: run, line: #"{"t":"s","ts":1.000}"#)
        try s.append(runId: run, line: #"{"t":"summary","verdict":"go"}"#)
        s.finish(runId: run)
        try s.append(runId: run, line: #"{"t":"mark","ts":20.0,"label":"pocket"}"#) // reopens
        XCTAssertEqual(try s.lines(runId: run).count, 4)
        XCTAssertEqual(try s.lines(runId: run).first?.hasPrefix(#"{"t":"run""#), true)
        let raw = try String(contentsOf: s.url(for: run), encoding: .utf8)
        XCTAssertTrue(raw.hasSuffix("\n"), "JSON Lines: newline-terminated")
        try s.writeSummary(runId: run, csv: "runId,bench,verdict\n\(run),T0,go\n")
        let listed = try XCTUnwrap(s.list().first)
        XCTAssertEqual(listed.runId, run)
        XCTAssertGreaterThan(listed.bytes, 50)
        XCTAssertNotNil(listed.summaryURL)
        XCTAssertEqual(try s.exportURLs(runId: run).map(\.lastPathComponent),
                       ["\(run).jsonl", "\(run)-summary.csv"])
        try s.delete(runId: run)
        XCTAssertTrue(s.list().isEmpty)
        XCTAssertEqual(try s.exportURLs(runId: run), [])
    }

    func testRunIdsCannotEscapeTheDirectory() throws {
        let s = try LabFileStore(directory: dir)
        for bad in ["../x", "a/b", "", ".hidden", "r\n1", String(repeating: "a", count: 97), "ü"] {
            XCTAssertFalse(LabFileStore.validate(bad), bad)
            XCTAssertThrowsError(try s.append(runId: bad, line: "{}"))
        }
        XCTAssertTrue(LabFileStore.validate("r-20261023-0912-ab12"))
        XCTAssertThrowsError(try s.append(runId: "ok", line: "two\nlines"))
    }

    func testListIsNewestFirstAndIgnoresOtherFiles() throws {
        let s = try LabFileStore(directory: dir)
        try s.append(runId: "r-1", line: "{}")
        try s.append(runId: "r-2", line: "{}")
        let old = Date(timeIntervalSinceNow: -3600)
        try FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: s.url(for: "r-1").path)
        try Data("x".utf8).write(to: dir.appending(path: "notes.txt"))
        XCTAssertEqual(s.list().map(\.runId), ["r-2", "r-1"])
    }

    func testConcurrentAppendsKeepWholeLines() throws {
        let s = try LabFileStore(directory: dir)
        DispatchQueue.concurrentPerform(iterations: 200) { i in
            try? s.append(runId: "r-c", line: #"{"t":"e","i":\#(i)}"#)
        }
        s.finish(runId: "r-c")
        let lines = try s.lines(runId: "r-c")
        XCTAssertEqual(lines.count, 200)
        XCTAssertTrue(lines.allSatisfy { $0.hasPrefix(#"{"t":"e""#) && $0.hasSuffix("}") })
    }

    func testDefaultDirectoryIsDocumentsLab() throws {
        let s = try LabFileStore()
        XCTAssertEqual(s.directory.lastPathComponent, "lab")
        XCTAssertEqual(s.directory.deletingLastPathComponent().lastPathComponent, "Documents")
    }
}
