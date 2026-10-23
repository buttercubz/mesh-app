import CMeshAudio
import XCTest

// Global sink state for the C function-pointer callbacks (they cannot capture context).
// Tests in this file run serially and reset it in setUp.
nonisolated(unsafe) var gPushed: [Int16] = []
nonisolated(unsafe) var gPushTimes: [UInt64] = []
nonisolated(unsafe) var gPullCalls = 0
nonisolated(unsafe) var gFlagsSeen: [UInt32] = []
nonisolated(unsafe) var gStatsCalls = 0

let testPush: cmesh_push_fn = { pcm, frames, t in
    if let pcm { gPushed += Array(UnsafeBufferPointer(start: pcm, count: Int(frames))) }
    gPushTimes.append(t)
    return Int32(frames)
}

let testPull: cmesh_pull_fn = { pcm, frames, _ in
    gPullCalls += 1
    for i in 0..<Int(frames) { pcm?[i] = Int16(i) }
    return Int32(frames)
}

let testFlags: cmesh_flags_fn = { f in gFlagsSeen.append(f) }

let testStats: cmesh_stats_fn = { out in
    gStatsCalls += 1
    out?.pointee.capture_callbacks = 4242
    out?.pointee.generation = 7
}

/// Resets the process-global C host to a known state.
func resetCMesh() {
    _ = cmesh_clear_sink(nil)
    cmesh_use_builtin(CMESH_BUILTIN_SILENCE)
    cmesh_reset_counters()
    cmesh_set_capture_muted(true)
    cmesh_set_flags(0)
    gPushed = []
    gPushTimes = []
    gPullCalls = 0
    gFlagsSeen = []
    gStatsCalls = 0
}

final class CMeshDispatchTests: XCTestCase {
    override func setUp() {
        super.setUp()
        resetCMesh()
    }

    func testMutedCaptureNeverReachesTheSink() {
        cmesh_set_sink(testPush, testPull, nil, nil, nil)
        let pcm = [Int16](repeating: 1000, count: 160)
        XCTAssertEqual(cmesh_dispatch_capture(pcm, 160, 1), 0)
        XCTAssertTrue(gPushed.isEmpty, "the unit starts muted (§3.2)")
        var c = CMeshIoCounters()
        cmesh_io_counters(&c)
        XCTAssertEqual(c.input_callbacks, 1, "callbacks still count while muted")
        cmesh_set_capture_muted(false)
        XCTAssertEqual(cmesh_dispatch_capture(pcm, 160, 99), 160)
        XCTAssertEqual(gPushed, pcm)
        XCTAssertEqual(gPushTimes, [99])
    }

    func testRenderUsesExternalPullThenBuiltinAfterClear() {
        let token = UnsafeMutableRawPointer(bitPattern: 0x1234)
        cmesh_set_sink(testPush, testPull, nil, nil, token)
        var out = [Int16](repeating: -1, count: 80)
        XCTAssertEqual(cmesh_dispatch_render(&out, 80, 0), 80)
        XCTAssertEqual(gPullCalls, 1)
        XCTAssertEqual(out[79], 79)
        XCTAssertFalse(cmesh_clear_sink(UnsafeMutableRawPointer(bitPattern: 0x9999)), "foreign token")
        XCTAssertTrue(cmesh_has_external_sink())
        XCTAssertTrue(cmesh_clear_sink(token))
        XCTAssertFalse(cmesh_has_external_sink())
        _ = cmesh_dispatch_render(&out, 80, 0)
        XCTAssertEqual(gPullCalls, 1)
        XCTAssertTrue(out.allSatisfy { $0 == 0 }, "built-in silence")
    }

    func testFlagsAreStoredAndForwarded() {
        cmesh_set_flags(CMESH_FLAG_PTT_DOWN)
        cmesh_set_sink(nil, nil, testFlags, nil, nil)
        XCTAssertEqual(gFlagsSeen, [CMESH_FLAG_PTT_DOWN], "registration replays the current flags")
        let v = cmesh_update_flags(CMESH_FLAG_HARD_MUTE, CMESH_FLAG_PTT_DOWN, CMESH_FLAG_PTT_LATCH_TOGGLE)
        XCTAssertEqual(v, CMESH_FLAG_HARD_MUTE | CMESH_FLAG_PTT_LATCH_TOGGLE)
        XCTAssertEqual(cmesh_flags(), v)
        XCTAssertEqual(gFlagsSeen.last, v)
        XCTAssertEqual(cmesh_update_flags(0, 0, CMESH_FLAG_PTT_LATCH_TOGGLE), CMESH_FLAG_HARD_MUTE)
    }

    func testStatsFromSinkOrFromHostCounters() {
        cmesh_set_capture_muted(false)
        let pcm = [Int16](repeating: 0, count: 160)
        _ = cmesh_dispatch_capture(pcm, 160, 0)
        _ = cmesh_dispatch_capture(pcm, 160, 0)
        var s = CMeshRtStats()
        cmesh_read_stats(&s)
        XCTAssertEqual(s.capture_callbacks, 2)
        XCTAssertEqual(s.captured_frames, 320)
        XCTAssertEqual(s.capture_rms_dbfs, -120, accuracy: 0.01)
        cmesh_set_sink(nil, nil, nil, testStats, nil)
        cmesh_read_stats(&s)
        XCTAssertEqual(gStatsCalls, 1)
        XCTAssertEqual(s.capture_callbacks, 4242)
        XCTAssertEqual(s.generation, 7)
    }

    func testStatsLayoutMatchesMeshRtStats() {
        // mesh_rt.h (C5): u64 x2, u32 x4, f32 x2, u32 → 44 B of fields, sizeof 48.
        XCTAssertEqual(MemoryLayout<CMeshRtStats>.size, 48)
        XCTAssertEqual(MemoryLayout<CMeshRtStats>.stride, 48)
        XCTAssertEqual(MemoryLayout<CMeshRtStats>.offset(of: \CMeshRtStats.capture_callbacks), 24)
        XCTAssertEqual(MemoryLayout<CMeshRtStats>.offset(of: \CMeshRtStats.generation), 40)
    }

    func testMeterReportsBlockLevels() {
        cmesh_use_builtin(CMESH_BUILTIN_METER)
        cmesh_set_capture_muted(false)
        // Full-scale square wave: 0 dBFS (within rounding).
        let square = (0..<160).map { $0 % 2 == 0 ? Int16(32767) : Int16(-32767) }
        _ = cmesh_dispatch_capture(square, 160, 0)
        var cap: Float = 0, ren: Float = 0
        cmesh_meter_read(&cap, &ren)
        XCTAssertEqual(cap, 0, accuracy: 0.01)
        var out = [Int16](repeating: 5, count: 160)
        _ = cmesh_dispatch_render(&out, 160, 0)
        cmesh_meter_read(&cap, &ren)
        XCTAssertEqual(ren, -120, accuracy: 0.01, "meter sink renders silence")
    }
}
