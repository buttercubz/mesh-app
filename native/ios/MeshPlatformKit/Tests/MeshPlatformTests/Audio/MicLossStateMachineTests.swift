import Foundation
import MeshActivityShared
@testable import MeshPlatform
import XCTest

/// Every transition of DESIGN §10.1.4 (WP-14 acceptance).
final class MicLossStateMachineTests: XCTestCase {
    typealias M = MicLossStateMachine

    private func running() -> M {
        var m = M()
        XCTAssertEqual(m.handle(.started, now: 0), [.report(.running)])
        XCTAssertEqual(m.state, .running)
        XCTAssertEqual(m.ladder, .l0)
        return m
    }

    private let lost: [M.Action] = [.report(.captureLost), .startOutputOnly, .notifyCaptureLost, .cue(.micPaused)]
    private let back: [M.Action] = [.report(.restored), .report(.running), .cue(.micBack)]

    func testInterruptionEndedWithShouldResumeRestartsToRunning() {
        var m = running()
        XCTAssertEqual(m.handle(.interruptionBegan(.otherSession), now: 10), [.report(.interrupted(.otherSession))])
        XCTAssertEqual(m.state, .interrupted(since: 10))
        XCTAssertEqual(m.ladder, .l1)
        XCTAssertEqual(m.handle(.interruptionEnded(shouldResume: true), now: 40), [.report(.restarting), .restartUnit])
        XCTAssertEqual(m.state, .restarting(.interruptionEnded))
        XCTAssertEqual(m.handle(.restartResult(ok: true), now: 40), back)
        XCTAssertEqual(m.state, .running)
        XCTAssertEqual(m.ladder, .l0)
    }

    func testRestartFailureAfterInterruptionGoesToCaptureLost() {
        var m = running()
        _ = m.handle(.interruptionBegan(.otherSession), now: 0)
        _ = m.handle(.interruptionEnded(shouldResume: true), now: 1)
        XCTAssertEqual(m.handle(.restartResult(ok: false), now: 1), lost, "'!rec' → CAPTURE_LOST")
        XCTAssertEqual(m.state, .captureLost)
        XCTAssertEqual(m.ladder, .l2)
    }

    func testEndedWithoutShouldResumeKeepsWaiting() {
        var m = running()
        _ = m.handle(.interruptionBegan(.otherSession), now: 0)
        XCTAssertEqual(m.handle(.interruptionEnded(shouldResume: false), now: 1), [])
        XCTAssertEqual(m.state, .interrupted(since: 0))
    }

    func testRetryScheduleFiveSecondsForTwoMinutesThenThirty() {
        var m = running()
        _ = m.handle(.interruptionBegan(.otherSession), now: 100)
        XCTAssertEqual(m.nextDeadline(), 105)
        XCTAssertEqual(m.handle(.tick, now: 104.9), [], "no 'ended' yet, < 5 s")
        var attempts: [TimeInterval] = []
        var t: TimeInterval = 100
        while t < 100 + 300 {
            t += 0.5
            let a = m.handle(.tick, now: t)
            if a == [.report(.restarting), .restartUnit] {
                attempts.append(t - 100)
                // The call is still holding the session: the retry fails and waits again.
                XCTAssertEqual(m.handle(.restartResult(ok: false), now: t), [])
                XCTAssertEqual(m.state, .interrupted(since: 100))
            }
        }
        XCTAssertEqual(Array(attempts.prefix(3)), [5, 10, 15])
        let fast = attempts.filter { $0 <= 120 }
        XCTAssertEqual(fast.count, 24, "every 5 s for 2 min")
        let slow = attempts.filter { $0 > 120 }
        XCTAssertEqual(slow.first, 150, "then every 30 s")
        XCTAssertEqual(zip(slow, slow.dropFirst()).map { $1 - $0 }, Array(repeating: 30, count: slow.count - 1))
    }

    func testSuccessfulRetryReturnsToRunning() {
        var m = running()
        _ = m.handle(.interruptionBegan(.appWasSuspended), now: 0)
        XCTAssertEqual(m.handle(.tick, now: 5), [.report(.restarting), .restartUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: true), now: 5), back)
        XCTAssertNil(m.nextDeadline())
    }

    func testRouteChangeWithUnchangedFormatDoesNothing() {
        var m = running()
        XCTAssertEqual(m.handle(.routeChanged(formatChanged: false, unitRunning: true), now: 1), [])
        XCTAssertEqual(m.state, .running)
    }

    func testRouteChangeWithNewFormatOrStoppedUnitReconfigures() {
        var m = running()
        XCTAssertEqual(m.handle(.routeChanged(formatChanged: true, unitRunning: true), now: 1),
                       [.report(.restarting), .reconfigureUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: true), now: 1), back)
        XCTAssertEqual(m.handle(.routeChanged(formatChanged: false, unitRunning: false), now: 2),
                       [.report(.restarting), .reconfigureUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: false), now: 2), lost)
    }

    func testWatchdogStallRestartsAndFailureIsCaptureLost() {
        var m = running()
        XCTAssertEqual(m.handle(.watchdogStall, now: 1), [.report(.restarting), .restartUnit])
        XCTAssertEqual(m.state, .restarting(.watchdog))
        XCTAssertEqual(m.handle(.restartResult(ok: false), now: 1), lost)
    }

    func testMediaServicesResetRebuildsFromAnyLiveState() {
        var m = running()
        XCTAssertEqual(m.handle(.mediaServicesReset, now: 1), [.report(.restarting), .rebuildSessionAndUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: true), now: 1), [.clearCaptureLostNotice] + back)
        _ = m.handle(.watchdogStall, now: 2)
        _ = m.handle(.restartResult(ok: false), now: 2)
        XCTAssertEqual(m.state, .captureLost)
        XCTAssertEqual(m.handle(.mediaServicesReset, now: 3), [.report(.restarting), .rebuildSessionAndUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: false), now: 3), lost)
    }

    func testOutputOnlyFailureIsRelayOnlyL3() {
        var m = running()
        _ = m.handle(.watchdogStall, now: 1)
        _ = m.handle(.restartResult(ok: false), now: 1)
        XCTAssertEqual(m.handle(.outputOnlyResult(ok: true), now: 1), [])
        XCTAssertEqual(m.state, .captureLost)
        XCTAssertEqual(m.handle(.outputOnlyResult(ok: false), now: 2), [.report(.renderLost)])
        XCTAssertEqual(m.state, .renderLost)
        XCTAssertEqual(m.ladder, .l3)
    }

    func testResumeFromCaptureLostAndRenderLost() {
        var m = running()
        _ = m.handle(.watchdogStall, now: 1)
        _ = m.handle(.restartResult(ok: false), now: 1)
        XCTAssertEqual(m.handle(.resumeRequested, now: 2), [.stopOutputOnly, .report(.restarting), .restartUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: true), now: 2), [.clearCaptureLostNotice] + back)
        _ = m.handle(.watchdogStall, now: 3)
        _ = m.handle(.restartResult(ok: false), now: 3)
        _ = m.handle(.outputOnlyResult(ok: false), now: 3)
        XCTAssertEqual(m.handle(.resumeRequested, now: 4), [.stopOutputOnly, .report(.restarting), .restartUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: false), now: 4), lost, "still no mic: L2 again")
    }

    func testResumeWhileInterruptedTriesImmediately() {
        var m = running()
        _ = m.handle(.interruptionBegan(.otherSession), now: 0)
        XCTAssertEqual(m.handle(.resumeRequested, now: 1), [.report(.restarting), .restartUnit])
        XCTAssertEqual(m.handle(.restartResult(ok: false), now: 1), lost)
    }

    func testInterruptionDuringRestartAndStop() {
        var m = running()
        _ = m.handle(.watchdogStall, now: 1)
        XCTAssertEqual(m.handle(.interruptionBegan(.otherSession), now: 2), [.report(.interrupted(.otherSession))])
        XCTAssertEqual(m.state, .interrupted(since: 2))
        XCTAssertEqual(m.handle(.stopped, now: 3), [])
        XCTAssertEqual(m.state, .idle)
        XCTAssertEqual(m.ladder, .l0)
    }

    func testIrrelevantEventsAreIgnored() {
        var m = M()
        XCTAssertEqual(m.handle(.watchdogStall, now: 0), [], "idle ignores everything but started")
        XCTAssertEqual(m.handle(.resumeRequested, now: 0), [])
        m = running()
        XCTAssertEqual(m.handle(.interruptionEnded(shouldResume: true), now: 1), [])
        XCTAssertEqual(m.handle(.outputOnlyResult(ok: false), now: 1), [])
        XCTAssertEqual(m.handle(.restartResult(ok: true), now: 1), [])
        XCTAssertEqual(m.handle(.started, now: 1), [])
        _ = m.handle(.interruptionBegan(.otherSession), now: 1)
        XCTAssertEqual(m.handle(.watchdogStall, now: 2), [], "no watchdog while interrupted")
        XCTAssertEqual(m.handle(.routeChanged(formatChanged: true, unitRunning: false), now: 2), [])
    }
}

final class MuteWatchdogTests: XCTestCase {
    func testFiresOnceAfterStallAndRearmsOnProgress() {
        var w = MuteWatchdog()
        XCTAssertFalse(w.check(count: 0, now: 0), "disarmed")
        w.arm(count: 10, now: 0)
        XCTAssertFalse(w.check(count: 20, now: 1))
        XCTAssertFalse(w.check(count: 20, now: 2.5), "exactly 1.5 s is not yet a stall")
        XCTAssertTrue(w.check(count: 20, now: 2.6))
        XCTAssertFalse(w.check(count: 20, now: 5), "reported once")
        XCTAssertFalse(w.check(count: 21, now: 6))
        XCTAssertTrue(w.check(count: 21, now: 7.6))
        w.disarm()
        XCTAssertFalse(w.isArmed)
        XCTAssertFalse(w.check(count: 21, now: 100))
    }
}
