import CMeshAudio
@preconcurrency import MeshCoreBindings
import MeshCoreFFI
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// The xcframework links, the generated bindings call into Rust, and the constants the
/// platform layer hard-codes agree with the core's single source of truth.
final class BindingsSmokeTests: XCTestCase {
    func testVersionsComeFromTheCore() {
        XCTAssertEqual(protocolVersion(), 1)
        XCTAssertEqual(apiVersion() >> 8, EngineHost.apiMajor)
        XCTAssertEqual(apiVersion() & 0xFF, 2, "UI contract 1.2 (DESIGN §3.7)")
        XCTAssertEqual(rtAbiVersion(), 1)
    }

    /// WP-14's `BleConstants.design` must equal the core's `ble_constants()` (§3.5).
    func testBleConstantsMatchTheCore() throws {
        let c = bleConstants()
        let mapped = try XCTUnwrap(BleConstants(c))
        XCTAssertEqual(mapped, BleConstants.design)
        XCTAssertEqual(c.connectTimeoutMs, 8_000)
        XCTAssertEqual(c.helloLen, 16)
    }

    /// `mesh_rt_stats` is registered as `cmesh_stats_fn`: the layouts must be identical.
    func testRtStatsLayoutsMatch() {
        XCTAssertEqual(MemoryLayout<MeshRtStats>.size, MemoryLayout<CMeshRtStats>.size)
        XCTAssertEqual(MemoryLayout<MeshRtStats>.alignment, MemoryLayout<CMeshRtStats>.alignment)
        XCTAssertEqual(MemoryLayout<MeshRtStats>.offset(of: \.generation),
                       MemoryLayout<CMeshRtStats>.offset(of: \.generation))
        XCTAssertEqual(MemoryLayout<MeshRtStats>.offset(of: \.capture_rms_dbfs),
                       MemoryLayout<CMeshRtStats>.offset(of: \.capture_rms_dbfs))
    }

    /// The RT ABI behind the C audio host: registered through `cmesh_set_sink`, the core's
    /// stats come back through `cmesh_read_stats`, and the flags reach the core.
    func testRtSinkRegistrationReachesTheCore() {
        let token = UnsafeMutableRawPointer(bitPattern: 0x5EED)
        cmesh_set_sink(mesh_rt_capture, mesh_rt_render, mesh_rt_set_flags,
                       unsafeBitCast(mesh_rt_stats as @convention(c) (UnsafeMutablePointer<MeshRtStats>?) -> Void,
                                     to: cmesh_stats_fn.self), token)
        defer { _ = cmesh_clear_sink(token) }
        XCTAssertTrue(cmesh_has_external_sink())
        var direct = MeshRtStats()
        mesh_rt_stats(&direct)
        var viaHost = CMeshRtStats()
        cmesh_read_stats(&viaHost)
        XCTAssertEqual(viaHost.generation, direct.generation)
        // Inactive rings (no session): render fills zeros, capture is discarded (§3.6).
        var pcm = [Int16](repeating: 7, count: 160)
        XCTAssertEqual(cmesh_dispatch_render(&pcm, 160, 0), 160)
        XCTAssertEqual(pcm, [Int16](repeating: 0, count: 160))
    }

    func testSessionStartHexParsing() {
        XCTAssertEqual(SessionStart.group(hex: "00112233445566778899aabbccddeeff"),
                       .group(groupId: Data([0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
                                             0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF])))
        XCTAssertNil(SessionStart.group(hex: "0011"))
        XCTAssertNil(SessionStart.group(hex: "zz112233445566778899aabbccddeeff"))
    }
}
