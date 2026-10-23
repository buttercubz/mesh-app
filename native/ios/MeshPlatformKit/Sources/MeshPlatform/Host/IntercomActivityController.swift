@preconcurrency import ActivityKit
import Foundation
import MeshActivityShared

/// ActivityKit behind a protocol (tests use a fake).
@MainActor
public protocol LiveActivityPort: AnyObject {
    var activitiesEnabled: Bool { get }
    /// Starts an activity and returns its id.
    func request(_ attributes: IntercomAttributes, _ state: IntercomAttributes.ContentState) throws -> String
    func update(id: String, _ state: IntercomAttributes.ContentState) async
    func end(id: String, _ state: IntercomAttributes.ContentState?) async
}

@MainActor
public final class SystemLiveActivityPort: LiveActivityPort {
    private var activities: [String: Activity<IntercomAttributes>] = [:]

    public init() {}

    public var activitiesEnabled: Bool { ActivityAuthorizationInfo().areActivitiesEnabled }

    public func request(_ attributes: IntercomAttributes, _ state: IntercomAttributes.ContentState) throws -> String {
        let a = try Activity.request(attributes: attributes,
                                     content: ActivityContent(state: state, staleDate: nil),
                                     pushType: nil)
        activities[a.id] = a
        return a.id
    }

    public func update(id: String, _ state: IntercomAttributes.ContentState) async {
        await activities[id]?.update(ActivityContent(state: state, staleDate: nil))
    }

    public func end(id: String, _ state: IntercomAttributes.ContentState?) async {
        guard let a = activities.removeValue(forKey: id) else { return }
        await a.end(state.map { ActivityContent(state: $0, staleDate: nil) }, dismissalPolicy: .immediate)
    }
}

/// Owns the intercom Live Activity (DESIGN §9.4, §10.1.3, §10.1.5):
/// - started at Go live, before backgrounding (foreground-class BT privileges on iOS 26);
/// - updated at most once per second and only on change (coalescing the latest state);
/// - the system ends a Live Activity after 8 h, so whenever the app comes to the
///   foreground after 7.5 h the activity is ended and restarted with the current state.
@MainActor
public final class IntercomActivityController {
    public static let minUpdateInterval: TimeInterval = 1
    public static let renewalAge: TimeInterval = 7.5 * 3600

    private let port: any LiveActivityPort
    private let clock: @Sendable () -> TimeInterval
    private let autoFlush: Bool
    private var id: String?
    private var attributes: IntercomAttributes?
    private var startedAt: TimeInterval = 0
    private var lastSent: IntercomAttributes.ContentState?
    private var lastSentAt: TimeInterval = -.infinity
    private var pending: IntercomAttributes.ContentState?
    private var flushTask: Task<Void, Never>?
    public private(set) var renewals = 0

    /// - Parameter autoFlush: schedule the coalesced update by itself (tests drive `tick`).
    public init(port: any LiveActivityPort = SystemLiveActivityPort(), autoFlush: Bool = true,
                clock: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
        self.port = port
        self.autoFlush = autoFlush
        self.clock = clock
    }

    public var isActive: Bool { id != nil }
    public var currentState: IntercomAttributes.ContentState? { pending ?? lastSent }
    public var age: TimeInterval? { id == nil ? nil : clock() - startedAt }

    /// Starts the activity (gesture-bound, foreground). Updates it instead when running.
    public func start(_ attributes: IntercomAttributes, _ state: IntercomAttributes.ContentState) async throws {
        if id != nil, self.attributes == attributes {
            await update(state)
            return
        }
        if id != nil { await end() }
        guard port.activitiesEnabled else { return }
        let newId = try port.request(attributes, state)
        id = newId
        self.attributes = attributes
        startedAt = clock()
        lastSent = state
        lastSentAt = clock()
        pending = nil
    }

    /// Queues `state`; sent now if the last update is ≥ 1 s old, else coalesced.
    public func update(_ state: IntercomAttributes.ContentState) async {
        guard id != nil else { return }
        if state == lastSent {
            pending = nil
            return
        }
        pending = state
        await tick()
        if pending != nil { scheduleFlush() }
    }

    /// Sends the coalesced state when due.
    public func tick() async {
        guard let id, let p = pending else { return }
        let now = clock()
        guard now - lastSentAt >= Self.minUpdateInterval else { return }
        pending = nil
        lastSent = p
        lastSentAt = now
        await port.update(id: id, p)
    }

    /// When the next coalesced update may go out.
    public var nextFlush: TimeInterval? {
        pending == nil ? nil : lastSentAt + Self.minUpdateInterval
    }

    /// App came to the foreground: renew an activity older than 7.5 h.
    public func appWillEnterForeground() async {
        guard let attrs = attributes, id != nil, clock() - startedAt >= Self.renewalAge else { return }
        let state = pending ?? lastSent ?? IntercomAttributes.ContentState()
        await end()
        do {
            try await start(attrs, state)
            renewals += 1
        } catch {
            PlatformLog.host.error("Live Activity renewal refused: \(error.localizedDescription)")
        }
    }

    public func end() async {
        flushTask?.cancel()
        flushTask = nil
        guard let id else { return }
        let final = pending ?? lastSent
        self.id = nil
        pending = nil
        await port.end(id: id, final)
    }

    private func scheduleFlush() {
        guard autoFlush, flushTask == nil, let due = nextFlush else { return }
        let wait = max(0, due - clock())
        flushTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            guard let self else { return }
            self.flushTask = nil
            await self.tick()
            if self.pending != nil { self.scheduleFlush() }
        }
    }
}
