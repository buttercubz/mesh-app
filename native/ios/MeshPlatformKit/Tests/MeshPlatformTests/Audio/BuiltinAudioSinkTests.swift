import CMeshAudio
import Foundation
@testable import MeshPlatform
import XCTest

final class BuiltinAudioSinkTests: XCTestCase {
    override func setUp() {
        super.setUp()
        _ = cmesh_clear_sink(nil)
        cmesh_set_capture_muted(false)
        cmesh_reset_counters()
    }

    override func tearDown() {
        BuiltinAudioSink.select(.silence)
        cmesh_set_capture_muted(true)
        super.tearDown()
    }

    func testClickRoundTripInMilliseconds() throws {
        BuiltinAudioSink.select(.click(periodMs: 100, captureSeconds: 2, dbfs: -6))
        let delay = 320 // 20 ms
        var line = [Int16](repeating: 0, count: delay)
        for _ in 0..<100 {
            var out = [Int16](repeating: 0, count: 160)
            _ = cmesh_dispatch_render(&out, 160, 0)
            line += out
            _ = cmesh_dispatch_capture(Array(line[0..<160]), 160, 0)
            line.removeFirst(160)
        }
        let r = try XCTUnwrap(BuiltinAudioSink.clickRoundTripMs())
        XCTAssertEqual(r.ms, 20, accuracy: 0.5)
        XCTAssertGreaterThanOrEqual(r.found, 9)
        XCTAssertEqual(BuiltinAudioSink.counters().render_callbacks, 100)
    }

    func testToneAndLevels() {
        BuiltinAudioSink.select(.tone(hz: 500, dbfs: -12))
        var out = [Int16](repeating: 0, count: 320)
        _ = cmesh_dispatch_render(&out, 320, 0)
        XCTAssertEqual(BuiltinAudioSink.levels().render, -15, accuracy: 0.3)
        BuiltinAudioSink.select(.click(periodMs: 100, captureSeconds: 1, dbfs: -6))
        XCTAssertNil(BuiltinAudioSink.clickRoundTripMs(), "nothing captured yet")
    }
}
