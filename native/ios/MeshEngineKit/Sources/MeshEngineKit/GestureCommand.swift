import Foundation
import MeshPlatform

/// The ★ capture-gated commands (DESIGN §3.2, §3.7, §11.1). Mirrors mesh-api
/// `GESTURE_COMMANDS`: goLive, resumeMic, labStart with `params.audio ≠ "off"`,
/// audioCheck with `op == "start"`. The shared fixture `crates/mesh-api/fixtures/
/// commands.json` (`requiresGesture`) is replayed by the tests so both stay equal.
public enum GestureCommand: Sendable, Equatable {
    case goLive
    case resumeMic
    case labStart(LabGesture)
    case audioCheckStart(mode: String)

    /// The Lab parameters that decide the OS-level work of a `labStart` tap.
    public struct LabGesture: Sendable, Equatable {
        public var bench: String
        /// "renderOnly" or "full" ("off" is never gesture-gated).
        public var audio: String
        /// "none" | "audio" | "audioLocation".
        public var anchor: String
        /// Lab T2 toggle (§10.1.2); absent → mixable, like the product session.
        public var mixable: Bool
    }

    /// Classifies a UI command JSON; `nil` for every command that is not gesture-gated
    /// (including invalid JSON, which the engine itself refuses with `invalidCommand`).
    public static func classify(_ json: String) -> GestureCommand? {
        guard let obj = CommandJSON.object(json), let type = obj["type"] as? String else { return nil }
        switch type {
        case "goLive":
            return .goLive
        case "resumeMic":
            return .resumeMic
        case "labStart":
            let params = obj["params"] as? [String: Any] ?? [:]
            // NotEquals: the field must be present and differ from "off" (mesh-api rule).
            guard let audio = params["audio"] as? String, audio != "off" else { return nil }
            return .labStart(LabGesture(bench: obj["bench"] as? String ?? "",
                                        audio: audio,
                                        anchor: params["anchor"] as? String ?? "audio",
                                        mixable: params["mixable"] as? Bool ?? true))
        case "audioCheck":
            guard obj["op"] as? String == "start" else { return nil }
            return .audioCheckStart(mode: obj["mode"] as? String ?? "tone")
        default:
            return nil
        }
    }

    public static func requiresGesture(_ json: String) -> Bool {
        classify(json) != nil
    }

    /// The engine refuses these while the ride is live (`goLive` → alreadyLive,
    /// `audioCheck{start}` → wrongMode, §3.2), so the bridge skips their OS-level work then.
    public static func isRefusedWhileLive(_ g: GestureCommand) -> Bool {
        switch g {
        case .goLive, .audioCheckStart: return true
        case .resumeMic, .labStart: return false
        }
    }
}

/// The OS-level work the bridge performs synchronously inside the tap, before the command
/// reaches the engine (§3.2): activate the session and start VPIO muted, start the
/// location anchor when requested, start or update the Live Activity.
public struct GesturePlan: Sendable, Equatable {
    public enum Activity: Sendable, Equatable {
        case ride
        case lab(bench: String)
    }

    /// The VPIO path to start (nil: no audio work).
    public var path: AudioPath?
    public var mixable: Bool
    public var locationAnchor: Bool
    public var activity: Activity?
    /// The gesture restarts a lost capture of a live session (resumeMic).
    public var resume: Bool

    /// - Parameter shareDistance: the session's "Share my distance" (location anchor of
    ///   Go live, D8).
    public static func make(_ g: GestureCommand, shareDistance: Bool) -> GesturePlan {
        switch g {
        case .goLive:
            // Mixable product session: music keeps playing, ducked while someone talks
            // (AudioSessionConfig.make, 2026-10-23 deviation from §10.1.2).
            return GesturePlan(path: .full, mixable: true, locationAnchor: shareDistance,
                               activity: .ride, resume: false)
        case .resumeMic:
            return GesturePlan(path: .full, mixable: true, locationAnchor: shareDistance,
                               activity: .ride, resume: true)
        case .labStart(let l):
            let path: AudioPath
            if l.audio == "renderOnly" {
                path = .renderOnly
            } else {
                // T5a measures the headset loop with voice processing bypassed (§12.2).
                path = l.bench == "T5a" ? .t5aBypass : .full
            }
            return GesturePlan(path: path, mixable: l.mixable, locationAnchor: l.anchor == "audioLocation",
                               activity: .lab(bench: l.bench), resume: false)
        case .audioCheckStart:
            // A temporary session: no anchor, no Live Activity (F0.3).
            return GesturePlan(path: .full, mixable: true, locationAnchor: false, activity: nil,
                               resume: false)
        }
    }
}

/// Small JSON helpers for UI commands and results (the contract types are generated for
/// TypeScript; the bridge only peeks at a few discriminants).
enum CommandJSON {
    static func object(_ json: String) -> [String: Any]? {
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return obj
    }

    /// `{"ok":false,"code":<code>}` (mesh-api `CommandResult` error shape).
    static func error(_ code: String) -> String {
        #"{"ok":false,"code":"\#(code)"}"#
    }

    static func isOk(_ result: String) -> Bool {
        object(result)?["ok"] as? Bool == true
    }

    static func string(_ obj: [String: Any]) -> String {
        guard let d = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys]) else { return "{}" }
        return String(decoding: d, as: UTF8.self)
    }
}
