@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform

/// `PlatformHooks.diag_write` over `DiagFileStore` (ridelog/1 segments in
/// `Application Support/diag`). The hook runs on the engine thread, so writes are queued
/// on a utility queue and the engine never waits for the disk. A store that could not be
/// created (nil) leaves only the runtime's RAM ring.
final class DiagAdapter: Sendable {
    let store: DiagFileStore?
    private let queue = DispatchQueue(label: "mesh.diag.files", qos: .utility)
    private let log: @Sendable (FfiLogLevel, String) -> Void

    init(store: DiagFileStore?, log: @escaping @Sendable (FfiLogLevel, String) -> Void) {
        self.store = store
        self.log = log
    }

    /// Queues one `diag_write` batch.
    func write(sessionId: String, seg: UInt32, lines: [String]) {
        guard let store else { return }
        let log = self.log
        queue.async {
            do {
                try store.append(sessionId: sessionId, seg: seg, lines: lines)
            } catch {
                log(.warn, "diag: append to \(sessionId)/\(seg) failed: \(error)")
            }
        }
    }

    /// Waits until every queued batch is on disk (before an export or a listing).
    func drain() {
        queue.sync {}
    }
}

extension EngineRef {
    /// Sends a platform note to the engine (no-op before the engine exists).
    func note(_ n: DiagNote) {
        _ = engine?.diagNote(src: "plat", ev: n.ev, fieldsJson: n.fieldsJSON)
    }
}

/// Recorder state for the Settings screen (`diagStatus`).
public struct DiagStatus: Sendable, Equatable {
    public var enabled: Bool
    public var sessionId: String?
    public var rideKey: String?
    public var node: UInt16?
    public var bytes: Int
    public var sessions: Int
    /// Oldest stored session (modification date), if any.
    public var oldest: Date?
}

/// The recording preference (UserDefaults: a plain boolean, never a secret). Default: on
/// in debug builds and while the app version is below 1.0 (field-testing period).
public enum DiagPreference {
    static let key = "convlink.diag.enabled"

    public static func defaultEnabled(appLabel: String, debug: Bool) -> Bool {
        if debug { return true }
        let major = Int(appLabel.split(separator: ".").first ?? "") ?? 0
        return major < 1
    }

    public static func load(appLabel: String, debug: Bool, defaults: UserDefaults = .standard) -> Bool {
        if defaults.object(forKey: key) == nil { return defaultEnabled(appLabel: appLabel, debug: debug) }
        return defaults.bool(forKey: key)
    }

    public static func save(_ on: Bool, defaults: UserDefaults = .standard) {
        defaults.set(on, forKey: key)
    }

    public static var isDebugBuild: Bool {
        #if DEBUG
        true
        #else
        false
        #endif
    }
}
