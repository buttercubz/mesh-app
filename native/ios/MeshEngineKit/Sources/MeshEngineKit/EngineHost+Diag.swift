@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform

/// Diagnostics flight recorder (ridelog/1) for the bridge's `diag*` plugin methods
/// (contract C6 1.x): notes from platform and UI code, marks, status, export, clear.
extension EngineHost: DiagNoteSink {
    /// A platform note (src `plat`), stamped with the engine clock.
    public func diagNote(_ note: DiagNote) {
        _ = engineHandle.diagNote(src: "plat", ev: note.ev, fieldsJson: note.fieldsJSON)
    }

    /// A UI note forwarded by the plugin (src `ui`; `fieldsJson` is a JSON object of
    /// scalars). Returns false when the runtime refused it.
    @discardableResult
    public func diagUiNote(ev: String, fieldsJson: String) -> Bool {
        engineHandle.diagNote(src: "ui", ev: ev, fieldsJson: fieldsJson)
    }

    /// A user marker from a fixed chip label (`crackle`, `cantHear`, …).
    public func diagMark(_ label: String) -> Bool {
        engineHandle.diagMark(label: label)
    }

    /// Recording on/off (persisted; the RAM ring always records).
    public func diagSetEnabled(_ on: Bool) {
        DiagPreference.save(on)
        engineHandle.diagConfigure(enabled: on, debugBuild: DiagPreference.isDebugBuild)
    }

    public func diagStatus() -> DiagStatus {
        let s = engineHandle.diagStatus()
        hooks.diag.drain()
        let usage = hooks.diag.store?.usage() ?? DiagUsage(bytes: 0, sessions: 0, oldest: nil)
        return DiagStatus(enabled: s.enabled, sessionId: s.sessionId, rideKey: s.rideKey, node: s.node,
                          bytes: usage.bytes, sessions: usage.sessions, oldest: usage.oldest)
    }

    /// Stored sessions, newest first.
    public func diagList() -> [DiagSessionInfo] {
        engineHandle.diagFlush()
        hooks.diag.drain()
        return hooks.diag.store?.sessions() ?? []
    }

    /// Deletes every diagnostics file (keeps the toggle).
    public func diagClear() {
        engineHandle.diagFlush()
        hooks.diag.drain()
        hooks.diag.store?.clear()
    }

    /// Packs the last session (or all) plus `device.json` into a zip in the temporary
    /// directory, ready for the share sheet. Without files, the RAM ring is exported.
    public func diagExport(scope: DiagFileStore.ExportScope, extraDevice: [String: String] = [:]) throws -> URL {
        engineHandle.diagFlush()
        hooks.diag.drain()
        let s = engineHandle.diagStatus()
        let i = info()
        var device: [String: Any] = [
            "platform": "ios",
            "model": EngineHostConfig.machineIdentifier(),
            "app": i.appVersion,
            "apiVersion": i.apiVersion,
            "protocolVersion": Int(i.protocolVersion),
            "rtAbiVersion": Int(i.rtAbiVersion),
            "engine": i.engine,
            "status": i.status,
            "build": DiagPreference.isDebugBuild ? "debug" : "release",
            "recording": s.enabled,
        ]
        for (k, v) in extraDevice { device[k] = v }
        let data = try JSONSerialization.data(withJSONObject: device, options: [.sortedKeys])
        let stamp = Self.stamp(Date())
        let label = "\(s.rideKey ?? "nokey")-n\(s.node.map(String.init) ?? "x")-\(stamp)"
        let out = FileManager.default.temporaryDirectory.appending(path: "diag-export", directoryHint: .isDirectory)
        guard let store = hooks.diag.store else {
            let tmp = try DiagFileStore(directory: out.appending(path: "empty"))
            return try tmp.exportZip(scope: scope, label: label, deviceJSON: String(decoding: data, as: UTF8.self),
                                     ring: engineHandle.diagSnapshotRing(), outDir: out).url
        }
        return try store.exportZip(scope: scope, label: label, deviceJSON: String(decoding: data, as: UTF8.self),
                                   ring: engineHandle.diagSnapshotRing(), outDir: out).url
    }

    /// The app is terminating: closes the open segment.
    public func diagTerminate() {
        engineHandle.diagTerminate()
        hooks.diag.drain()
    }

    static func stamp(_ d: Date) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "UTC")
        f.dateFormat = "yyyyMMdd-HHmm"
        return f.string(from: d)
    }
}
