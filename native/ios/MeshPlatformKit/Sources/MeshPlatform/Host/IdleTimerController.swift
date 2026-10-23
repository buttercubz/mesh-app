import Foundation
import UIKit

/// The idle-timer switch behind a protocol (tests use a fake).
@MainActor
public protocol IdleTimerPort: AnyObject {
    var isIdleTimerDisabled: Bool { get set }
}

/// `UIApplication.shared.isIdleTimerDisabled`.
@MainActor
public final class SystemIdleTimer: IdleTimerPort {
    public init() {}

    public var isIdleTimerDisabled: Bool {
        get { UIApplication.shared.isIdleTimerDisabled }
        set { UIApplication.shared.isIdleTimerDisabled = newValue }
    }
}

/// Keeps the screen on for the lobby, the PTT ride screen and the car "keep screen on
/// while mounted" option (DESIGN §10.1.3), through the plugin's `setKeepScreenOn`. The
/// switch is always reset when the app backgrounds, so a forgotten `true` never keeps a
/// locked-in-pocket phone awake.
@MainActor
public final class IdleTimerController {
    private let port: any IdleTimerPort
    private let center: NotificationCenter
    private var observer: (any NSObjectProtocol)?
    public private(set) var requested = false

    public init(port: any IdleTimerPort = SystemIdleTimer(), center: NotificationCenter = .default) {
        self.port = port
        self.center = center
        observer = center.addObserver(forName: UIApplication.didEnterBackgroundNotification,
                                      object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.appDidEnterBackground() }
        }
    }

    isolated deinit {
        if let observer { center.removeObserver(observer) }
    }

    /// `setKeepScreenOn({on})`.
    public func setKeepScreenOn(_ on: Bool) {
        requested = on
        port.isIdleTimerDisabled = on
    }

    /// Background transition: the screen may lock again; the screen that wants it on must
    /// ask again when it is visible.
    public func appDidEnterBackground() {
        requested = false
        port.isIdleTimerDisabled = false
    }

    public var isKeepingScreenOn: Bool { port.isIdleTimerDisabled }
}
