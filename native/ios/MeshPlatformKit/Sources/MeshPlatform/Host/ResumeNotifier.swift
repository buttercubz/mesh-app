import Foundation
import UserNotifications

/// A local notification, as data (tests use a fake port).
public struct LocalNotificationSpec: Sendable, Equatable {
    public var id: String
    public var title: String
    public var body: String
    public var timeSensitive: Bool
    public var category: String
}

/// UNUserNotificationCenter behind a protocol.
public protocol LocalNotificationPort: AnyObject, Sendable {
    func add(_ spec: LocalNotificationSpec) async throws
    func remove(ids: [String])
}

public final class SystemNotificationPort: LocalNotificationPort {
    public init() {}

    public func add(_ spec: LocalNotificationSpec) async throws {
        let c = UNMutableNotificationContent()
        c.title = spec.title
        c.body = spec.body
        c.sound = .default
        c.categoryIdentifier = spec.category
        c.threadIdentifier = "mesh.intercom"
        c.interruptionLevel = spec.timeSensitive ? .timeSensitive : .active
        let req = UNNotificationRequest(identifier: spec.id, content: c, trigger: nil)
        try await UNUserNotificationCenter.current().add(req)
    }

    public func remove(ids: [String]) {
        let center = UNUserNotificationCenter.current()
        center.removePendingNotificationRequests(withIdentifiers: ids)
        center.removeDeliveredNotifications(withIdentifiers: ids)
    }
}

/// "Tap to resume intercom" (DESIGN §10.1.4 CAPTURE_LOST step 3): a time-sensitive local
/// notification (entitlement `usernotifications.time-sensitive`). Title and body come
/// localised from the UI's i18n catalogs through the engine's `notify` hook; this class
/// never hard-codes user-facing text.
public final class ResumeNotifier: Sendable {
    public static let notificationId = "mesh.resume-intercom"
    public static let category = "MESH_RESUME"

    private let port: any LocalNotificationPort

    public init(port: any LocalNotificationPort = SystemNotificationPort()) {
        self.port = port
    }

    /// Shows (or replaces) the resume notification.
    public func show(title: String, body: String) async throws {
        try await port.add(LocalNotificationSpec(id: Self.notificationId, title: title, body: body,
                                                 timeSensitive: true, category: Self.category))
    }

    /// Removes it once capture is back.
    public func clear() {
        port.remove(ids: [Self.notificationId])
    }

    /// Any other engine notification (`FfiNotification`), e.g. `engine: crashed`.
    public func post(id: String, title: String, body: String, timeSensitive: Bool) async throws {
        try await port.add(LocalNotificationSpec(id: id, title: title, body: body,
                                                 timeSensitive: timeSensitive, category: Self.category))
    }
}
