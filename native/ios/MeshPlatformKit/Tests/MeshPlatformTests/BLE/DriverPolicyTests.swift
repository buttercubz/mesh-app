import CoreBluetooth
import Foundation
@testable import MeshPlatform
import XCTest

final class ScanPolicyTests: XCTestCase {
    private func decide(_ mode: DiscoveryMode, fg: Bool = true, links: Int = 0, armed: Int = 0,
                        hint: ScanHint = .auto) -> ScanPolicy {
        ScanPolicy.decide(mode: mode, foreground: fg, bleLinks: links, armedReconnects: armed, hint: hint)
    }

    func testOffStopsScanning() {
        XCTAssertEqual(decide(.off), ScanPolicy(scan: false, allowDuplicates: false))
    }

    func testFormingWithoutLinksUsesDuplicatesInForegroundOnly() {
        XCTAssertEqual(decide(.forming), ScanPolicy(scan: true, allowDuplicates: true))
        XCTAssertEqual(decide(.forming, fg: false), ScanPolicy(scan: true, allowDuplicates: false))
    }

    func testFormingWithLinksIsFilteredUnlessHunting() {
        // Field test 1: forming with a live link must not run a 100 % duty scan.
        XCTAssertEqual(decide(.forming, links: 1), ScanPolicy(scan: true, allowDuplicates: false))
        XCTAssertEqual(decide(.forming, links: 2, armed: 1), ScanPolicy(scan: true, allowDuplicates: true))
        XCTAssertEqual(decide(.forming, links: 2, hint: .hunt), ScanPolicy(scan: true, allowDuplicates: true))
        XCTAssertEqual(decide(.forming, links: 0, hint: .quiet), ScanPolicy(scan: true, allowDuplicates: false))
    }

    func testStableNeverUsesDuplicates() {
        XCTAssertEqual(decide(.stable, links: 0, armed: 3, hint: .hunt),
                       ScanPolicy(scan: true, allowDuplicates: false))
    }
}

final class RepublishBackoffTests: XCTestCase {
    func testExponentialCappedAndReset() {
        var b = RepublishBackoff()
        XCTAssertEqual((0..<7).map { _ in b.failed() }, [1, 2, 4, 8, 16, 30, 30])
        b.succeeded()
        XCTAssertEqual(b.failed(), 1)
        let g = b.generation
        b.reset()
        XCTAssertEqual(b.failures, 0)
        XCTAssertNotEqual(b.generation, g)
    }
}

final class L2capListenerStateTests: XCTestCase {
    func testPublishesOnceAndReportsPsmOnlyWhenConfirmed() {
        var s = L2capListenerState()
        XCTAssertEqual(s.wantListener(), .publish)
        XCTAssertEqual(s.wantListener(), .none, "one publish in flight")
        XCTAssertNil(s.psm, "no PSM in HELLO until CoreBluetooth confirms it")
        XCTAssertEqual(s.published(psm: 0x81, failed: false), .none)
        XCTAssertEqual(s.psm, 0x81)
        XCTAssertEqual(s.wantListener(), .none, "a live listener is not re-published")
    }

    func testFailureClearsPsmAndRetriesWithBackoff() {
        var s = L2capListenerState()
        _ = s.wantListener()
        let gen = s.backoff.generation
        XCTAssertEqual(s.published(psm: 0, failed: true), .retry(after: 1, generation: gen))
        XCTAssertNil(s.psm)
        XCTAssertEqual(s.retryFired(generation: gen), .publish)
        XCTAssertEqual(s.published(psm: 0, failed: true), .retry(after: 2, generation: gen))
        XCTAssertEqual(s.retryFired(generation: gen), .publish)
        XCTAssertEqual(s.published(psm: 0x85, failed: false), .none)
        XCTAssertEqual(s.psm, 0x85)
        XCTAssertEqual(s.backoff.failures, 0)
    }

    func testUnexpectedUnpublishRepublishes() {
        var s = L2capListenerState()
        _ = s.wantListener()
        _ = s.published(psm: 0x81, failed: false)
        guard case .retry(1, let gen) = s.unpublished() else { return XCTFail("expected a retry") }
        XCTAssertNil(s.psm, "HELLO must stop advertising a dead PSM")
        XCTAssertEqual(s.retryFired(generation: gen), .publish)
    }

    func testRadioLossMakesPendingRetriesStale() {
        var s = L2capListenerState()
        _ = s.wantListener()
        guard case .retry(_, let gen) = s.published(psm: nil, failed: true) else { return XCTFail() }
        s.radioLost()
        XCTAssertEqual(s.retryFired(generation: gen), .none)
        XCTAssertEqual(s.wantListener(), .publish, "power on publishes afresh")
    }
}

final class OpenCheckTests: XCTestCase {
    func testPreconditions() {
        XCTAssertEqual(OpenCheck.check(via: .gatt, connected: false, hasRx: true, hasTx: true), .notConnected)
        XCTAssertEqual(OpenCheck.check(via: .gatt, connected: true, hasRx: true, hasTx: false),
                       .missingGattCharacteristics)
        XCTAssertEqual(OpenCheck.check(via: .gatt, connected: true, hasRx: true, hasTx: true), .ok)
        XCTAssertEqual(OpenCheck.check(via: .l2cap(psm: 0), connected: true, hasRx: false, hasTx: false), .noPsm)
        XCTAssertEqual(OpenCheck.check(via: .l2cap(psm: 0x81), connected: true, hasRx: false, hasTx: false), .ok)
    }

    func testErrorInfoKeepsDomainAndCode() {
        let e = NSError(domain: CBErrorDomain, code: CBError.Code.connectionTimeout.rawValue)
        XCTAssertEqual(BleErrorInfo(e), BleErrorInfo(domain: CBErrorDomain, code: 6))
        XCTAssertNil(BleErrorInfo(nil))
    }
}
