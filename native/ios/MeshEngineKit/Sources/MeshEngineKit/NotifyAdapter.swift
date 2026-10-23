import CMeshAudio
import Foundation
import MeshActivityShared
import MeshPlatform

/// Localised notification text. The engine sends i18n keys + params, never text (§3.7);
/// the bridge hands the UI's `notices`/`errors` catalogs of the current language to the
/// host (`EngineHost.setNotificationCatalog`), so native notifications use the same
/// strings as the toasts. Without a catalog nothing is posted (logged instead).
public struct NotificationCatalog: Sendable, Equatable {
    /// Notification title (the app name in the UI language).
    public var title: String
    /// key → template with `{{param}}` placeholders (i18next syntax).
    public var templates: [String: String]

    public init(title: String, templates: [String: String]) {
        self.title = title
        self.templates = templates
    }

    /// The body for `key`, with params substituted; nil for an unknown key.
    public func body(_ key: String, params: [String: Any]) -> String? {
        guard var t = templates[key] else { return nil }
        for (k, v) in params {
            let s: String
            switch v {
            case let n as NSNumber: s = n.stringValue
            case let str as String: s = str
            case let arr as [Any]: s = arr.map { "\($0)" }.joined(separator: ", ")
            default: s = "\(v)"
            }
            t = t.replacingOccurrences(of: "{{\(k)}}", with: s)
        }
        return t
    }
}

/// Local notifications and the Live Activity (DESIGN §10.1.3-§10.1.5): the engine's
/// `notify` hook, the platform's capture-lost notice and the UI event digest end here.
@MainActor
final class NotifyAdapter {
    let activity: IntercomActivityController
    let notifier: ResumeNotifier
    private(set) var digest = EventDigest()
    private var catalog: NotificationCatalog?
    private var attributes: IntercomAttributes?
    private let log: @Sendable (String) -> Void
    /// Posted notification ids, newest last (tests and diagnostics).
    private(set) var posted: [String] = []
    /// Diagnostics notes (Live Activity start / failure / end).
    var diag: (@Sendable (DiagNote) -> Void)?

    init(activity: IntercomActivityController, notifier: ResumeNotifier, log: @escaping @Sendable (String) -> Void) {
        self.activity = activity
        self.notifier = notifier
        self.log = log
    }

    func setCatalog(_ c: NotificationCatalog?) {
        catalog = c
    }

    // MARK: Live Activity

    /// Starts (or updates) the activity inside the gesture, before backgrounding.
    func startActivity(_ kind: GesturePlan.Activity) {
        let attrs: IntercomAttributes
        switch kind {
        case .ride:
            attrs = IntercomAttributes(kind: .ride, title: digest.groupName)
        case .lab(let bench):
            if digest.lab?.bench != bench || digest.lab?.phase != .running {
                digest.labStarted(bench: bench, runId: nil)
            }
            attrs = IntercomAttributes(kind: .lab, title: bench)
        }
        attributes = attrs
        let state = currentState()
        Task { [activity, log, diag] in
            do {
                try await activity.start(attrs, state)
                diag?(.liveActivity("started"))
            } catch {
                let ns = error as NSError
                diag?(.liveActivity("fail", err: "\(ns.domain):\(ns.code)"))
                log("Live Activity not started: \(error.localizedDescription)")
            }
        }
    }

    func endActivity() {
        attributes = nil
        diag?(.liveActivity("end"))
        Task { [activity] in await activity.end() }
    }

    var hasActivity: Bool { attributes != nil }

    /// RT flags changed (mute / latch from the bridge or the Live Activity buttons).
    func flagsChanged() {
        pushState()
    }

    func currentState() -> IntercomAttributes.ContentState {
        let flags = cmesh_flags()
        return digest.contentState(hardMuted: flags & CMESH_FLAG_HARD_MUTE != 0,
                                   pttLatched: flags & CMESH_FLAG_PTT_LATCH_TOGGLE != 0)
    }

    private func pushState() {
        guard attributes != nil else { return }
        let state = currentState()
        Task { [activity] in await activity.update(state) }
    }

    // MARK: Inputs

    func uiEvent(_ json: String) {
        if digest.apply(json) { pushState() }
    }

    func labStarted(bench: String, runId: String?) {
        digest.labStarted(bench: bench, runId: runId)
        pushState()
    }

    /// `PlatformHooks.notify`.
    func notify(key: String, paramsJson: String, isError: Bool) {
        switch key {
        case "captureLost":
            showResume()
        case "captureRestored":
            notifier.clear()
        default:
            post(id: (isError ? "mesh.error." : "mesh.notice.") + key, key: key, paramsJson: paramsJson,
                 timeSensitive: isError)
        }
    }

    /// The platform's own capture-lost notice (§10.1.4 CAPTURE_LOST step 3).
    func audioEvent(_ e: PlatformAudioEvent) {
        switch e {
        case .captureLostNotice(true): showResume()
        case .captureLostNotice(false): notifier.clear()
        case .ladder(let l):
            if digest.ladder != l {
                digest.ladder = l
                pushState()
            }
        default: break
        }
    }

    private func showResume() {
        guard let c = catalog, let body = c.body("captureLost", params: [:]) else {
            log("notify: no catalog for captureLost; notification not posted")
            return
        }
        posted.append(ResumeNotifier.notificationId)
        Task { [notifier, log] in
            do { try await notifier.show(title: c.title, body: body) } catch {
                log("notify: resume notification refused: \(error.localizedDescription)")
            }
        }
    }

    private func post(id: String, key: String, paramsJson: String, timeSensitive: Bool) {
        let params = CommandJSON.object(paramsJson) ?? [:]
        guard let c = catalog, let body = c.body(key, params: params) else {
            log("notify: no text for \(key); notification not posted")
            return
        }
        posted.append(id)
        Task { [notifier, log] in
            do { try await notifier.post(id: id, title: c.title, body: body, timeSensitive: timeSensitive) } catch {
                log("notify: \(key) refused: \(error.localizedDescription)")
            }
        }
    }
}
