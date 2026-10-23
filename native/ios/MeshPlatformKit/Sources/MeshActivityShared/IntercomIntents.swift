import AppIntents
import Foundation

// Titles resolve from the `MeshActivity` strings table of the main bundle (AppIntents
// requires the main bundle). The app and widget targets ship `MeshActivity.strings` in
// en.lproj and es.lproj with the keys used below.

/// Lock-screen / Dynamic Island mute toggle. Runs in the app process and only writes the
/// hard-mute RT flag through the registered handler (DESIGN §10.1.5).
public struct ToggleMuteIntent: LiveActivityIntent {
    public static let title = LocalizedStringResource(
        "intent.toggleMute.title", defaultValue: "Mute or unmute", table: "MeshActivity")
    public static let isDiscoverable = false
    public static let supportedModes: IntentModes = .background

    public init() {}

    public func perform() async throws -> some IntentResult {
        guard LiveActivityActionHandler.handle(.toggleMute) else {
            throw LiveActivityHandlerMissing(action: .toggleMute)
        }
        return .result()
    }
}

/// Lock-screen PTT latch. Runs in the app process and only toggles RT flag bit 3 through
/// the registered handler, so it is legal while the phone is locked (Appendix D.2 #11).
public struct LatchTalkIntent: LiveActivityIntent {
    public static let title = LocalizedStringResource(
        "intent.latchTalk.title", defaultValue: "Latch talk", table: "MeshActivity")
    public static let isDiscoverable = false
    public static let supportedModes: IntentModes = .background

    public init() {}

    public func perform() async throws -> some IntentResult {
        guard LiveActivityActionHandler.handle(.latchTalk) else {
            throw LiveActivityHandlerMissing(action: .latchTalk)
        }
        return .result()
    }
}

/// "Mic paused — Resume": opens the app, because capture cannot restart in the background
/// (DESIGN §10.1.4 CAPTURE_LOST step 5). The handler forwards `resumeMic` once the app is
/// in the foreground.
public struct ResumeMicIntent: LiveActivityIntent {
    public static let title = LocalizedStringResource(
        "intent.resumeMic.title", defaultValue: "Resume microphone", table: "MeshActivity")
    public static let isDiscoverable = false
    public static let supportedModes: IntentModes = .foreground(.immediate)

    public init() {}

    public func perform() async throws -> some IntentResult {
        guard LiveActivityActionHandler.handle(.resumeMic) else {
            throw LiveActivityHandlerMissing(action: .resumeMic)
        }
        return .result()
    }
}
