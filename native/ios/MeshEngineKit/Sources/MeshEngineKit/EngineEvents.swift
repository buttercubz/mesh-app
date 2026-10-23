import Foundation
import MeshActivityShared
import Synchronization

/// Fans the engine's UI events (JSON, forwarded verbatim, §3.5) out to every subscriber:
/// `AsyncStream`s and an optional callback (the Capacitor bridge's `notifyListeners`).
final class EventHub: Sendable {
    private struct State {
        var next = 0
        var streams: [Int: AsyncStream<String>.Continuation] = [:]
        var handler: (@Sendable (String) -> Void)?
    }

    private let state = Mutex(State())

    /// A new stream of every event from now on (buffering the newest 256).
    func stream() -> AsyncStream<String> {
        let (s, c) = AsyncStream.makeStream(of: String.self, bufferingPolicy: .bufferingNewest(256))
        let id = state.withLock { st -> Int in
            st.next += 1
            st.streams[st.next] = c
            return st.next
        }
        c.onTermination = { [weak self] _ in
            _ = self?.state.withLock { $0.streams.removeValue(forKey: id) }
        }
        return s
    }

    func setHandler(_ h: (@Sendable (String) -> Void)?) {
        state.withLock { $0.handler = h }
    }

    func emit(_ json: String) {
        let (conts, handler) = state.withLock { (Array($0.streams.values), $0.handler) }
        for c in conts { c.yield(json) }
        handler?(json)
    }

    func finishAll() {
        let conts = state.withLock { st -> [AsyncStream<String>.Continuation] in
            let c = Array(st.streams.values)
            st.streams.removeAll()
            return c
        }
        for c in conts { c.finish() }
    }
}

/// What the host needs from the UI event stream: Go-live inputs (share distance), the Live
/// Activity content (§10.1.5) and Lab run status. Only a few discriminants are read.
struct EventDigest: Sendable, Equatable {
    var hasSession = false
    var groupName = ""
    var live = false
    var mode = "vox"
    var talk: TalkState = .idle
    var ladder: LadderLevel = .l0
    var shareDistance = false
    var speakers: [String] = []
    var inRange = 0
    var total = 0
    var onCall = false
    var bluetoothOff = false
    var lab: LabStatus?
    var labRunId: String?

    /// Applies one UI event; returns `true` when the Live Activity content may change.
    mutating func apply(_ json: String) -> Bool {
        guard let e = CommandJSON.object(json), let type = e["type"] as? String else { return false }
        let before = self
        switch type {
        case "snapshot":
            if let s = e["s"] as? [String: Any] { snapshot(s) }
        case "labProgress":
            labRunId = e["runId"] as? String ?? labRunId
            let sample = e["sample"] as? [String: Any] ?? [:]
            lab = LabStatus(bench: lab?.bench ?? "", phase: .running,
                            elapsedS: Int((e["elapsedS"] as? Double) ?? 0),
                            linksUp: sample["links"] as? Int ?? 0)
        case "labFinished":
            labRunId = e["runId"] as? String ?? labRunId
            let summary = e["summary"] as? [String: Any] ?? [:]
            let verdict = summary["verdict"] as? String
            lab = LabStatus(bench: lab?.bench ?? "", phase: verdict == "inconclusive" ? .failed : .finished,
                            elapsedS: Int((summary["durationS"] as? Double) ?? Double(lab?.elapsedS ?? 0)),
                            linksUp: lab?.linksUp ?? 0, verdict: verdict)
        case "notice":
            switch e["key"] as? String {
            case "onCall": onCall = true
            case "callEnded": onCall = false
            default: break
            }
        default:
            return false
        }
        return self != before
    }

    /// A Lab run was started from this phone (`labStart` accepted).
    mutating func labStarted(bench: String, runId: String?) {
        labRunId = runId
        lab = LabStatus(bench: bench, phase: .waiting)
    }

    private mutating func snapshot(_ s: [String: Any]) {
        let session = s["session"] as? [String: Any]
        hasSession = session != nil
        groupName = session?["groupName"] as? String ?? ""
        live = session?["live"] as? Bool ?? false
        mode = session?["mode"] as? String ?? "vox"
        talk = TalkState(rawValue: session?["talk"] as? String ?? "idle") ?? .idle
        ladder = LadderLevel(rawValue: session?["ladder"] as? String ?? "L0") ?? .l0
        shareDistance = session?["shareDistance"] as? Bool ?? false
        let me = session?["myNodeId"] as? Int
        let talkers = s["talkers"] as? [[String: Any]] ?? []
        speakers = talkers.filter { ($0["nodeId"] as? Int) != me }.compactMap { $0["name"] as? String }
        let members = s["members"] as? [[String: Any]] ?? []
        let others = members.filter { ($0["status"] as? String) != "me" }
        total = others.count
        inRange = others.filter { ["direct", "relayed", "oneWay"].contains($0["status"] as? String ?? "") }.count
        if let mine = members.first(where: { ($0["status"] as? String) == "me" }) {
            onCall = (mine["flags"] as? [String] ?? []).contains("onCall")
        }
        let device = s["device"] as? [String: Any]
        bluetoothOff = (device?["bluetooth"] as? String) == "off"
    }

    /// The Live Activity content (`IntercomAttributes.ContentState`, contract C9).
    func contentState(hardMuted: Bool, pttLatched: Bool) -> IntercomAttributes.ContentState {
        IntercomAttributes.ContentState(
            live: live,
            muted: hardMuted || mode == "muted",
            talk: hardMuted ? .muted : talk,
            pttLatched: pttLatched && mode == "ptt",
            speakers: speakers,
            inRange: inRange,
            total: total,
            ladder: ladder,
            onCall: onCall,
            alone: live && total > 0 && inRange == 0,
            bluetoothOff: bluetoothOff,
            labStatus: lab
        )
    }
}
