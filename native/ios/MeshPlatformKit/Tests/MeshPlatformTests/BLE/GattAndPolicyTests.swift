import CoreBluetooth
import Foundation
@testable import MeshPlatform
import Synchronization
import XCTest

final class FakeGattPort: GattTxPort {
    let room = Mutex(true)
    let sent = Recorder<Data>()
    let shutdowns = Recorder<Int>()
    func trySend(_ data: Data) -> Bool {
        guard room.withLock({ $0 }) else { return false }
        sent.add(data)
        return true
    }
    var maxUnit: Int { 182 }
    func shutdown() { shutdowns.add(1) }
}

final class GattLinkTests: XCTestCase {
    func testWouldBlockThenSingleWritableSignal() {
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        let port = FakeGattPort()
        let g = GattLink(id: r.allocateId(), role: .initiator, port: port, registry: r)
        r.up(g, info: LinkInfo(id: g.id, kind: .bleGatt, role: .initiator, maxFrame: g.maxFrame,
                               unit: .message, peerPlatformHint: .android))
        XCTAssertEqual(g.maxFrame, 182)
        XCTAssertEqual(g.send(Data([1])), .accepted)
        port.room.withLock { $0 = false }
        XCTAssertEqual(g.send(Data([2])), .wouldBlock)
        g.readyToSend() // OS: room again
        g.readyToSend() // no pending wish: no second signal
        XCTAssertEqual(sink.writables(g.id), 1)
        XCTAssertEqual(port.sent.values, [Data([1])], "a refused value is not queued natively")
    }

    func testInboundValuesAndDownOnce() {
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        let port = FakeGattPort()
        let g = GattLink(id: r.allocateId(), role: .responder, port: port, registry: r)
        r.up(g, info: LinkInfo(id: g.id, kind: .bleGatt, role: .responder, maxFrame: 182,
                               unit: .message, peerPlatformHint: .unknown))
        g.received(Data([0x80, 1]))
        g.received(Data([0x40, 2]))
        XCTAssertEqual(sink.received(g.id), Data([0x80, 1, 0x40, 2]), "values passed through unparsed")
        g.down(.peerClosed)
        g.close(reason: 0)
        g.received(Data([9]))
        XCTAssertEqual(sink.downs(g.id), [.peerClosed])
        XCTAssertEqual(port.shutdowns.values.count, 1)
        XCTAssertEqual(g.send(Data([1])), .noLink)
    }
}

final class ConnectTrackerTests: XCTestCase {
    func testOneOutstandingConnectAndEightSecondTimeout() {
        var t = ConnectTracker(timeout: 8)
        XCTAssertEqual(t.begin(1, now: 0), .start)
        XCTAssertEqual(t.begin(2, now: 1), .busy(1), "one connect per transport")
        XCTAssertEqual(t.begin(1, now: 1), .start, "re-begin of the same peer is allowed")
        XCTAssertNil(t.expired(now: 8.9))
        XCTAssertEqual(t.nextDeadline(now: 2), 9)
        XCTAssertEqual(t.expired(now: 9), 1)
        XCTAssertNil(t.current)
        XCTAssertEqual(t.begin(2, now: 9), .start)
        t.finish(2)
        XCTAssertNil(t.nextDeadline(now: 10))
    }

    func testAbortIgnoresPeerForIgnoreSeconds() {
        var t = ConnectTracker(timeout: 8)
        XCTAssertEqual(t.begin(3, now: 0), .start)
        t.ignore(3, seconds: 60, now: 1)
        XCTAssertNil(t.current, "abort ends the attempt")
        XCTAssertEqual(t.begin(3, now: 30), .ignored)
        XCTAssertEqual(t.begin(3, now: 61), .start)
    }

    func testArmedPendingReconnectDoesNotUseTheSlotOrTimeOut() {
        var t = ConnectTracker(timeout: 8)
        t.arm(5, untilS: 900, now: 0)
        XCTAssertTrue(t.isArmed(5, now: 100))
        XCTAssertEqual(t.begin(5, now: 0), .startPending)
        XCTAssertEqual(t.begin(6, now: 0), .start, "pending reconnects leave the slot free")
        XCTAssertEqual(t.expired(now: 8), 6)
        XCTAssertTrue(t.expiredPending(now: 899).isEmpty, "no 8 s cap while armed")
        XCTAssertEqual(t.nextDeadline(now: 10), 900)
        XCTAssertEqual(t.expiredPending(now: 900), [5], "cancelled when the window closes")
        XCTAssertEqual(t.armedPeers(now: 901), [])
    }

    func testDisarm() {
        var t = ConnectTracker(timeout: 8)
        t.arm(5, untilS: 60, now: 0)
        t.arm(5, untilS: 0, now: 1)
        XCTAssertFalse(t.isArmed(5, now: 2))
        XCTAssertEqual(t.begin(5, now: 2), .start)
    }

    func testDiscoveryTableIsStablePerPeer() {
        var d = DiscoveryTable(firstId: 100)
        let a = UUID(), b = UUID()
        XCTAssertEqual(d.id(for: a), 100)
        XCTAssertEqual(d.id(for: b), 101)
        XCTAssertEqual(d.id(for: a), 100)
        XCTAssertEqual(d.peer(for: 101), b)
        XCTAssertNil(d.existingId(for: UUID()))
    }
}

final class DiscoveryParserTests: XCTestCase {
    private let svc = BleConstants.design.meshService

    func testAndroidServiceData() throws {
        let sd = AdvBuilder.serviceData(android: true, lab: false, groupTag: [0xA1, 0xB2, 0xC3, 0xD4],
                                        nodeToken: [0x1F, 0x2E, 0x3D, 0x4C])
        let p = DiscoveryParser.parse(advertisement: [CBAdvertisementDataServiceDataKey: [svc: sd]], service: svc)
        XCTAssertEqual(p.platform, .android)
        let t = try XCTUnwrap(p.tokens)
        XCTAssertEqual(t.nodeToken, Data([0x1F, 0x2E, 0x3D, 0x4C]))
        XCTAssertEqual(t.groupTag, Data([0xA1, 0xB2, 0xC3, 0xD4]))
        XCTAssertEqual(t.tokenPrefix14, 1995)
    }

    func testIphoneForegroundLocalName() throws {
        let p = DiscoveryParser.parse(advertisement: [CBAdvertisementDataLocalNameKey: "UGZB6L",
                                                      CBAdvertisementDataServiceUUIDsKey: [svc]], service: svc)
        XCTAssertEqual(p.platform, .ios)
        let t = try XCTUnwrap(p.tokens)
        XCTAssertEqual(t.groupTag, Data([0xA1, 0xB2]))
        XCTAssertEqual(t.tokenPrefix14, 1995)
        XCTAssertNil(t.nodeToken)
    }

    func testBackgroundIphoneAndForeignDevices() {
        let bg = DiscoveryParser.parse(advertisement: [CBAdvertisementDataOverflowServiceUUIDsKey: [svc]], service: svc)
        XCTAssertNil(bg.tokens)
        XCTAssertEqual(bg.platform, .ios)
        let other = DiscoveryParser.parse(advertisement: [CBAdvertisementDataLocalNameKey: "Ana's AirPods"], service: svc)
        XCTAssertNil(other.tokens)
        XCTAssertEqual(other.platform, .unknown)
    }

    func testRssiClamp() {
        XCTAssertNil(DiscoveryParser.rssi(127))
        XCTAssertEqual(DiscoveryParser.rssi(-61), -61)
    }
}
