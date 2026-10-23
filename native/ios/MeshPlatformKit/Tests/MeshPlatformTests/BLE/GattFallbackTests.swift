import CoreBluetooth
import Foundation
@testable import MeshPlatform
import Synchronization
import XCTest

/// Stands in for a connected `CBPeripheral` + `RX` characteristic.
final class FakeWriteTarget: GattWriteTarget, @unchecked Sendable {
    struct State {
        var connected = true
        var canSend = true
        var maxWrite = 244
        var written: [Data] = []
        var cancels = 0
    }

    let s = Mutex(State())
    var isConnected: Bool { s.withLock { $0.connected } }
    var canSendWriteWithoutResponse: Bool { s.withLock { $0.canSend } }
    var maxWriteWithoutResponse: Int { s.withLock { $0.maxWrite } }
    func writeWithoutResponse(_ data: Data) { s.withLock { $0.written.append(data) } }
    func cancelConnection() { s.withLock { $0.cancels += 1 } }
}

final class CentralGattPortTests: XCTestCase {
    func testUnitFollowsMaximumWriteValueLengthWithAttFloor() {
        let q = BleQueue(label: "t.gatt")
        let t = FakeWriteTarget()
        XCTAssertEqual(CentralGattPort(q: q, target: t).maxUnit, 244)
        t.s.withLock { $0.maxWrite = 0 }
        XCTAssertEqual(CentralGattPort(q: q, target: t).maxUnit, 20, "ATT minimum (MTU 23)")
    }

    func testEndToEndLinkWritesWithoutResponseAndSignalsWritableOnce() {
        let q = BleQueue(label: "t.gatt")
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        let t = FakeWriteTarget()
        t.s.withLock { $0.maxWrite = 182 }
        let g = GattLink(id: r.allocateId(), role: .initiator, port: CentralGattPort(q: q, target: t),
                         registry: r)
        r.up(g, info: LinkInfo(id: g.id, kind: .bleGatt, role: .initiator, maxFrame: g.maxFrame,
                               unit: .message, peerPlatformHint: .android, discId: 7))
        XCTAssertEqual(g.maxFrame, 182, "the core chunks GATT frames to this size")

        // Outbound: one ATT write per unit, refused (not queued) while the OS has no room.
        XCTAssertEqual(g.send(Data(repeating: 1, count: 182)), .accepted)
        t.s.withLock { $0.canSend = false }
        XCTAssertEqual(g.send(Data([2])), .wouldBlock)
        g.readyToSend() // peripheralIsReady(toSendWriteWithoutResponse:)
        g.readyToSend()
        XCTAssertEqual(sink.writables(g.id), 1)
        t.s.withLock { $0.canSend = true }
        XCTAssertEqual(g.send(Data([2])), .accepted)
        XCTAssertEqual(t.s.withLock { $0.written }, [Data(repeating: 1, count: 182), Data([2])])

        // Inbound: TX notifications pass through unparsed.
        g.received(Data([0x80, 9]))
        XCTAssertEqual(sink.received(g.id), Data([0x80, 9]))

        // Down: the connection is cancelled on the driver queue and linkDown fires once.
        g.down(.localClose)
        XCTAssertTrue(waitUntil { t.s.withLock { $0.cancels } == 1 })
        XCTAssertEqual(sink.downs(g.id), [.localClose])
        XCTAssertEqual(g.send(Data([3])), .noLink)
    }

    func testDisconnectedTargetRefusesAndOversizeIsDroppedAndReported() {
        let q = BleQueue(label: "t.gatt")
        let t = FakeWriteTarget()
        t.s.withLock { $0.maxWrite = 20 }
        let reports = Recorder<[Int]>()
        let port = CentralGattPort(q: q, target: t, diag: { reports.add([$0, $1]) })
        XCTAssertTrue(port.trySend(Data(repeating: 0, count: 21)), "dropped, never retried")
        XCTAssertEqual(reports.values, [[21, 20]])
        XCTAssertTrue(t.s.withLock { $0.written }.isEmpty, "CoreBluetooth would truncate it")
        t.s.withLock { $0.connected = false }
        XCTAssertFalse(port.trySend(Data([1])))
    }
}

/// Driver-level checks that run without a radio (the simulator's managers never reach
/// `poweredOn`): commands that cannot be executed are reported, never dropped silently.
final class CentralDriverFallbackTests: XCTestCase {
    func testOpenLinkOnUnknownPeerReportsConnectFailed() {
        let sink = RecordingSink()
        let r = LinkRegistry(sink: sink)
        let io = LinkIOThread(name: "t.io")
        let d = BleCentralDriver(constants: .design, registry: r, io: io, restore: false)
        d.apply(.openLink(discId: 42, via: .gatt))
        XCTAssertTrue(waitUntil { sink.events.values.contains(.connectFailed(42, .error(-1))) })
    }

    func testDiagnosticsReportConnectSkips() {
        let r = LinkRegistry(sink: RecordingSink())
        let d = BleCentralDriver(constants: .design, registry: r, io: LinkIOThread(name: "t.io"),
                                 restore: false)
        let events = Recorder<BleDiagEvent>()
        d.onDiagnostics { events.add($0) }
        d.apply(.connect(discId: 5))
        XCTAssertTrue(waitUntil { events.values.contains(.connectSkipped(discId: 5, why: "unknown")) })
    }
}

final class PeripheralDriverListenerTests: XCTestCase {
    func testPsmReportedOnlyWhileListenerIsLive() throws {
        let r = LinkRegistry(sink: RecordingSink())
        let d = BlePeripheralDriver(constants: .design, registry: r, io: LinkIOThread(name: "t.io"),
                                    restore: false)
        let psms = Recorder<UInt16?>()
        let events = Recorder<BleDiagEvent>()
        d.onDiagnostics { events.add($0) }
        d.onPsm { psms.add($0) }
        let mgr = try XCTUnwrap(d.q.sync { d.manager })
        d.q.sync {
            _ = d.listener.wantListener() // as publishService would on poweredOn
            d.peripheralManager(mgr, didPublishL2CAPChannel: 0x81, error: nil)
            d.peripheralManager(mgr, didPublishL2CAPChannel: 0x81, error: nil) // duplicate: no report
            d.peripheralManager(mgr, didUnpublishL2CAPChannel: 0x81, error: nil)
        }
        XCTAssertTrue(waitUntil { psms.values.count == 3 })
        XCTAssertEqual(psms.values, [nil, 0x81, nil], "a dead listener is withdrawn from HELLO")
        XCTAssertTrue(events.values.contains(.l2capListener(psm: nil, error: nil, retryIn: 1)),
                      "re-publish scheduled")

        let err = NSError(domain: CBErrorDomain, code: CBError.Code.unknown.rawValue)
        d.q.sync {
            d.listener = L2capListenerState()
            _ = d.listener.wantListener()
            d.peripheralManager(mgr, didPublishL2CAPChannel: 0, error: err)
        }
        XCTAssertTrue(waitUntil {
            events.values.contains(.l2capListener(psm: nil, error: BleErrorInfo(err), retryIn: nil))
        })
        XCTAssertNil(d.q.sync { d.psm })
    }
}
