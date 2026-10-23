@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform
import Security
import UIKit

/// Static facts about this app and device for `FfiEngineConfig` and `info()`.
public struct EngineHostConfig: Sendable {
    /// Build number as u16 (`CFBundleVersion`).
    public var appVersion: UInt16
    /// "0.1.0 (12)" for labrun/1 `device.app`.
    public var appLabel: String
    /// "iPhone15,2".
    public var deviceModel: String
    /// "iOS 27.0".
    public var deviceOs: String
    /// 32 bytes; nil draws them from the system CSPRNG.
    public var rngSeed: Data?
    /// Platform sample period (DESIGN §12.1: 1 Hz).
    public var sampleInterval: TimeInterval
    /// Detach the UI (snapshot gate) on background and attach it on foreground. The bridge
    /// may also call `setUiAttached`; both are idempotent.
    public var followAppStateForUi: Bool
    /// Install the Live Activity action handler (mute / latch / resume) at init. Only one
    /// host per process may do this; tests with several hosts turn it off.
    public var registerActivityActions: Bool

    public init(appVersion: UInt16, appLabel: String, deviceModel: String, deviceOs: String,
                rngSeed: Data? = nil, sampleInterval: TimeInterval = 1,
                followAppStateForUi: Bool = true, registerActivityActions: Bool = true) {
        self.appVersion = appVersion
        self.appLabel = appLabel
        self.deviceModel = deviceModel
        self.deviceOs = deviceOs
        self.rngSeed = rngSeed
        self.sampleInterval = sampleInterval
        self.followAppStateForUi = followAppStateForUi
        self.registerActivityActions = registerActivityActions
    }

    /// Values read from the main bundle and the device.
    @MainActor
    public static func current() -> EngineHostConfig {
        let info = Bundle.main.infoDictionary ?? [:]
        let build = info["CFBundleVersion"] as? String ?? "0"
        let short = info["CFBundleShortVersionString"] as? String ?? "0.0.0"
        return EngineHostConfig(appVersion: UInt16(clamping: Int(build) ?? 0),
                                appLabel: "\(short) (\(build))",
                                deviceModel: machineIdentifier(),
                                deviceOs: "iOS \(UIDevice.current.systemVersion)")
    }

    /// `utsname.machine` ("iPhone15,2"; the simulator reports its host's model id).
    public static func machineIdentifier() -> String {
        var u = utsname()
        uname(&u)
        return withUnsafeBytes(of: &u.machine) { raw in
            String(decoding: raw.prefix { $0 != 0 }, as: UTF8.self)
        }
    }

    /// 32 bytes from `SecRandomCopyBytes` (the engine's CSPRNG seed on device, §3.2).
    static func randomSeed() -> Data {
        var bytes = [UInt8](repeating: 0, count: 32)
        while true {
            let st = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
            if st == errSecSuccess, bytes.contains(where: { $0 != 0 }) { return Data(bytes) }
        }
    }
}

/// The OS services an `EngineHost` uses. Defaults are the production implementations;
/// tests replace them with fakes (fake drivers, a recording audio control, an in-memory
/// keychain, a temporary Lab directory).
@MainActor
public struct EngineHostDependencies {
    public var engineFactory: EngineFactory
    public var radioFactory: RadioStackFactory
    public var bleConstants: BleConstants?
    public var audio: any AudioControl
    public var secureStore: SecureStore
    public var labStore: LabFileStore
    /// ridelog/1 diagnostics files (nil: RAM ring only).
    public var diagStore: DiagFileStore?
    public var foreground: ForegroundState
    /// Reads the real foreground state inside a gesture (also refreshes `foreground`).
    public var foregroundCheck: @Sendable () -> Bool
    public var calls: any CallStatePort
    public var promptFactory: PromptRendererFactory
    public var activity: IntercomActivityController
    public var notifier: ResumeNotifier
    public var location: KeepAliveLocation
    public var device: DeviceSampler
    public var lifecycle: LifecycleMonitor
    public var logSink: (@Sendable (FfiLogLevel, String) -> Void)?

    public init(engineFactory: @escaping EngineFactory = { cfg, hooks in try MeshEngine(cfg: cfg, hooks: hooks) },
                radioFactory: @escaping RadioStackFactory = { BleRadioStack($0) },
                bleConstants: BleConstants? = nil,
                audio: any AudioControl = PlatformAudio(),
                secureStore: SecureStore = SecureStore(),
                labStore: LabFileStore,
                diagStore: DiagFileStore? = nil,
                foreground: ForegroundState? = nil,
                foregroundCheck: (@Sendable () -> Bool)? = nil,
                calls: any CallStatePort = SystemCallState(),
                promptFactory: @escaping PromptRendererFactory = { PromptRenderer(onPcm: $0, onFailure: $1) },
                activity: IntercomActivityController = IntercomActivityController(),
                notifier: ResumeNotifier = ResumeNotifier(),
                location: KeepAliveLocation = KeepAliveLocation(),
                device: DeviceSampler = DeviceSampler(),
                lifecycle: LifecycleMonitor = LifecycleMonitor(),
                logSink: (@Sendable (FfiLogLevel, String) -> Void)? = nil) {
        let fg = foreground ?? ForegroundState.system()
        self.engineFactory = engineFactory
        self.radioFactory = radioFactory
        self.bleConstants = bleConstants
        self.audio = audio
        self.secureStore = secureStore
        self.labStore = labStore
        self.diagStore = diagStore
        self.foreground = fg
        self.foregroundCheck = foregroundCheck ?? {
            let active = MainHop.sync { UIApplication.shared.applicationState == .active }
            fg.set(active)
            return active
        }
        self.calls = calls
        self.promptFactory = promptFactory
        self.activity = activity
        self.notifier = notifier
        self.location = location
        self.device = device
        self.lifecycle = lifecycle
        self.logSink = logSink
    }

    /// Production dependencies (Lab files in Documents/lab, diagnostics in
    /// Application Support/diag; a diagnostics store that cannot be created leaves the
    /// RAM ring only and never blocks the engine).
    public static func system() throws -> EngineHostDependencies {
        EngineHostDependencies(labStore: try LabFileStore(), diagStore: try? DiagFileStore())
    }
}

/// Errors of `EngineHost`.
public enum EngineHostError: Error, Equatable, Sendable {
    /// `gestureCommand` outside a foreground-active app (§3.2).
    case notForeground
    /// The engine's UI contract major differs from this bridge's (§11.1).
    case apiVersionMismatch(engine: UInt16)
    /// The engine refused the configuration or the start.
    case engine(String)
}
