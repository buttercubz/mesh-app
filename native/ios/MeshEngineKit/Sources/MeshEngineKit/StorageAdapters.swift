@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform
import Synchronization

/// `PlatformHooks.persist` over the Keychain (`SecureStore`, §8): per-group identities,
/// epoch secrets and the Lab run index. Loaded once into `FfiEngineConfig.persisted`.
final class PersistAdapter: Sendable {
    let store: SecureStore
    private let log: @Sendable (FfiLogLevel, String) -> Void

    init(store: SecureStore, log: @escaping @Sendable (FfiLogLevel, String) -> Void) {
        self.store = store
        self.log = log
    }

    /// Every stored entry (sorted by key) for the engine configuration.
    func load() -> [FfiPersistedEntry] {
        let keys: [String]
        do {
            keys = try store.keys()
        } catch {
            log(.error, "persist: cannot list keychain items: \(error)")
            return []
        }
        return keys.compactMap { k in
            do {
                guard let v = try store.get(k) else { return nil }
                return FfiPersistedEntry(key: k, value: v)
            } catch {
                log(.error, "persist: cannot read \(k): \(error)")
                return nil
            }
        }
    }

    /// `persist(key, value)`; `nil` deletes.
    func persist(_ key: String, _ value: Data?) {
        do {
            try store.persist(key, value)
        } catch {
            // Logged, never silently dropped: the engine keeps its in-memory mirror.
            log(.error, "persist: keychain write of \(key) failed: \(error)")
        }
    }
}

/// `lab_write` / `lab_write_summary` / `lab_delete` over `LabFileStore` (labrun/1 JSON
/// Lines in Documents/lab, contract C7, §12.3).
final class LabAdapter: Sendable {
    let store: LabFileStore
    private let log: @Sendable (FfiLogLevel, String) -> Void

    init(store: LabFileStore, log: @escaping @Sendable (FfiLogLevel, String) -> Void) {
        self.store = store
        self.log = log
    }

    /// Appends one JSONL line to `<runId>.jsonl`; the `summary` line closes the file. The
    /// core's lines end with their own newline (`to_jsonl`); the store adds it back.
    func write(runId: String, line raw: String) {
        let line = Self.stripNewline(raw)
        do {
            try store.append(runId: runId, line: line)
        } catch {
            log(.error, "lab: append to \(runId) failed: \(error)")
            return
        }
        if Self.isSummaryLine(line) { store.finish(runId: runId) }
    }

    /// Appends one CSV line (header or row) to `<runId>-summary.csv`.
    func writeSummary(runId: String, line raw: String) {
        let line = Self.stripNewline(raw)
        do {
            let url = try store.summaryURL(for: runId)
            var text = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
            let existing = text.split(separator: "\n", omittingEmptySubsequences: true)
            // A header is written once even if a run's summary is emitted again.
            if existing.first.map(String.init) == line { return }
            if !text.isEmpty, !text.hasSuffix("\n") { text += "\n" }
            text += line + "\n"
            try store.writeSummary(runId: runId, csv: text)
        } catch {
            log(.error, "lab: summary of \(runId) failed: \(error)")
        }
    }

    func delete(runId: String) {
        do {
            try store.delete(runId: runId)
        } catch {
            log(.error, "lab: delete \(runId) failed: \(error)")
        }
    }

    /// Run finished (UI event): the file handle is released.
    func finish(runId: String) {
        store.finish(runId: runId)
    }

    /// One trailing `\n` (or `\r\n`) removed; inner newlines stay (and are refused).
    static func stripNewline(_ s: String) -> String {
        // Unicode scalars: "\r\n" is one Character in Swift.
        var u = Substring(s).unicodeScalars
        if u.last == "\n" { u.removeLast() }
        if u.last == "\r" { u.removeLast() }
        return String(u)
    }

    static func isSummaryLine(_ line: String) -> Bool {
        line.contains(#""summary""#) && CommandJSON.object(line)?["t"] as? String == "summary"
    }
}
