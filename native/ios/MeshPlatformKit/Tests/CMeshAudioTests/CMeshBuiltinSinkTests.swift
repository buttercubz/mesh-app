import CMeshAudio
import Foundation
import XCTest

final class CMeshBuiltinSinkTests: XCTestCase {
    override func setUp() {
        super.setUp()
        resetCMesh()
        cmesh_set_capture_muted(false)
    }

    private func render(_ n: Int) -> [Int16] {
        var out = [Int16](repeating: 0, count: n)
        _ = cmesh_dispatch_render(&out, UInt32(n), 0)
        return out
    }

    func testLoopbackReturnsCapturedSamplesInOrder() {
        cmesh_use_builtin(CMESH_BUILTIN_LOOPBACK)
        _ = render(16) // applies the reset request on the render side
        let a = (0..<160).map { Int16($0) }
        let b = (160..<320).map { Int16($0) }
        _ = cmesh_dispatch_capture(a, 160, 0)
        _ = cmesh_dispatch_capture(b, 160, 0)
        XCTAssertEqual(render(200), Array((a + b)[0..<200]))
        let tail = render(160)
        XCTAssertEqual(Array(tail[0..<120]), Array(b[40..<160]))
        XCTAssertTrue(tail[120...].allSatisfy { $0 == 0 }, "underrun renders silence")
    }

    func testLoopbackOverrunDropsNewestAndNeverBlocks() {
        cmesh_use_builtin(CMESH_BUILTIN_LOOPBACK)
        _ = render(1)
        let block = [Int16](repeating: 7, count: 4096)
        XCTAssertEqual(cmesh_dispatch_capture(block, 4096, 0), 4096)
        XCTAssertEqual(cmesh_dispatch_capture(block, 4096, 0), 4096)
        XCTAssertEqual(cmesh_dispatch_capture(block, 4096, 0), 0, "ring full: nothing accepted")
    }

    func testToneHasConfiguredFrequencyAndLevel() {
        cmesh_use_builtin(CMESH_BUILTIN_TONE)
        cmesh_tone_configure(1000, -20)
        let out = render(1600) // 100 ms
        let rms = sqrt(out.reduce(0.0) { $0 + Double($1) * Double($1) } / Double(out.count))
        let db = 20 * log10(rms / 32768)
        XCTAssertEqual(db, -23.01, accuracy: 0.2, "sine at -20 dBFS peak = -23 dBFS RMS")
        var crossings = 0
        for i in 1..<out.count where (out[i - 1] < 0) != (out[i] < 0) { crossings += 1 }
        XCTAssertEqual(crossings, 200, accuracy: 2, "1 kHz over 100 ms → 200 zero crossings")
    }

    func testToneOutOfRangeIsSilent() {
        cmesh_use_builtin(CMESH_BUILTIN_TONE)
        cmesh_tone_configure(9000, -6)
        XCTAssertTrue(render(160).allSatisfy { $0 == 0 })
    }

    /// T5a: the click train goes out, the capture is the same signal delayed by a known
    /// round trip, and the onset search recovers that delay.
    func testClickTrainLatencyEstimate() {
        cmesh_use_builtin(CMESH_BUILTIN_CLICK)
        cmesh_click_configure(1600, 16_000, -6)
        let delay = 37
        var history = [Int16](repeating: 0, count: delay)
        for _ in 0..<100 { // 1 s in 10 ms blocks, duplex like the IO thread
            let out = render(160)
            history += out
            let captured = Array(history[0..<160])
            history.removeFirst(160)
            _ = cmesh_dispatch_capture(captured, 160, 0)
        }
        XCTAssertEqual(cmesh_click_emitted(), 10)
        XCTAssertEqual(cmesh_click_captured_frames(), 16_000)
        var found: UInt32 = 0
        let lag = cmesh_click_estimate_latency_frames(-30, &found)
        XCTAssertGreaterThanOrEqual(found, 9)
        XCTAssertTrue((delay...delay + 4).contains(Int(lag)), "lag \(lag) ≈ \(delay) (+ burst rise)")
    }

    func testClickEstimateWithoutCaptureIsMinusOne() {
        cmesh_use_builtin(CMESH_BUILTIN_CLICK)
        cmesh_click_configure(800, 1000, -6)
        _ = render(1600)
        var found: UInt32 = 99
        XCTAssertEqual(cmesh_click_estimate_latency_frames(-30, &found), -1)
        XCTAssertEqual(found, 0)
    }

    func testBlockDbfsOfSilenceAndHalfScale() {
        cmesh_use_builtin(CMESH_BUILTIN_METER)
        _ = cmesh_dispatch_capture([Int16](repeating: 16384, count: 160), 160, 0)
        var cap: Float = 0
        cmesh_meter_read(&cap, nil)
        XCTAssertEqual(cap, -6.02, accuracy: 0.05)
    }
}
