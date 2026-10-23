import CoreLocation
import Foundation

/// One location fix (Lab GPS, distance list; DESIGN §1.1, §12.1).
public struct LocationFix: Sendable, Equatable {
    public var latitude: Double
    public var longitude: Double
    /// m/s, nil when invalid.
    public var speed: Double?
    /// Horizontal accuracy in metres.
    public var accuracy: Double
    public var timestamp: Date

    public init(latitude: Double, longitude: Double, speed: Double?, accuracy: Double, timestamp: Date) {
        self.latitude = latitude
        self.longitude = longitude
        self.speed = speed
        self.accuracy = accuracy
        self.timestamp = timestamp
    }

    public init(_ l: CLLocation) {
        self.init(latitude: l.coordinate.latitude, longitude: l.coordinate.longitude,
                  speed: l.speed >= 0 ? l.speed : nil, accuracy: l.horizontalAccuracy,
                  timestamp: l.timestamp)
    }
}

/// One item of the live-updates stream.
public enum LocationUpdateItem: Sendable, Equatable {
    case fix(LocationFix)
    /// Location permission denied or restricted (the anchor cannot hold).
    case denied
    /// No fix yet / stationary.
    case idle
}

/// Something that ends a background activity session.
public protocol BackgroundActivityToken: AnyObject, Sendable {
    func invalidate()
}

/// CoreLocation behind a protocol (tests use a fake).
@MainActor
public protocol LocationAnchorPort: AnyObject {
    /// `CLBackgroundActivitySession()`; must be created in the foreground.
    func beginBackgroundActivity() -> any BackgroundActivityToken
    /// `CLLocationUpdate.liveUpdates(.otherNavigation)`.
    func liveUpdates() -> AsyncStream<LocationUpdateItem>
}

/// `CLBackgroundActivitySession` as a token.
final class BackgroundActivitySessionToken: BackgroundActivityToken, @unchecked Sendable {
    // The session is only invalidated once; CoreLocation objects are thread-safe here.
    private let session = CLBackgroundActivitySession()
    func invalidate() { session.invalidate() }
}

@MainActor
public final class SystemLocationAnchor: LocationAnchorPort {
    public init() {}

    public func beginBackgroundActivity() -> any BackgroundActivityToken {
        BackgroundActivitySessionToken()
    }

    public func liveUpdates() -> AsyncStream<LocationUpdateItem> {
        AsyncStream { cont in
            let task = Task {
                do {
                    for try await u in CLLocationUpdate.liveUpdates(.otherNavigation) {
                        if u.authorizationDenied || u.authorizationDeniedGlobally || u.authorizationRestricted {
                            cont.yield(.denied)
                        } else if let l = u.location {
                            cont.yield(.fix(LocationFix(l)))
                        } else {
                            cont.yield(.idle)
                        }
                    }
                } catch {
                    cont.yield(.denied)
                }
                cont.finish()
            }
            cont.onTermination = { _ in task.cancel() }
        }
    }
}

/// The `location` keep-alive anchor (DESIGN §10.1.3, D8): a `CLBackgroundActivitySession`
/// plus `CLLocationUpdate.liveUpdates(.otherNavigation)`, started inside the Go-live tap
/// only when "Share my distance" is on (or for Lab `audioLocation`). It keeps relaying
/// alive at ladder L3 even if audio dies; the fixes feed the visible distance list.
@MainActor
public final class KeepAliveLocation {
    public typealias Listener = @MainActor (LocationUpdateItem) -> Void

    private let port: any LocationAnchorPort
    private var token: (any BackgroundActivityToken)?
    private var task: Task<Void, Never>?
    public private(set) var lastFix: LocationFix?
    public private(set) var denied = false

    public init(port: any LocationAnchorPort = SystemLocationAnchor()) {
        self.port = port
    }

    public var isActive: Bool { token != nil }

    /// Starts the anchor (idempotent). Must run in the foreground (gesture-bound).
    public func start(_ listener: @escaping Listener) {
        guard token == nil else { return }
        token = port.beginBackgroundActivity()
        denied = false
        let stream = port.liveUpdates()
        task = Task { [weak self] in
            for await item in stream {
                guard let self else { return }
                self.receive(item)
                listener(item)
            }
        }
    }

    func receive(_ item: LocationUpdateItem) {
        switch item {
        case .fix(let f): lastFix = f
        case .denied: denied = true
        case .idle: break
        }
    }

    public func stop() {
        task?.cancel()
        task = nil
        token?.invalidate()
        token = nil
    }
}
