import CMeshAudio
import Foundation
import MeshActivityShared
@testable import MeshPlatform
import XCTest

final class FakeToken: BackgroundActivityToken {
    let invalidations = Recorder<Int>()
    func invalidate() { invalidations.add(1) }
}

@MainActor
final class FakeLocationAnchor: LocationAnchorPort {
    var tokens: [FakeToken] = []
    var continuation: AsyncStream<LocationUpdateItem>.Continuation?
    func beginBackgroundActivity() -> any BackgroundActivityToken {
        let t = FakeToken()
        tokens.append(t)
        return t
    }
    func liveUpdates() -> AsyncStream<LocationUpdateItem> {
        AsyncStream { continuation = $0 }
    }
}

@MainActor
final class KeepAliveLocationTests: XCTestCase {
    func testStartForwardsFixesAndStopInvalidates() async {
        let port = FakeLocationAnchor()
        let k = KeepAliveLocation(port: port)
        var seen: [LocationUpdateItem] = []
        let got = expectation(description: "three items")
        k.start { item in
            seen.append(item)
            if seen.count == 3 { got.fulfill() }
        }
        k.start { _ in XCTFail("second start is a no-op") }
        XCTAssertTrue(k.isActive)
        XCTAssertEqual(port.tokens.count, 1, "one CLBackgroundActivitySession")
        let fix = LocationFix(latitude: 18.4861, longitude: -69.9312, speed: 12, accuracy: 4.8,
                              timestamp: Date(timeIntervalSince1970: 1))
        port.continuation?.yield(.idle)
        port.continuation?.yield(.fix(fix))
        port.continuation?.yield(.denied)
        await fulfillment(of: [got], timeout: 2)
        XCTAssertEqual(seen, [.idle, .fix(fix), .denied])
        XCTAssertEqual(k.lastFix, fix)
        XCTAssertTrue(k.denied)
        k.stop()
        XCTAssertFalse(k.isActive)
        XCTAssertEqual(port.tokens[0].invalidations.values.count, 1)
    }
}

final class FakeNotificationPort: LocalNotificationPort {
    let added = Recorder<LocalNotificationSpec>()
    let removed = Recorder<[String]>()
    func add(_ spec: LocalNotificationSpec) async throws { added.add(spec) }
    func remove(ids: [String]) { removed.add(ids) }
}

final class ResumeNotifierTests: XCTestCase {
    func testTimeSensitiveResumeNoticeAndClear() async throws {
        let port = FakeNotificationPort()
        let n = ResumeNotifier(port: port)
        try await n.show(title: "Micrófono en pausa", body: "Toca para reanudar el intercomunicador")
        let spec = try XCTUnwrap(port.added.values.first)
        XCTAssertEqual(spec.id, ResumeNotifier.notificationId)
        XCTAssertTrue(spec.timeSensitive)
        XCTAssertEqual(spec.title, "Micrófono en pausa", "text comes from the caller's i18n")
        n.clear()
        XCTAssertEqual(port.removed.values, [[ResumeNotifier.notificationId]])
    }
}

final class LiveActivityRtActionsTests: XCTestCase {
    override func setUp() {
        super.setUp()
        _ = cmesh_clear_sink(nil)
        cmesh_set_flags(0)
    }

    override func tearDown() {
        LiveActivityActionHandler.register(nil)
        cmesh_set_flags(0)
        super.tearDown()
    }

    func testLatchTalkIntentTogglesBit3AndMuteTogglesBit1() async throws {
        let resumed = Recorder<Int>()
        LiveActivityRtActions.register { resumed.add(1) }
        _ = try await LatchTalkIntent().perform()
        XCTAssertEqual(cmesh_flags(), CMESH_FLAG_PTT_LATCH_TOGGLE)
        _ = try await LatchTalkIntent().perform()
        XCTAssertEqual(cmesh_flags(), 0, "each tap is one edge (toggle)")
        _ = try await ToggleMuteIntent().perform()
        XCTAssertEqual(cmesh_flags(), CMESH_FLAG_HARD_MUTE)
        _ = try await ResumeMicIntent().perform()
        XCTAssertEqual(resumed.values, [1])
        XCTAssertEqual(cmesh_flags(), CMESH_FLAG_HARD_MUTE, "resume writes no flag")
    }

    func testPttFastPathKeepsOtherBits() {
        cmesh_set_flags(CMESH_FLAG_HARD_MUTE)
        XCTAssertEqual(LiveActivityRtActions.setPttDown(true), CMESH_FLAG_HARD_MUTE | CMESH_FLAG_PTT_DOWN)
        XCTAssertEqual(LiveActivityRtActions.setPttDown(false), CMESH_FLAG_HARD_MUTE)
    }
}
