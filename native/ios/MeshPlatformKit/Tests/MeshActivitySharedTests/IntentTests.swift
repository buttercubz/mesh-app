import AppIntents
import Foundation
import MeshActivityShared
import Synchronization
import XCTest

final class Recorder<T: Sendable>: Sendable {
    private let m = Mutex<[T]>([])
    func add(_ v: T) { m.withLock { $0.append(v) } }
    var values: [T] { m.withLock { $0 } }
    func reset() { m.withLock { $0 = [] } }
}

final class IntentTests: XCTestCase {
    private let seen = Recorder<LiveActivityAction>()

    override func setUp() {
        super.setUp()
        seen.reset()
        LiveActivityActionHandler.register { [seen] a in seen.add(a) }
    }

    override func tearDown() {
        LiveActivityActionHandler.register(nil)
        super.tearDown()
    }

    func testLatchTalkIntentCallsTheRegisteredHandler() async throws {
        _ = try await LatchTalkIntent().perform()
        XCTAssertEqual(seen.values, [.latchTalk])
    }

    func testToggleMuteAndResumeMicIntents() async throws {
        _ = try await ToggleMuteIntent().perform()
        _ = try await ResumeMicIntent().perform()
        XCTAssertEqual(seen.values, [.toggleMute, .resumeMic])
    }

    func testIntentsFailWithoutAHandler() async {
        LiveActivityActionHandler.register(nil)
        XCTAssertFalse(LiveActivityActionHandler.isRegistered)
        do {
            _ = try await LatchTalkIntent().perform()
            XCTFail("expected LiveActivityHandlerMissing")
        } catch let e as LiveActivityHandlerMissing {
            XCTAssertEqual(e.action, .latchTalk)
        } catch {
            XCTFail("unexpected \(error)")
        }
    }

    func testIntentModesAndDiscoverability() {
        XCTAssertFalse(LatchTalkIntent.isDiscoverable)
        XCTAssertFalse(ToggleMuteIntent.isDiscoverable)
        XCTAssertEqual(LatchTalkIntent.supportedModes, .background, "runs in place on the lock screen")
        XCTAssertEqual(ToggleMuteIntent.supportedModes, .background)
        XCTAssertEqual(ResumeMicIntent.supportedModes, .foreground(.immediate), "capture needs the foreground")
    }

    func testReplacingTheHandler() {
        let second = Recorder<LiveActivityAction>()
        LiveActivityActionHandler.register { a in second.add(a) }
        XCTAssertTrue(LiveActivityActionHandler.handle(.toggleMute))
        XCTAssertEqual(second.values, [.toggleMute])
        XCTAssertTrue(seen.values.isEmpty)
    }
}
