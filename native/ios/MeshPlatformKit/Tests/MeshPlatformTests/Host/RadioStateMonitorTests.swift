import CoreBluetooth
import Foundation
@testable import MeshPlatform
import XCTest

final class RadioStateMonitorTests: XCTestCase {
    func testManagerStateMapping() {
        XCTAssertEqual(RadioStateMonitor.map(.poweredOn), .on)
        XCTAssertEqual(RadioStateMonitor.map(.poweredOff), .off)
        XCTAssertEqual(RadioStateMonitor.map(.unauthorized), .unauthorized)
        XCTAssertEqual(RadioStateMonitor.map(.unsupported), .unsupported)
        XCTAssertNil(RadioStateMonitor.map(.resetting))
        XCTAssertNil(RadioStateMonitor.map(.unknown))
    }

    func testReportsEachDistinctCombinedStateOnce() {
        let seen = Recorder<RadioState>()
        let m = RadioStateMonitor { seen.add($0) }
        m.update(.central, .unknown)
        m.update(.central, .poweredOn)
        m.update(.peripheral, .poweredOn)
        m.update(.central, .poweredOff) // airplane mode: the worse state wins
        m.update(.peripheral, .poweredOff)
        m.update(.central, .poweredOn)
        m.update(.peripheral, .poweredOn)
        XCTAssertEqual(seen.values, [.on, .off, .on])
        XCTAssertEqual(m.current, .on)
    }

    func testUnauthorizedOutranksOff() {
        XCTAssertEqual(RadioStateMonitor.combine(.off, .unauthorized), .unauthorized)
        XCTAssertEqual(RadioStateMonitor.combine(nil, .on), .on)
        XCTAssertNil(RadioStateMonitor.combine(nil, nil))
    }

    func testConnectionLimitMapsToLimitReached() {
        let limit = NSError(domain: CBErrorDomain, code: CBError.Code.connectionLimitReached.rawValue)
        XCTAssertEqual(RadioStateMonitor.downReason(for: limit, cancelledLocally: false), .limitReached)
        let timeout = NSError(domain: CBErrorDomain, code: CBError.Code.connectionTimeout.rawValue)
        XCTAssertEqual(RadioStateMonitor.downReason(for: timeout, cancelledLocally: false), .timeout)
        let gone = NSError(domain: CBErrorDomain, code: CBError.Code.peripheralDisconnected.rawValue)
        XCTAssertEqual(RadioStateMonitor.downReason(for: gone, cancelledLocally: false), .peerClosed)
        let other = NSError(domain: CBErrorDomain, code: CBError.Code.encryptionTimedOut.rawValue)
        XCTAssertEqual(RadioStateMonitor.downReason(for: other, cancelledLocally: false),
                       .error(Int32(CBError.Code.encryptionTimedOut.rawValue)))
        XCTAssertEqual(RadioStateMonitor.downReason(for: nil, cancelledLocally: true), .localClose)
        XCTAssertEqual(RadioStateMonitor.downReason(for: nil, cancelledLocally: false), .peerClosed)
        let att = NSError(domain: CBATTErrorDomain, code: 3)
        XCTAssertEqual(RadioStateMonitor.downReason(for: att, cancelledLocally: false), .error(3))
    }
}
