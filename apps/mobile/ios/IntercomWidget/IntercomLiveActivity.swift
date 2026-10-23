import ActivityKit
import AppIntents
import MeshActivityShared
import SwiftUI
import WidgetKit

/// Lock screen and Dynamic Island views for `IntercomAttributes` (DESIGN §10.1.5).
/// Ride activities show status, speakers, the in-range count and the Mute, Talk (PTT
/// latch, latched state shown) and Resume mic buttons; Lab activities show the Phase 0
/// minimal run status for the T3 variant.
struct IntercomLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: IntercomAttributes.self) { context in
            LockScreenView(attributes: context.attributes, state: context.state)
                .activityBackgroundTint(WidgetPalette.background)
                .activitySystemActionForegroundColor(WidgetPalette.foreground)
        } dynamicIsland: { context in
            IslandBuilder.make(attributes: context.attributes, state: context.state)
        }
    }
}

// MARK: - Palette (ui-kit dark tokens, packages/ui-kit/src/styles/tokens.css)

enum WidgetPalette {
    static let background = Color(red: 0x1c / 255, green: 0x1c / 255, blue: 0x20 / 255)
    static let foreground = Color.white
    static let brand = Color(red: 0x8b / 255, green: 0x7b / 255, blue: 0xff / 255)
    static let talkActive = Color(red: 0x30 / 255, green: 0xd1 / 255, blue: 0x58 / 255)
    static let warn = Color(red: 0xff / 255, green: 0xd6 / 255, blue: 0x0a / 255)
    static let warnForeground = Color(red: 0x24 / 255, green: 0x1c / 255, blue: 0x00 / 255)
    static let danger = Color(red: 0xff / 255, green: 0x45 / 255, blue: 0x3a / 255)
    static let control = Color(red: 118 / 255, green: 118 / 255, blue: 128 / 255).opacity(0.3)

    static func tint(_ tone: RidePresentation.Tone) -> Color {
        switch tone {
        case .neutral: brand
        case .live: talkActive
        case .warn: warn
        case .danger: danger
        }
    }
}

// MARK: - Lock screen

struct LockScreenView: View {
    let attributes: IntercomAttributes
    let state: IntercomAttributes.ContentState

    var body: some View {
        if attributes.kind == .lab {
            LabStatusView(attributes: attributes, lab: IslandBuilder.labStatus(attributes, state), state: state)
                .padding(14)
        } else {
            RideLockScreenView(attributes: attributes, state: state)
                .padding(14)
        }
    }
}

struct RideLockScreenView: View {
    @Environment(\.locale) private var locale
    let attributes: IntercomAttributes
    let state: IntercomAttributes.ContentState

    var body: some View {
        let lang = WidgetLanguage.resolve(locale)
        let p = RidePresentation(state)
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .center, spacing: 10) {
                StatusGlyph(symbol: p.symbol, tone: p.tone)
                VStack(alignment: .leading, spacing: 1) {
                    Text(attributes.title.isEmpty ? WidgetText.appName.string(lang) : attributes.title)
                        .font(.headline)
                        .lineLimit(1)
                    Text(p.speakers?.string(lang) ?? p.status.string(lang))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                InRangeBadge(text: p.inRange.string(lang), count: state.inRange, total: state.total)
            }
            if let banner = p.bannerText {
                BannerView(text: banner.string(lang), tone: p.tone)
            }
            RideButtons(presentation: p, lang: lang)
        }
        .foregroundStyle(WidgetPalette.foreground)
    }
}

struct StatusGlyph: View {
    let symbol: String
    let tone: RidePresentation.Tone

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 18, weight: .semibold))
            .foregroundStyle(WidgetPalette.tint(tone))
            .frame(width: 36, height: 36)
            .background(Circle().fill(WidgetPalette.tint(tone).opacity(0.18)))
            .accessibilityHidden(true)
    }
}

struct InRangeBadge: View {
    let text: String
    let count: Int
    let total: Int

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "person.2.fill").font(.caption)
            Text("\(min(count, total))/\(total)")
                .font(.subheadline.monospacedDigit().weight(.semibold))
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Capsule().fill(WidgetPalette.control))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text)
    }
}

struct BannerView: View {
    let text: String
    let tone: RidePresentation.Tone

    var body: some View {
        Text(text)
            .font(.footnote.weight(.semibold))
            .lineLimit(2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .foregroundStyle(tone == .warn ? WidgetPalette.warnForeground : WidgetPalette.foreground)
            .background(RoundedRectangle(cornerRadius: 10).fill(WidgetPalette.tint(tone)))
    }
}

/// The intent buttons. Mute and Talk run in the app process in the background
/// (`LiveActivityIntent`, RT flag bits 1/3); Resume opens the app.
struct RideButtons: View {
    let presentation: RidePresentation
    let lang: WidgetLanguage

    var body: some View {
        let p = presentation
        if p.showMute || p.showTalk || p.showResume {
            HStack(spacing: 8) {
                if p.showResume {
                    Button(intent: ResumeMicIntent()) {
                        ButtonFace(
                            symbol: "mic.fill", title: WidgetText.buttonResume.string(lang),
                            fill: WidgetPalette.brand, foreground: Color.black)
                    }
                    .buttonStyle(.plain)
                }
                if p.showMute {
                    Button(intent: ToggleMuteIntent()) {
                        ButtonFace(
                            symbol: p.muted ? "mic.slash.fill" : "mic.fill",
                            title: (p.muted ? WidgetText.buttonUnmute : WidgetText.buttonMute).string(lang),
                            fill: p.muted ? WidgetPalette.danger : WidgetPalette.control,
                            foreground: WidgetPalette.foreground)
                    }
                    .buttonStyle(.plain)
                }
                if p.showTalk {
                    Button(intent: LatchTalkIntent()) {
                        ButtonFace(
                            symbol: p.latched ? "waveform" : "hand.tap.fill",
                            title: (p.latched ? WidgetText.buttonTalkOn : WidgetText.buttonTalk).string(lang),
                            fill: p.latched ? WidgetPalette.talkActive : WidgetPalette.control,
                            foreground: p.latched ? Color.black : WidgetPalette.foreground)
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(p.latched ? .isSelected : [])
                }
            }
        }
    }
}

struct ButtonFace: View {
    let symbol: String
    let title: String
    let fill: Color
    let foreground: Color

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol).accessibilityHidden(true)
            Text(title).lineLimit(1).minimumScaleFactor(0.8)
        }
        .font(.subheadline.weight(.semibold))
        .foregroundStyle(foreground)
        .frame(maxWidth: .infinity, minHeight: 40)
        .background(RoundedRectangle(cornerRadius: 12).fill(fill))
    }
}

// MARK: - Lab (Phase 0, T3 variant)

struct LabStatusView: View {
    @Environment(\.locale) private var locale
    let attributes: IntercomAttributes
    let lab: LabStatus
    let state: IntercomAttributes.ContentState

    var body: some View {
        let lang = WidgetLanguage.resolve(locale)
        let p = LabPresentation(lab)
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                StatusGlyph(symbol: "flask.fill", tone: p.tone)
                VStack(alignment: .leading, spacing: 1) {
                    Text(p.title.string(lang)).font(.headline).lineLimit(1)
                    Text(attributes.title).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 4)
                Text(p.elapsed).font(.title3.monospacedDigit().weight(.semibold))
            }
            HStack(spacing: 8) {
                LabChip(text: p.phase.string(lang), tone: p.tone)
                LabChip(text: p.links.string(lang), tone: .neutral)
                if let verdict = p.verdict {
                    LabChip(text: verdict.string(lang), tone: verdict == .labVerdictGo ? .live : .warn)
                }
                Spacer(minLength: 0)
            }
            if state.bluetoothOff {
                BannerView(text: WidgetText.bluetoothOff.string(lang), tone: .danger)
            }
        }
        .foregroundStyle(WidgetPalette.foreground)
    }
}

struct LabChip: View {
    let text: String
    let tone: RidePresentation.Tone

    var body: some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .lineLimit(1)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Capsule().fill(WidgetPalette.tint(tone).opacity(0.25)))
    }
}

// MARK: - Dynamic Island

enum IslandBuilder {
    /// Lab activities always render a status; before the first sample it is "waiting".
    static func labStatus(
        _ attributes: IntercomAttributes, _ state: IntercomAttributes.ContentState
    ) -> LabStatus {
        state.labStatus ?? LabStatus(bench: attributes.title, phase: .waiting)
    }

    static func make(
        attributes: IntercomAttributes, state: IntercomAttributes.ContentState
    ) -> DynamicIsland {
        if attributes.kind == .lab {
            return lab(attributes: attributes, lab: labStatus(attributes, state), state: state)
        }
        return ride(attributes: attributes, state: state)
    }

    private static func ride(
        attributes: IntercomAttributes, state: IntercomAttributes.ContentState
    ) -> DynamicIsland {
        let p = RidePresentation(state)
        return DynamicIsland {
            DynamicIslandExpandedRegion(.leading) {
                StatusGlyph(symbol: p.symbol, tone: p.tone)
            }
            DynamicIslandExpandedRegion(.trailing) {
                InRangeIslandText(state: state)
            }
            DynamicIslandExpandedRegion(.center) {
                IslandCenter(attributes: attributes, presentation: p)
            }
            DynamicIslandExpandedRegion(.bottom) {
                IslandBottom(presentation: p)
            }
        } compactLeading: {
            Image(systemName: p.symbol)
                .foregroundStyle(WidgetPalette.tint(p.tone))
        } compactTrailing: {
            Text("\(min(state.inRange, state.total))/\(state.total)")
                .font(.caption.monospacedDigit().weight(.semibold))
                .foregroundStyle(p.banner == nil ? WidgetPalette.foreground : WidgetPalette.tint(p.tone))
        } minimal: {
            Image(systemName: p.symbol)
                .foregroundStyle(WidgetPalette.tint(p.tone))
        }
        .keylineTint(WidgetPalette.tint(p.tone))
    }

    private static func lab(
        attributes: IntercomAttributes, lab: LabStatus, state: IntercomAttributes.ContentState
    ) -> DynamicIsland {
        let p = LabPresentation(lab)
        return DynamicIsland {
            DynamicIslandExpandedRegion(.bottom) {
                LabStatusView(attributes: attributes, lab: lab, state: state)
            }
        } compactLeading: {
            Image(systemName: "flask.fill").foregroundStyle(WidgetPalette.tint(p.tone))
        } compactTrailing: {
            Text(p.elapsed).font(.caption.monospacedDigit().weight(.semibold))
        } minimal: {
            Image(systemName: "flask.fill").foregroundStyle(WidgetPalette.tint(p.tone))
        }
        .keylineTint(WidgetPalette.tint(p.tone))
    }
}

struct InRangeIslandText: View {
    @Environment(\.locale) private var locale
    let state: IntercomAttributes.ContentState

    var body: some View {
        let text = RidePresentation(state).inRange.string(WidgetLanguage.resolve(locale))
        InRangeBadge(text: text, count: state.inRange, total: state.total)
    }
}

struct IslandCenter: View {
    @Environment(\.locale) private var locale
    let attributes: IntercomAttributes
    let presentation: RidePresentation

    var body: some View {
        let lang = WidgetLanguage.resolve(locale)
        VStack(spacing: 1) {
            Text(attributes.title.isEmpty ? WidgetText.appName.string(lang) : attributes.title)
                .font(.headline).lineLimit(1)
            Text(presentation.speakers?.string(lang) ?? presentation.status.string(lang))
                .font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
    }
}

struct IslandBottom: View {
    @Environment(\.locale) private var locale
    let presentation: RidePresentation

    var body: some View {
        let lang = WidgetLanguage.resolve(locale)
        VStack(spacing: 6) {
            if let banner = presentation.bannerText {
                BannerView(text: banner.string(lang), tone: presentation.tone)
            }
            RideButtons(presentation: presentation, lang: lang)
        }
    }
}

// MARK: - Previews (WP-23 acceptance: every ladder level, latched and unlatched Talk,
// Bluetooth off; plus alone, on a call, muted lobby and the Lab states)

enum PreviewFixtures {
    static let ride = IntercomAttributes(kind: .ride, title: "Sierra Norte")
    static let lab = IntercomAttributes(kind: .lab, title: "T3 · both background")

    static func riding(
        ladder: LadderLevel = .l0, latched: Bool = false, talk: TalkState = .idle,
        muted: Bool = false, speakers: [String] = [], inRange: Int = 4,
        alone: Bool = false, onCall: Bool = false, bluetoothOff: Bool = false
    ) -> IntercomAttributes.ContentState {
        IntercomAttributes.ContentState(
            live: true, muted: muted, talk: talk, pttLatched: latched, speakers: speakers,
            inRange: inRange, total: 5, ladder: ladder, onCall: onCall, alone: alone,
            bluetoothOff: bluetoothOff)
    }

    static let ladderStates: [IntercomAttributes.ContentState] =
        LadderLevel.allCases.map { riding(ladder: $0, speakers: $0 == .l0 ? ["Ana"] : []) }

    static let talkStates: [IntercomAttributes.ContentState] = [
        riding(latched: false, talk: .idle),
        riding(latched: true, talk: .talking),
        riding(talk: .blocked, speakers: ["Ana", "Luis"]),
    ]

    static let problemStates: [IntercomAttributes.ContentState] = [
        riding(inRange: 0, bluetoothOff: true),
        riding(inRange: 0, alone: true),
        riding(onCall: true),
        riding(muted: true),
        IntercomAttributes.ContentState(live: false, muted: true, inRange: 2, total: 5),
    ]

    static let labStates: [IntercomAttributes.ContentState] = [
        IntercomAttributes.ContentState(labStatus: LabStatus(bench: "T3", phase: .waiting)),
        IntercomAttributes.ContentState(
            labStatus: LabStatus(bench: "T3", phase: .running, elapsedS: 754, linksUp: 2)),
        IntercomAttributes.ContentState(
            labStatus: LabStatus(bench: "T3", phase: .finished, elapsedS: 3_725, linksUp: 1, verdict: "go")),
        IntercomAttributes.ContentState(
            bluetoothOff: true,
            labStatus: LabStatus(bench: "T3", phase: .failed, elapsedS: 41, verdict: "noGo")),
    ]
}

#Preview("Ladder L0-L3", as: .content, using: PreviewFixtures.ride) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.ladderStates[0]
    PreviewFixtures.ladderStates[1]
    PreviewFixtures.ladderStates[2]
    PreviewFixtures.ladderStates[3]
}

#Preview("Talk latch", as: .content, using: PreviewFixtures.ride) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.talkStates[0]
    PreviewFixtures.talkStates[1]
    PreviewFixtures.talkStates[2]
}

#Preview("Bluetooth off, alone, call, muted", as: .content, using: PreviewFixtures.ride) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.problemStates[0]
    PreviewFixtures.problemStates[1]
    PreviewFixtures.problemStates[2]
    PreviewFixtures.problemStates[3]
    PreviewFixtures.problemStates[4]
}

#Preview("Island expanded", as: .dynamicIsland(.expanded), using: PreviewFixtures.ride) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.talkStates[1]
    PreviewFixtures.ladderStates[2]
    PreviewFixtures.problemStates[0]
}

#Preview("Island compact", as: .dynamicIsland(.compact), using: PreviewFixtures.ride) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.ladderStates[0]
    PreviewFixtures.ladderStates[3]
}

#Preview("Island minimal", as: .dynamicIsland(.minimal), using: PreviewFixtures.ride) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.talkStates[1]
    PreviewFixtures.problemStates[0]
}

#Preview("Lab T3", as: .content, using: PreviewFixtures.lab) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.labStates[0]
    PreviewFixtures.labStates[1]
    PreviewFixtures.labStates[2]
    PreviewFixtures.labStates[3]
}

#Preview("Lab T3 island", as: .dynamicIsland(.expanded), using: PreviewFixtures.lab) {
    IntercomLiveActivity()
} contentStates: {
    PreviewFixtures.labStates[1]
}
