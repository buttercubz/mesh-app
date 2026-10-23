import Foundation
@testable import MeshPlatform
import UIKit
import XCTest

@MainActor
final class FakeIdleTimer: IdleTimerPort {
    var isIdleTimerDisabled = false
    var writes: [Bool] = []
}

@MainActor
final class IdleTimerControllerTests: XCTestCase {
    func testKeepScreenOnAndResetOnBackground() {
        let center = NotificationCenter()
        let port = FakeIdleTimer()
        let c = IdleTimerController(port: port, center: center)
        c.setKeepScreenOn(true)
        XCTAssertTrue(port.isIdleTimerDisabled)
        XCTAssertTrue(c.isKeepingScreenOn)
        center.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
        XCTAssertFalse(port.isIdleTimerDisabled, "always reset when the app backgrounds")
        XCTAssertFalse(c.requested)
        c.setKeepScreenOn(true)
        c.setKeepScreenOn(false)
        XCTAssertFalse(port.isIdleTimerDisabled)
    }

    func testNoObserverAfterDeinit() {
        let center = NotificationCenter()
        let port = FakeIdleTimer()
        var c: IdleTimerController? = IdleTimerController(port: port, center: center)
        c?.setKeepScreenOn(true)
        c = nil
        center.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
        XCTAssertTrue(port.isIdleTimerDisabled, "a released controller no longer reacts")
    }
}

@MainActor
final class LifecycleMonitorTests: XCTestCase {
    func testReportsDistinctVisibilityAndLockChanges() {
        let center = NotificationCenter()
        let m = LifecycleMonitor(center: center, initialVisibility: .foreground, initiallyLocked: false)
        var seen: [LifecycleEvent] = []
        m.start { seen.append($0) }
        center.post(name: UIApplication.willResignActiveNotification, object: nil)
        center.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
        center.post(name: UIApplication.protectedDataWillBecomeUnavailableNotification, object: nil)
        center.post(name: UIApplication.protectedDataWillBecomeUnavailableNotification, object: nil)
        center.post(name: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil)
        center.post(name: UIApplication.didBecomeActiveNotification, object: nil)
        XCTAssertEqual(seen, [.visibility(.inactive), .visibility(.background), .deviceLocked(true),
                              .deviceLocked(false), .visibility(.foreground)])
        XCTAssertEqual(m.visibility, .foreground)
        XCTAssertFalse(m.deviceLocked)
    }
}

@MainActor
final class FakeDeviceInfo: DeviceInfoPort {
    var batteryLevel: Float = 0.81
    var batteryState: UIDevice.BatteryState = .unplugged
    var thermalState: ProcessInfo.ThermalState = .nominal
    var isLowPowerModeEnabled = false
    var isProtectedDataAvailable = true
    var monitoring = false
    func enableBatteryMonitoring() { monitoring = true }
}

@MainActor
final class DeviceSamplerTests: XCTestCase {
    func testSampleMapping() {
        let p = FakeDeviceInfo()
        let s = DeviceSampler(port: p)
        XCTAssertTrue(p.monitoring)
        XCTAssertEqual(s.sample(), DeviceSample(battery: Double(Float(0.81)), charging: false, thermal: .nominal,
                                                lowPowerMode: false, locked: false))
        p.batteryLevel = -1 // simulator / unknown
        p.batteryState = .full
        p.thermalState = .serious
        p.isLowPowerModeEnabled = true
        p.isProtectedDataAvailable = false
        let x = s.sample()
        XCTAssertNil(x.battery)
        XCTAssertTrue(x.charging, "full counts as on charger")
        XCTAssertEqual(x.thermal, .serious)
        XCTAssertTrue(x.lowPowerMode)
        XCTAssertTrue(x.locked)
        XCTAssertEqual(ThermalLevel(.critical).rawValue, "critical")
    }

    func testRealSamplerRunsInTheSimulator() {
        let s = DeviceSampler().sample()
        XCTAssertEqual(s.thermal, ThermalLevel(ProcessInfo.processInfo.thermalState))
    }
}
