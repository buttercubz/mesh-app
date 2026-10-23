import Foundation

/// A scalar of a diagnostics note field (no objects: the runtime refuses them).
public enum DiagValue: Sendable, Equatable {
    case string(String)
    case int(Int)
    case double(Double)
    case bool(Bool)
    case null
    case ints([Int])
}

/// A platform diagnostics event (`ridelog/1` `e` line, src `plat`), injected through
/// `MeshEngine.diag_note` so it shares the engine clock with core events.
///
/// Privacy rules (diagnostics spec §5) are enforced here **and** by the runtime's
/// redaction filter: only codes and numbers; never device or headset names, MAC
/// addresses, keys or user text. String values longer than 64 characters are cut.
public struct DiagNote: Sendable, Equatable {
    public var ev: String
    public var fields: [(String, DiagValue)]

    public init(_ ev: String, _ fields: [(String, DiagValue)] = []) {
        self.ev = ev
        self.fields = fields
    }

    public static func == (a: DiagNote, b: DiagNote) -> Bool {
        a.ev == b.ev && a.fieldsJSON == b.fieldsJSON
    }

    /// The fields as a compact JSON object (keys in the given order).
    public var fieldsJSON: String {
        var out = "{"
        for (i, (k, v)) in fields.enumerated() {
            if i > 0 { out += "," }
            out += Self.quote(k) + ":" + Self.encode(v)
        }
        return out + "}"
    }

    static func encode(_ v: DiagValue) -> String {
        switch v {
        case .string(let s): return quote(String(s.prefix(64)))
        case .int(let i): return String(i)
        case .double(let d): return d.isFinite ? String(format: "%.3f", d) : "null"
        case .bool(let b): return b ? "true" : "false"
        case .null: return "null"
        case .ints(let xs): return "[" + xs.prefix(64).map(String.init).joined(separator: ",") + "]"
        }
    }

    static func quote(_ s: String) -> String {
        var out = "\""
        for u in s.unicodeScalars {
            switch u {
            case "\"": out += "\\\""
            case "\\": out += "\\\\"
            case "\n": out += "\\n"
            case "\r": out += "\\r"
            case "\t": out += "\\t"
            default:
                if u.value < 0x20 {
                    out += String(format: "\\u%04x", u.value)
                } else {
                    out.unicodeScalars.append(u)
                }
            }
        }
        return out + "\""
    }
}

/// Where platform code sends notes (the engine host; a no-op before it exists).
public protocol DiagNoteSink: AnyObject, Sendable {
    func diagNote(_ note: DiagNote)
}

// MARK: - Typed notes (the events only the platform can see, spec §2.4 "Platform")

extension DiagNote {
    /// A permission check or request with the exact API used and its answer.
    public static func perm(name: String, state: String, via: String, api: String) -> DiagNote {
        DiagNote("perm", [("name", .string(name)), ("state", .string(state)), ("via", .string(via)),
                          ("api", .string(api))])
    }

    /// App visibility change.
    public static func app(_ v: AppVisibility, locked: Bool?) -> DiagNote {
        let state: String
        switch v {
        case .foreground: state = "foreground"
        case .background: state = "background"
        case .inactive: state = "inactive"
        }
        return DiagNote("app", [("state", .string(state)), ("locked", locked.map(DiagValue.bool) ?? .null)])
    }

    /// Bluetooth radio state (CoreBluetooth manager state mapped by `RadioStateMonitor`).
    public static func bt(_ s: RadioState) -> DiagNote {
        let state: String
        switch s {
        case .on: state = "on"
        case .off: state = "off"
        case .unauthorized: state = "unauthorized"
        case .unsupported: state = "unsupported"
        }
        return DiagNote("bt", [("state", .string(state))])
    }

    /// Audio IO state reported to the core (interruptions, capture / render loss).
    public static func audioIo(_ s: AudioIoState) -> DiagNote {
        switch s {
        case .running: return DiagNote("audioIo", [("state", .string("running"))])
        case .interrupted(let r):
            return DiagNote("interruption", [("kind", .string(r.rawValue)), ("phase", .string("begin"))])
        case .restarting: return DiagNote("audioIo", [("state", .string("restarting"))])
        case .captureLost: return DiagNote("audioIo", [("state", .string("captureLost"))])
        case .renderLost: return DiagNote("audioIo", [("state", .string("renderLost"))])
        case .restored:
            return DiagNote("interruption", [("kind", .string("restored")), ("phase", .string("end"))])
        }
    }

    /// Audio route change (never the device name: only a coarse headset class).
    public static func audioRoute(_ r: AudioRouteInfo, prev: AudioRouteInfo?) -> DiagNote {
        DiagNote("audioRoute", [
            ("route", .string(r.kind.rawValue)),
            ("prev", prev.map { .string($0.kind.rawValue) } ?? .null),
            ("inputRate", .int(Int(r.inputRate))),
            ("output", .string(r.output.rawValue)),
            ("headsetClass", .string(headsetClass(r))),
        ])
    }

    /// `car`, `bluetooth`, `wired`, `none` — what kind of headset, never which one.
    public static func headsetClass(_ r: AudioRouteInfo) -> String {
        switch r.output {
        case .carAudio: return "car"
        case .bluetoothHfp, .bluetoothLe: return "bluetooth"
        case .wired: return "wired"
        case .airplay: return "airplay"
        case .speaker, .receiver, .none: return r.headsetPresent ? "unknown" : "none"
        }
    }

    /// Audio session open attempt (VPIO) and its outcome.
    public static func audioOpen(api: String, path: String, ok: Bool, err: String?) -> DiagNote {
        DiagNote("audioOpen", [("api", .string(api)), ("path", .string(path)), ("ok", .bool(ok)),
                               ("err", err.map { .string($0) } ?? .null)])
    }

    /// Ladder level change (L0-L3).
    public static func ladder(_ level: String) -> DiagNote {
        DiagNote("ladder", [("level", .string(level))])
    }

    /// "Mic paused" banner shown or cleared by the platform's mic-loss state machine.
    public static func banner(_ key: String, on: Bool) -> DiagNote {
        DiagNote("banner", [("key", .string(key)), ("on", .bool(on))])
    }

    /// Live Activity requested / ended / failed.
    public static func liveActivity(_ state: String, err: String? = nil) -> DiagNote {
        DiagNote("liveActivity", [("state", .string(state)), ("err", err.map { .string($0) } ?? .null)])
    }

    /// Power and thermal state (sent on change).
    public static func power(lowPower: Bool?, thermal: String?, battery: Double?) -> DiagNote {
        DiagNote("power", [("lowPower", lowPower.map(DiagValue.bool) ?? .null),
                           ("thermal", thermal.map { .string($0) } ?? .null),
                           ("bat", battery.map { .double($0) } ?? .null)])
    }

    /// OS signals: memory warning, cold start, terminate.
    public static func os(_ kind: String) -> DiagNote {
        DiagNote("os", [("kind", .string(kind))])
    }
}
