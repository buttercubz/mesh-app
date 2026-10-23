import ActivityKit
import Foundation

/// Live Activity attributes shared by the app and the `IntercomWidget` extension
/// (contract C9, DESIGN §10.1.5). The widget UI (WP-23) only renders these types; it never
/// defines its own copies.
public struct IntercomAttributes: ActivityAttributes, Hashable, Sendable {
    /// What the activity represents.
    public enum Kind: String, Codable, Hashable, Sendable {
        /// A group ride (lobby or riding).
        case ride
        /// A Lab (field test) run; Phase 0 minimal view.
        case lab
    }

    public var kind: Kind
    /// Group name, or the Lab bench title for Lab runs (user content, not localised).
    public var title: String

    public init(kind: Kind, title: String) {
        self.kind = kind
        self.title = title
    }

    /// Dynamic state, updated at most once per second and on change.
    public struct ContentState: Codable, Hashable, Sendable {
        /// Go live done (capture running or muted until the ride starts).
        public var live: Bool
        /// Transmit hard-muted (RT flag bit 1).
        public var muted: Bool
        /// Local talk state (mesh-api `TalkState`).
        public var talk: TalkState
        /// PTT latched on (from the lock screen `LatchTalkIntent`).
        public var pttLatched: Bool
        /// Names of the current speakers, at most `maxSpeakers`.
        public var speakers: [String]
        /// Members currently reachable.
        public var inRange: Int
        /// Members in the group.
        public var total: Int
        /// Degradation ladder level (DESIGN §7.8).
        public var ladder: LadderLevel
        /// On a phone call (PRESENCE `on_call`).
        public var onCall: Bool
        /// No member reachable for 10 s.
        public var alone: Bool
        /// Bluetooth is off.
        public var bluetoothOff: Bool
        /// Lab run status (Lab activities only).
        public var labStatus: LabStatus?

        public static let maxSpeakers = 2

        public init(
            live: Bool = false,
            muted: Bool = false,
            talk: TalkState = .idle,
            pttLatched: Bool = false,
            speakers: [String] = [],
            inRange: Int = 0,
            total: Int = 0,
            ladder: LadderLevel = .l0,
            onCall: Bool = false,
            alone: Bool = false,
            bluetoothOff: Bool = false,
            labStatus: LabStatus? = nil
        ) {
            self.live = live
            self.muted = muted
            self.talk = talk
            self.pttLatched = pttLatched
            self.speakers = Array(speakers.prefix(Self.maxSpeakers))
            self.inRange = max(0, inRange)
            self.total = max(0, total)
            self.ladder = ladder
            self.onCall = onCall
            self.alone = alone
            self.bluetoothOff = bluetoothOff
            self.labStatus = labStatus
        }

        public init(from decoder: any Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            self.init(
                live: try c.decode(Bool.self, forKey: .live),
                muted: try c.decode(Bool.self, forKey: .muted),
                talk: try c.decode(TalkState.self, forKey: .talk),
                pttLatched: try c.decode(Bool.self, forKey: .pttLatched),
                speakers: try c.decode([String].self, forKey: .speakers),
                inRange: try c.decode(Int.self, forKey: .inRange),
                total: try c.decode(Int.self, forKey: .total),
                ladder: try c.decode(LadderLevel.self, forKey: .ladder),
                onCall: try c.decode(Bool.self, forKey: .onCall),
                alone: try c.decode(Bool.self, forKey: .alone),
                bluetoothOff: try c.decode(Bool.self, forKey: .bluetoothOff),
                labStatus: try c.decodeIfPresent(LabStatus.self, forKey: .labStatus)
            )
        }

        private enum CodingKeys: String, CodingKey {
            case live, muted, talk, pttLatched, speakers, inRange, total, ladder, onCall, alone
            case bluetoothOff, labStatus
        }
    }
}

/// Local talk state; raw values match mesh-api `TalkState`.
public enum TalkState: String, Codable, Hashable, Sendable, CaseIterable {
    case idle
    case talking
    case blocked
    case muted
}

/// Audio degradation ladder (DESIGN §7.8); raw values match mesh-api `LadderLevel`.
public enum LadderLevel: String, Codable, Hashable, Sendable, CaseIterable {
    case l0 = "L0"
    case l1 = "L1"
    case l2 = "L2"
    case l3 = "L3"
}

/// Minimal Lab run status for the Phase 0 Live Activity (T3 variant).
public struct LabStatus: Codable, Hashable, Sendable {
    public enum Phase: String, Codable, Hashable, Sendable, CaseIterable {
        case waiting
        case running
        case finished
        case failed
    }

    /// Bench id, e.g. "T3".
    public var bench: String
    public var phase: Phase
    /// Seconds since the run started.
    public var elapsedS: Int
    /// BLE links currently up.
    public var linksUp: Int
    /// Go / no-go verdict once finished ("go", "noGo", "inconclusive").
    public var verdict: String?

    public init(bench: String, phase: Phase, elapsedS: Int = 0, linksUp: Int = 0, verdict: String? = nil) {
        self.bench = bench
        self.phase = phase
        self.elapsedS = max(0, elapsedS)
        self.linksUp = max(0, linksUp)
        self.verdict = verdict
    }
}
