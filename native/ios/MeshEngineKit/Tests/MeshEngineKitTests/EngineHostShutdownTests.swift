@preconcurrency import MeshCoreBindings
import Foundation
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// Ordered engine shutdown (core `Engine::stop`, orchestrator request to WP-15): after
/// `stop()` the host keeps the links open and keeps executing the engine's radio
/// commands; the links close only through `RadioCommand.close`, after the leaving
/// records went out.
@MainActor
final class EngineHostShutdownTests: XCTestCase {
    func testStopLeavesLinkTeardownToTheEngine() throws {
        let air = FakeAir()
        let a = try HostRig(air: air, seed: 51)
        let b = try HostRig(air: air, seed: 52)
        a.host.setUiAttached(true)
        b.host.setUiAttached(true)
        try a.host.start(.lab(code: 7531))
        try b.host.start(.lab(code: 7531))
        waitUntil("Lab link up on both phones", timeout: 15) {
            a.snapshotLinks() == 1 && b.snapshotLinks() == 1
        }
        let closesBefore = air.executed(0).filter(Self.isClose).count
        XCTAssertEqual(closesBefore, 0)
        let sentBefore = air.sentUnits(0)

        a.host.stop()

        // The engine closes its link through the radio command, never the host directly,
        // and the radio stack survives the stop to execute it.
        waitUntil("engine-issued Close executed") { air.executed(0).contains(where: Self.isClose) }
        waitUntil("link closed on both ends") { air.openLinks(0) == 0 && air.openLinks(1) == 0 }
        XCTAssertTrue(a.host.radio.hasStack, "the drivers outlive stop()")
        XCTAssertGreaterThan(air.sentUnits(0), sentBefore, "CLOSE{leaving} went out before the platform close")
        // Discovery was switched off by the engine as part of the same stop.
        let afterStop = air.executed(0)
        XCTAssertTrue(afterStop.contains { if case .setDiscovery(.off, _) = $0 { return true } else { return false } })
        // The peer saw the link go and is alone again.
        waitUntil("peer link count back to 0") { b.snapshotLinks() == 0 }
        XCTAssertEqual(a.host.info().status, "idle")
        b.host.stop()
    }

    private static func isClose(_ c: RadioCommand) -> Bool {
        if case .close = c { return true }
        return false
    }
}
