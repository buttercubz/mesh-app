import Foundation
import MeshActivityShared
import SwiftUI
import WidgetKit

/// Widget extension entry point (WP-23, DESIGN §10.1.5). The extension renders the
/// `IntercomAttributes` Live Activity (contract C9, owned by WP-14); it never defines
/// shared types of its own.
@main
struct IntercomWidgetBundle: WidgetBundle {
    var body: some Widget {
        IntercomLiveActivity()
    }
}

// MARK: - Localisation

/// UI languages the widget ships (DESIGN §11: neutral Spanish and English).
enum WidgetLanguage: Sendable, Equatable {
    case en
    case es

    /// Picks Spanish for any `es*` locale and English otherwise.
    static func resolve(_ locale: Locale) -> WidgetLanguage {
        let code = locale.language.languageCode?.identifier
            ?? Locale.preferredLanguages.first.map { String($0.prefix(2)) }
        return code == "es" ? .es : .en
    }
}

/// Every user-facing string of the widget. The extension target carries no resource
/// files, so the catalog is compiled in; the exhaustive `switch` per key gives
/// compile-time EN/ES parity (a key cannot exist in one language only).
enum WidgetText: Sendable, Equatable {
    case appName
    case lobby
    case live
    case muted
    case listening
    case talking
    case channelBusy
    case talkLatched
    case speaking(String)
    case inRange(Int, Int)
    case bluetoothOff
    case alone
    case onCall
    case restarting
    case micPaused
    case relayOnly
    case buttonMute
    case buttonUnmute
    case buttonTalk
    case buttonTalkOn
    case buttonResume
    case labTitle(String)
    case labWaiting
    case labRunning
    case labFinished
    case labFailed
    case labLinks(Int)
    case labVerdictGo
    case labVerdictNoGo
    case labVerdictInconclusive

    func string(_ lang: WidgetLanguage) -> String {
        switch lang {
        case .en: english
        case .es: spanish
        }
    }

    private var english: String {
        switch self {
        case .appName: "convLink Voice"
        case .lobby: "Not live yet"
        case .live: "Live"
        case .muted: "Muted"
        case .listening: "Listening"
        case .talking: "Talking"
        case .channelBusy: "Channel busy"
        case .talkLatched: "Talk latched on"
        case .speaking(let names): "\(names) speaking"
        case .inRange(let n, let total): "\(n) of \(total) in range"
        case .bluetoothOff: "Bluetooth is off. Turn it on to stay connected"
        case .alone: "No one in range"
        case .onCall: "On a call. Intercom paused"
        case .restarting: "Restarting audio…"
        case .micPaused: "Mic paused. Tap Resume"
        case .relayOnly: "No audio (still relaying). Open the app"
        case .buttonMute: "Mute"
        case .buttonUnmute: "Unmute"
        case .buttonTalk: "Talk"
        case .buttonTalkOn: "Talk on"
        case .buttonResume: "Resume mic"
        case .labTitle(let bench): "Field test \(bench)"
        case .labWaiting: "Waiting"
        case .labRunning: "Running"
        case .labFinished: "Finished"
        case .labFailed: "Failed"
        case .labLinks(let n): "\(n) links up"
        case .labVerdictGo: "Go"
        case .labVerdictNoGo: "No-go"
        case .labVerdictInconclusive: "Inconclusive"
        }
    }

    private var spanish: String {
        switch self {
        case .appName: "convLink Voice"
        case .lobby: "Aún sin transmitir"
        case .live: "En vivo"
        case .muted: "Silenciado"
        case .listening: "Escuchando"
        case .talking: "Hablando"
        case .channelBusy: "Canal ocupado"
        case .talkLatched: "Hablar fijado"
        case .speaking(let names): "Habla: \(names)"
        case .inRange(let n, let total): "\(n) de \(total) en alcance"
        case .bluetoothOff: "Bluetooth apagado. Actívalo para seguir conectado"
        case .alone: "Nadie en alcance"
        case .onCall: "En una llamada. Intercomunicador en pausa"
        case .restarting: "Reiniciando el audio…"
        case .micPaused: "Micrófono en pausa. Toca Reanudar"
        case .relayOnly: "Sin audio (sigue retransmitiendo). Abre la app"
        case .buttonMute: "Silenciar"
        case .buttonUnmute: "Activar mic"
        case .buttonTalk: "Hablar"
        case .buttonTalkOn: "Hablando"
        case .buttonResume: "Reanudar mic"
        case .labTitle(let bench): "Prueba de campo \(bench)"
        case .labWaiting: "En espera"
        case .labRunning: "En curso"
        case .labFinished: "Terminada"
        case .labFailed: "Fallida"
        case .labLinks(let n): "\(n) enlaces activos"
        case .labVerdictGo: "Apta"
        case .labVerdictNoGo: "No apta"
        case .labVerdictInconclusive: "No concluyente"
        }
    }
}

// MARK: - Presentation model (pure, no SwiftUI state)

/// What the ride view shows for one `ContentState`. Kept free of views so the rules
/// (which banner wins, which buttons appear) read in one place.
struct RidePresentation: Equatable {
    /// Highest-priority problem banner, if any.
    enum Banner: Equatable {
        case bluetoothOff
        case onCall
        case micPaused
        case relayOnly
        case restarting
        case alone
    }

    enum Tone: Equatable {
        case neutral
        case live
        case warn
        case danger
    }

    let banner: Banner?
    let status: WidgetText
    let tone: Tone
    let symbol: String
    let showMute: Bool
    let showTalk: Bool
    let showResume: Bool
    let latched: Bool
    let muted: Bool
    let speakers: WidgetText?
    let inRange: WidgetText

    init(_ s: IntercomAttributes.ContentState) {
        muted = s.muted
        latched = s.pttLatched
        inRange = .inRange(min(s.inRange, s.total), s.total)
        speakers = s.speakers.isEmpty ? nil : .speaking(s.speakers.joined(separator: ", "))

        // Priority: radio off > call > capture lost > relay-only > restarting > alone.
        if s.bluetoothOff {
            banner = .bluetoothOff
        } else if s.onCall {
            banner = .onCall
        } else if s.ladder == .l2 {
            banner = .micPaused
        } else if s.ladder == .l3 {
            banner = .relayOnly
        } else if s.ladder == .l1 {
            banner = .restarting
        } else if s.alone {
            banner = .alone
        } else {
            banner = nil
        }

        let captureUsable = s.live && (s.ladder == .l0 || s.ladder == .l1) && !s.onCall
        showResume = s.live && (s.ladder == .l2 || s.ladder == .l3)
        showMute = captureUsable
        showTalk = captureUsable && !s.muted

        switch banner {
        case .bluetoothOff, .relayOnly:
            tone = .danger
        case .onCall, .micPaused, .restarting, .alone:
            tone = .warn
        case nil:
            tone = s.live && !s.muted ? .live : .neutral
        }

        if !s.live {
            status = .lobby
        } else if s.muted || s.talk == .muted {
            status = .muted
        } else if s.pttLatched {
            status = .talkLatched
        } else {
            switch s.talk {
            case .talking: status = .talking
            case .blocked: status = .channelBusy
            case .idle, .muted: status = .listening
            }
        }

        if s.bluetoothOff {
            symbol = "antenna.radiowaves.left.and.right.slash"
        } else if s.onCall {
            symbol = "phone.fill"
        } else if s.ladder == .l2 || s.ladder == .l3 || s.muted || !s.live {
            symbol = "mic.slash.fill"
        } else if s.talk == .talking || s.pttLatched {
            symbol = "waveform"
        } else {
            symbol = "mic.fill"
        }
    }

    var bannerText: WidgetText? {
        switch banner {
        case .bluetoothOff: .bluetoothOff
        case .onCall: .onCall
        case .micPaused: .micPaused
        case .relayOnly: .relayOnly
        case .restarting: .restarting
        case .alone: .alone
        case nil: nil
        }
    }
}

/// What the Phase 0 Lab view shows.
struct LabPresentation: Equatable {
    let title: WidgetText
    let phase: WidgetText
    let elapsed: String
    let links: WidgetText
    let verdict: WidgetText?
    let tone: RidePresentation.Tone

    init(_ lab: LabStatus) {
        title = .labTitle(lab.bench)
        switch lab.phase {
        case .waiting: phase = .labWaiting; tone = .neutral
        case .running: phase = .labRunning; tone = .live
        case .finished: phase = .labFinished; tone = .neutral
        case .failed: phase = .labFailed; tone = .danger
        }
        let m = lab.elapsedS / 60
        let s = lab.elapsedS % 60
        elapsed = m >= 60
            ? String(format: "%d:%02d:%02d", m / 60, m % 60, s)
            : String(format: "%02d:%02d", m, s)
        links = .labLinks(lab.linksUp)
        switch lab.verdict {
        case "go": verdict = .labVerdictGo
        case "noGo": verdict = .labVerdictNoGo
        case "inconclusive": verdict = .labVerdictInconclusive
        default: verdict = nil
        }
    }
}
