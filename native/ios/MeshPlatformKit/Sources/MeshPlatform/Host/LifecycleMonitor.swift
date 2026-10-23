import Foundation
import UIKit

/// App visibility as the core and the Lab samples see it.
public enum AppVisibility: String, Sendable, Equatable {
    case foreground
    case inactive
    case background
}

/// Lifecycle facts reported to the engine host (`PlatformSample` app visibility and device
/// lock, §3.2; Lab platform samples, §12.1).
public enum LifecycleEvent: Sendable, Equatable {
    case visibility(AppVisibility)
    /// Protected data became unavailable (device locked) or available (unlocked).
    case deviceLocked(Bool)
}

/// Observes UIApplication lifecycle and protected-data notifications.
@MainActor
public final class LifecycleMonitor {
    public typealias Listener = @MainActor (LifecycleEvent) -> Void

    private let center: NotificationCenter
    private var observers: [any NSObjectProtocol] = []
    private var listener: Listener?
    public private(set) var visibility: AppVisibility
    public private(set) var deviceLocked: Bool

    public init(center: NotificationCenter = .default,
                initialVisibility: AppVisibility? = nil, initiallyLocked: Bool? = nil) {
        self.center = center
        visibility = initialVisibility ?? Self.map(UIApplication.shared.applicationState)
        deviceLocked = initiallyLocked ?? !UIApplication.shared.isProtectedDataAvailable
    }

    isolated deinit {
        for o in observers { center.removeObserver(o) }
    }

    static func map(_ s: UIApplication.State) -> AppVisibility {
        switch s {
        case .active: return .foreground
        case .inactive: return .inactive
        case .background: return .background
        @unknown default: return .background
        }
    }

    public func start(_ listener: @escaping Listener) {
        self.listener = listener
        guard observers.isEmpty else { return }
        let pairs: [(Notification.Name, LifecycleEvent)] = [
            (UIApplication.didBecomeActiveNotification, .visibility(.foreground)),
            (UIApplication.willResignActiveNotification, .visibility(.inactive)),
            (UIApplication.didEnterBackgroundNotification, .visibility(.background)),
            (UIApplication.willEnterForegroundNotification, .visibility(.inactive)),
            (UIApplication.protectedDataWillBecomeUnavailableNotification, .deviceLocked(true)),
            (UIApplication.protectedDataDidBecomeAvailableNotification, .deviceLocked(false)),
        ]
        for (name, event) in pairs {
            observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.receive(event) }
            })
        }
    }

    /// Applies one event (also the test entry point). Duplicates are not re-reported.
    public func receive(_ e: LifecycleEvent) {
        switch e {
        case .visibility(let v):
            guard v != visibility else { return }
            visibility = v
        case .deviceLocked(let l):
            guard l != deviceLocked else { return }
            deviceLocked = l
        }
        listener?(e)
    }
}
