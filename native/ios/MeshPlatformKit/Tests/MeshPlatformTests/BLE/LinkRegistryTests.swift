import Foundation
@testable import MeshPlatform
import XCTest

final class LinkRegistryTests: XCTestCase {
    private static func info(_ id: UInt32, _ kind: LinkKind = .bleL2cap) -> LinkInfo {
        LinkInfo(id: id, kind: kind, role: .initiator, maxFrame: 1026, unit: .stream, peerPlatformHint: .ios)
    }

    func testIdsAreUniqueAndNeverZero() {
        let r = LinkRegistry()
        var seen = Set<UInt32>()
        for _ in 0..<1000 { XCTAssertTrue(seen.insert(r.allocateId()).inserted) }
        XCTAssertFalse(seen.contains(0))
    }

    func testUpDataDownFanInExactlyOnce() {
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        let link = FakeLink(id: r.allocateId())
        r.up(link, info: Self.info(link.id))
        r.data(link.id, Data([1, 2]))
        r.writable(link.id)
        r.metrics(link.id, rssi: -60, attMtu: 185)
        XCTAssertTrue(r.down(link.id, reason: .peerClosed))
        XCTAssertFalse(r.down(link.id, reason: .timeout), "second down is a no-op")
        r.data(link.id, Data([3]))
        XCTAssertEqual(sink.events.values, [
            .up(link.id, Self.info(link.id)), .data(link.id, Data([1, 2])), .writable(link.id),
            .metrics(link.id, -60, 185), .down(link.id, .peerClosed),
        ])
        XCTAssertEqual(r.count, 0)
    }

    func testSendRoutesByIdAndReportsNoLink() {
        let r = LinkRegistry()
        let link = FakeLink(id: r.allocateId())
        XCTAssertEqual(r.send(link.id, Data([9])), .noLink)
        r.up(link, info: Self.info(link.id))
        link.outcome.withLock { $0 = .wouldBlock }
        XCTAssertEqual(r.send(link.id, Data([9])), .wouldBlock)
        XCTAssertEqual(link.sent.values, [Data([9])])
        XCTAssertEqual(r.send(999, Data()), .noLink)
    }

    func testDropAllByKindForRadioOff() {
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        let a = FakeLink(id: r.allocateId(), kind: .bleL2cap)
        let b = FakeLink(id: r.allocateId(), kind: .bleGatt)
        let c = FakeLink(id: r.allocateId(), kind: .wifiAware)
        r.up(a, info: Self.info(a.id, .bleL2cap))
        r.up(b, info: Self.info(b.id, .bleGatt))
        r.up(c, info: Self.info(c.id, .wifiAware))
        XCTAssertEqual(r.dropAll(kinds: [.bleL2cap, .bleGatt], reason: .radioOff), [a.id, b.id])
        XCTAssertEqual(sink.downs(a.id), [.radioOff])
        XCTAssertEqual(sink.downs(b.id), [.radioOff])
        XCTAssertEqual(r.ids(), [c.id])
        XCTAssertEqual(r.ids(kind: .wifiAware), [c.id])
    }

    func testSinkIsHeldWeakly() {
        let r = LinkRegistry()
        do {
            let sink = RecordingSink()
            r.setSink(sink)
            r.discovered(Discovery(discId: 1, transport: .ble, rssi: nil, tokens: nil, platformHint: .unknown))
            XCTAssertEqual(sink.events.values.count, 1)
        }
        // The sink is gone; events are dropped without crashing.
        r.helloRead(1, Data([0xC1]))
        r.connectFailed(1, reason: .limitReached)
    }

    func testConnectFailedAndHelloForwarded() {
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        r.helloRead(5, Data([0xC1]))
        r.connectFailed(5, reason: .limitReached)
        XCTAssertEqual(sink.events.values, [.hello(5, Data([0xC1])), .connectFailed(5, .limitReached)])
    }

    func testConcurrentUseIsSafe() {
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        DispatchQueue.concurrentPerform(iterations: 200) { _ in
            let l = FakeLink(id: r.allocateId())
            r.up(l, info: Self.info(l.id))
            r.data(l.id, Data([1]))
            _ = r.send(l.id, Data([2]))
            r.down(l.id, reason: .localClose)
        }
        XCTAssertEqual(r.count, 0)
        XCTAssertEqual(sink.events.values.filter { if case .down = $0 { return true }; return false }.count, 200)
    }
}
