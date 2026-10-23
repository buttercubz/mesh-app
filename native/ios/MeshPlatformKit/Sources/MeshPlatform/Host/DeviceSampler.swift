import Foundation
import UIKit

/// Thermal state; raw values match mesh-api `ThermalState`.
public enum ThermalLevel: String, Sendable, Equatable {
    case nominal
    case fair
    case serious
    case critical

    public init(_ s: ProcessInfo.ThermalState) {
        switch s {
        case .nominal: self = .nominal
        case .fair: self = .fair
        case .serious: self = .serious
        case .critical: self = .critical
        @unknown default: self = .critical
        }
    }
}

/// One device sample (Lab platform samples at 1 Hz, §12.1; `PlatformSample`, §3.2).
public struct DeviceSample: Sendable, Equatable {
    /// 0...1, nil when unknown (simulator).
    public var battery: Double?
    public var charging: Bool
    public var thermal: ThermalLevel
    public var lowPowerMode: Bool
    /// `!isProtectedDataAvailable`.
    public var locked: Bool

    public init(battery: Double?, charging: Bool, thermal: ThermalLevel, lowPowerMode: Bool, locked: Bool) {
        self.battery = battery
        self.charging = charging
        self.thermal = thermal
        self.lowPowerMode = lowPowerMode
        self.locked = locked
    }
}

/// Raw device readings behind a protocol (tests use a fake).
@MainActor
public protocol DeviceInfoPort: AnyObject {
    var batteryLevel: Float { get }
    var batteryState: UIDevice.BatteryState { get }
    var thermalState: ProcessInfo.ThermalState { get }
    var isLowPowerModeEnabled: Bool { get }
    var isProtectedDataAvailable: Bool { get }
    func enableBatteryMonitoring()
}

@MainActor
public final class SystemDeviceInfo: DeviceInfoPort {
    public init() {}
    public var batteryLevel: Float { UIDevice.current.batteryLevel }
    public var batteryState: UIDevice.BatteryState { UIDevice.current.batteryState }
    public var thermalState: ProcessInfo.ThermalState { ProcessInfo.processInfo.thermalState }
    public var isLowPowerModeEnabled: Bool { ProcessInfo.processInfo.isLowPowerModeEnabled }
    public var isProtectedDataAvailable: Bool { UIApplication.shared.isProtectedDataAvailable }
    public func enableBatteryMonitoring() { UIDevice.current.isBatteryMonitoringEnabled = true }
}

/// Samples battery, charging, thermal state, Low Power Mode and protected data.
@MainActor
public final class DeviceSampler {
    private let port: any DeviceInfoPort

    public init(port: any DeviceInfoPort = SystemDeviceInfo()) {
        self.port = port
        port.enableBatteryMonitoring()
    }

    public func sample() -> DeviceSample {
        let level = port.batteryLevel
        let state = port.batteryState
        return DeviceSample(
            battery: level < 0 ? nil : Double(min(1, max(0, level))),
            charging: state == .charging || state == .full,
            thermal: ThermalLevel(port.thermalState),
            lowPowerMode: port.isLowPowerModeEnabled,
            locked: !port.isProtectedDataAvailable
        )
    }
}
