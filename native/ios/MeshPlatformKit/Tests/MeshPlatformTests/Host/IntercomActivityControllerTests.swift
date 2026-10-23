import Foundation
import MeshActivityShared
@testable import MeshPlatform
import XCTest

@MainActor
final class FakeActivityPort: LiveActivityPort {
    enum Call: Equatable {
        case request(String, IntercomAttributes.ContentState)
        case update(String, IntercomAttributes.ContentState)
        case end(String)
    }

    var activitiesEnabled = true
    var calls: [Call] = []
    private var n = 0

    func request(_ attributes: IntercomAttributes, _ state: IntercomAttributes.ContentState) throws -> String {
        n += 1
        let id = "a\(n)"
        calls.append(.request(id, state))
        return id
    }
    func update(id: String, _ state: IntercomAttributes.ContentState) async { calls.append(.update(id, state)) }
    func end(id: String, _ state: IntercomAttributes.ContentState?) async { calls.append(.end(id)) }
}

/// Live Activity rate limit (≤ 1 update/s, on change) and the 8 h renewal (7.5 h on
/// foreground), with an injected clock (WP-14 acceptance).
@MainActor
final class IntercomActivityControllerTests: XCTestCase {
    private let attrs = IntercomAttributes(kind: .ride, title: "Sunday ride")

    private func st(_ inRange: Int, latched: Bool = false) -> IntercomAttributes.ContentState {
        IntercomAttributes.ContentState(live: true, pttLatched: latched, inRange: inRange, total: 5)
    }

    func testUpdatesAreCoalescedToOnePerSecondAndOnChange() async throws {
        let clock = ManualClock()
        let port = FakeActivityPort()
        let c = IntercomActivityController(port: port, autoFlush: false, clock: clock.fn)
        try await c.start(attrs, st(1))
        XCTAssertEqual(port.calls, [.request("a1", st(1))])
        await c.update(st(1))
        XCTAssertEqual(port.calls.count, 1, "unchanged state is not sent")
        clock.advance(0.3)
        await c.update(st(2))
        await c.update(st(3))
        XCTAssertEqual(port.calls.count, 1, "within 1 s: coalesced")
        XCTAssertEqual(c.nextFlush, clock.now + 0.7)
        clock.advance(0.6)
        await c.tick()
        XCTAssertEqual(port.calls.count, 1)
        clock.advance(0.1)
        await c.tick()
        XCTAssertEqual(port.calls.last, .update("a1", st(3)), "only the latest state goes out")
        XCTAssertNil(c.nextFlush)
        clock.advance(5)
        await c.update(st(4, latched: true))
        XCTAssertEqual(port.calls.last, .update("a1", st(4, latched: true)), "≥ 1 s since the last: immediate")
    }

    func testRenewalAfterSevenAndAHalfHoursOnForeground() async throws {
        let clock = ManualClock()
        let port = FakeActivityPort()
        let c = IntercomActivityController(port: port, autoFlush: false, clock: clock.fn)
        try await c.start(attrs, st(2))
        clock.advance(7 * 3600)
        await c.appWillEnterForeground()
        XCTAssertEqual(port.calls.count, 1, "7 h: no renewal")
        clock.advance(0.5 * 3600)
        await c.update(st(4))
        await c.appWillEnterForeground()
        XCTAssertEqual(Array(port.calls.suffix(3)), [.update("a1", st(4)), .end("a1"), .request("a2", st(4))])
        XCTAssertEqual(c.renewals, 1)
        XCTAssertEqual(c.age, 0)
        clock.advance(3600)
        await c.appWillEnterForeground()
        XCTAssertEqual(c.renewals, 1, "the renewed activity is young again")
    }

    func testRenewalCarriesThePendingState() async throws {
        let clock = ManualClock()
        let port = FakeActivityPort()
        let c = IntercomActivityController(port: port, autoFlush: false, clock: clock.fn)
        try await c.start(attrs, st(1))
        clock.advance(7.5 * 3600)
        await c.update(st(2))
        clock.advance(0.1)
        await c.update(st(3)) // coalesced, not yet sent
        await c.appWillEnterForeground()
        XCTAssertEqual(port.calls.last, .request("a2", st(3)))
    }

    func testDisabledActivitiesAndEnd() async throws {
        let port = FakeActivityPort()
        port.activitiesEnabled = false
        let c = IntercomActivityController(port: port, autoFlush: false, clock: ManualClock().fn)
        try await c.start(attrs, st(1))
        XCTAssertFalse(c.isActive)
        await c.update(st(2))
        XCTAssertTrue(port.calls.isEmpty)
        port.activitiesEnabled = true
        try await c.start(attrs, st(1))
        await c.end()
        XCTAssertEqual(port.calls, [.request("a1", st(1)), .end("a1")])
        XCTAssertFalse(c.isActive)
    }

    func testRestartWithNewAttributesEndsTheOldActivity() async throws {
        let port = FakeActivityPort()
        let c = IntercomActivityController(port: port, autoFlush: false, clock: ManualClock().fn)
        try await c.start(IntercomAttributes(kind: .lab, title: "T3"), st(0))
        try await c.start(attrs, st(1))
        XCTAssertEqual(port.calls, [.request("a1", st(0)), .end("a1"), .request("a2", st(1))])
    }

    func testAutoFlushDeliversTheCoalescedState() async throws {
        let port = FakeActivityPort()
        let c = IntercomActivityController(port: port, autoFlush: true)
        try await c.start(attrs, st(1))
        await c.update(st(2))
        try await Task.sleep(for: .milliseconds(1300))
        XCTAssertEqual(port.calls.last, .update("a1", st(2)))
    }
}
