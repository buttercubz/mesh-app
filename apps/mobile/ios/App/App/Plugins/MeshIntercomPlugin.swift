import AVFoundation
import Capacitor
import CoreBluetooth
import CoreLocation
import Foundation
import MeshEngineKit
import MeshPlatform
import UIKit
import UserNotifications
import VisionKit
import os

/// App-local MeshIntercom bridge (contract C6, DESIGN §2.4, §11.1; WP-19).
///
/// Forwards the `@meshapp/capacitor-intercom` API to MeshEngineKit's `EngineHost`:
/// - `send` is a JSON pass-through of mesh-api `UiCommand` → `CommandResult` and rejects
///   the ★ gesture-gated commands with `useGestureCommand`;
/// - `gestureCommand` (and `goLive` / `resumeMic`) rejects with `notForeground` unless the
///   app is foreground-active, then lets the host do the OS-level work synchronously
///   (session + VPIO muted, location anchor, Live Activity) before the engine sees it;
/// - `ptt` writes the RT flags first (fast path, T8) and then sends `{type:"ptt"}`;
/// - UI events reach JS as `notifyListeners("event", ["json": …])`.
///
/// Rejections carry a mesh-api `ErrorCode` as both message and code
/// (`call.reject(code, code)`), which is what `rejectionCode()` of engine-api reads.
///
/// Threading: Capacitor calls the methods on the bridge's serial queue. UIKit work hops
/// to the main thread; the host's gesture path is called from the bridge queue, never
/// while the main thread waits on the engine.
@objc(MeshIntercomPlugin)
public final class MeshIntercomPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "MeshIntercomPlugin"
    public let jsName = "MeshIntercom"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "getInfo", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "getStatus", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "checkPermissions", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "requestPermissions", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "startEngine", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "gestureCommand", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "goLive", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "resumeMic", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "stopEngine", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "send", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "ptt", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setKeepScreenOn", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "scanInviteQr", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "shareFile", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "openAppSettings", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "requestBatteryExemption", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "setNotificationCatalog", returnType: CAPPluginReturnPromise),
        // C6 1.1: diagnostics flight recorder (ridelog/1).
        CAPPluginMethod(name: "diagStatus", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "diagList", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "diagExport", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "diagClear", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "diagSetEnabled", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "diagMark", returnType: CAPPluginReturnPromise),
    ]

    /// UI contract version this bridge speaks when the engine is not linked (mesh-api
    /// `API_VERSION`); with the engine, `getInfo` reports the engine's own version.
    static let apiVersion = "1.1"
    /// Wire protocol version (Appendix A).
    static let protocolVersion = 1

    static let log = Logger(subsystem: "com.convlink.voice", category: "bridge")
    /// T8 (touch → `mesh_rt_set_flags`) samples: JS touch time, native receipt, flag write.
    static let t8Log = Logger(subsystem: "com.convlink.voice", category: "t8")

    /// Set once in `load()` on the main thread, read-only afterwards.
    private(set) var host: EngineHost?
    /// Why `host` is nil (API major mismatch, engine refused the configuration).
    private(set) var hostFailure: EngineHostError?
    /// Main-thread objects (keep-screen-on switch, QR scanner, permission prompts).
    @MainActor var idleTimer: IdleTimerController?
    @MainActor private var scanner: InviteScanner?
    @MainActor private lazy var permissions = PermissionCenter()
    private var observers: [any NSObjectProtocol] = []
    private let state = OSAllocatedUnfairLock(initialState: BridgeState())
    /// Foreground-active check of the gesture-gated calls (a seam for the bridge tests).
    var foregroundCheck: () -> Bool = MeshIntercomPlugin.isForegroundActive

    struct BridgeState {
        /// An accepted `goLive` holds the ride (cleared by `stopEngine`).
        var live = false
        /// The app went to the background since the last foreground snapshot request.
        var backgrounded = false
    }

    // MARK: Launch

    override public func load() {
        MainActor.assumeIsolated { self.launch() }
    }

    /// Creates the engine host at launch (cheap: the engine thread starts lazily). The
    /// host's init also registers the Live Activity action handler (mute / latch /
    /// resume), which must exist before the system delivers an intent.
    @MainActor
    private func launch() {
        idleTimer = IdleTimerController()
        do {
            let h = try EngineHost(config: .current(), deps: try EngineHostDependencies.system())
            let box = WeakPlugin(self)
            h.setEventHandler { json in
                box.plugin?.notifyListeners("event", data: ["json": json])
            }
            h.setNotificationCatalog(BuiltinNotificationCatalog.current())
            host = h
        } catch let e as EngineHostError {
            hostFailure = e
            Self.log.error("engine host not created: \(String(describing: e), privacy: .public)")
        } catch {
            hostFailure = .engine("\(error)")
            Self.log.error("engine host not created: \(error.localizedDescription, privacy: .public)")
        }
        observeLifecycle()
    }

    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
    }

    // MARK: Info and status

    @objc func getInfo(_ call: CAPPluginCall) {
        let appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
        guard let host else {
            var api = Self.apiVersion
            if case .apiVersionMismatch(let engine) = hostFailure {
                api = "\(engine >> 8).\(engine & 0xFF)"
            }
            call.resolve([
                "platform": "ios",
                "engine": "platform-only",
                "apiVersion": api,
                "protocolVersion": Self.protocolVersion,
                "appVersion": appVersion ?? "0.0.0",
            ])
            return
        }
        let info = host.info()
        call.resolve([
            "platform": "ios",
            "engine": info.engine,
            "apiVersion": info.apiVersion,
            "protocolVersion": Int(info.protocolVersion),
            "appVersion": appVersion ?? info.appVersion,
        ])
    }

    @objc func getStatus(_ call: CAPPluginCall) {
        guard let host else {
            call.resolve(["engine": "idle", "live": false, "uiAttached": false])
            return
        }
        let info = host.info()
        let running = info.status == "running" || info.status == "audioCheck"
        let live = state.withLock { $0.live }
        call.resolve(["engine": running ? "running" : "idle", "live": running && live,
                      "uiAttached": info.uiAttached])
    }

    // MARK: Engine lifecycle

    /// Gesture required: radios + engine thread for a group lobby, a Lab run or the Demo
    /// ride. Audio stays off until a gesture command.
    @objc func startEngine(_ call: CAPPluginCall) {
        guard foregroundCheck() else { return reject(call, "notForeground") }
        guard let start = Self.sessionStart(call) else { return reject(call, "invalidParams") }
        guard let host = requireHost(call) else { return }
        do {
            try host.start(start)
            call.resolve()
        } catch {
            reject(call, Self.errorCode(error), detail: "\(error)")
        }
    }

    @objc func stopEngine(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        host.stop()
        state.withLock { $0.live = false }
        call.resolve()
    }

    /// `startEngine` options → `SessionStart` (`demo`, `lab.code` "0000"–"9999", or a
    /// 32-hex `groupId`); nil when none is valid.
    static func sessionStart(_ call: CAPPluginCall) -> SessionStart? {
        if call.getBool("demo") == true { return .demo }
        // Read through `options`: a nested object is an NSDictionary from the bridge's JSON
        // (or a Swift dictionary in tests), which `getObject` does not always bridge.
        if let lab = call.options?["lab"] as? [AnyHashable: Any] {
            guard let code = lab["code"] as? String else { return nil }
            return labCode(code).map(SessionStart.lab(code:))
        }
        if let gid = call.getString("groupId") { return SessionStart.group(hex: gid) }
        return nil
    }

    /// A Lab code: 1 to 4 ASCII digits (§12.1).
    static func labCode(_ s: String) -> UInt16? {
        guard (1...4).contains(s.count), s.utf8.allSatisfy({ (0x30...0x39).contains($0) }) else { return nil }
        return UInt16(s)
    }
}

// MARK: - Commands

extension MeshIntercomPlugin {
    /// ★ commands (goLive, resumeMic, labStart with audio ≠ off, audioCheck start). Refused
    /// with `notForeground` unless the app is foreground-active; the host then performs
    /// the gesture-bound OS work synchronously and only afterwards forwards the command.
    @objc func gestureCommand(_ call: CAPPluginCall) {
        guard let json = call.getString("command") else { return reject(call, "invalidParams") }
        runGesture(call, json: json) { result in call.resolve(["result": result]) }
    }

    /// Sugar for `gestureCommand({type:"goLive"})`; a refusal rejects with its code.
    @objc func goLive(_ call: CAPPluginCall) {
        runGesture(call, json: #"{"type":"goLive"}"#) { result in
            self.resolveSugar(call, result: result)
        }
    }

    /// Sugar for `gestureCommand({type:"resumeMic"})`; a refusal rejects with its code.
    @objc func resumeMic(_ call: CAPPluginCall) {
        runGesture(call, json: #"{"type":"resumeMic"}"#) { result in
            self.resolveSugar(call, result: result)
        }
    }

    /// JSON pass-through of mesh-api `UiCommand` → `CommandResult`. The ★ commands are
    /// rejected with `useGestureCommand`: they must come from a tap, through
    /// `gestureCommand`, so the OS work happens inside the gesture.
    @objc func send(_ call: CAPPluginCall) {
        guard let json = call.getString("command") else { return reject(call, "invalidParams") }
        guard !GestureCommand.requiresGesture(json) else { return reject(call, "useGestureCommand") }
        guard let host = requireHost(call) else { return }
        let t0 = Self.nowMs()
        let result = host.send(json)
        Self.noteCommand(host, json: json, result: result, gesture: false, sinceMs: t0)
        call.resolve(["result": result])
    }

    /// PTT fast path (§3.6): the RT flag bit is written first, with no core round trip,
    /// then `{type:"ptt"}` follows for the talk FSM. The native receipt time and the flag
    /// write time are logged with the JS touch time for bench T8.
    @objc func ptt(_ call: CAPPluginCall) {
        let receivedMs = Self.nowMs()
        guard let host = requireHost(call) else { return }
        guard let down = call.getBool("down") else { return reject(call, "invalidParams") }
        let tUiMs = call.getDouble("tUiMs")
        if let t = tUiMs, !t.isFinite { return reject(call, "invalidParams") }
        let status = host.info().status
        guard status == "running" || status == "audioCheck" else { return reject(call, "engineNotRunning") }
        LiveActivityRtActions.setPttDown(down)
        let flagsMs = Self.nowMs()
        // The command's answer is not surfaced: talk state arrives in snapshots and
        // refusals as `talkDenied` events (same as the web implementation).
        _ = host.send(#"{"type":"ptt","down":\#(down)}"#)
        let ui = tUiMs.map { String(format: "%.1f", $0) } ?? "-"
        Self.t8Log.info("ptt down=\(down) tUiMs=\(ui, privacy: .public) rxMs=\(receivedMs, format: .fixed(precision: 1)) flagsMs=\(flagsMs, format: .fixed(precision: 1))")
        call.resolve()
    }

    /// Localised notification texts from the UI catalogs (`notices` + `errors` of the
    /// current language, i18next `{{param}}` templates). Until the UI sends them, the
    /// built-in fallback covers the capture-lost notification.
    @objc func setNotificationCatalog(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        guard let title = call.getString("title"),
              let raw = call.options?["templates"] as? [AnyHashable: Any] else {
            return reject(call, "invalidParams")
        }
        var templates: [String: String] = [:]
        for (k, v) in raw {
            if let key = k as? String, let s = v as? String { templates[key] = s }
        }
        host.setNotificationCatalog(NotificationCatalog(title: title, templates: templates))
        call.resolve()
    }

    // MARK: Gesture plumbing

    /// Foreground first (synchronously, inside the tap), then the host's gesture path.
    private func runGesture(_ call: CAPPluginCall, json: String, done: (String) -> Void) {
        guard foregroundCheck() else { return reject(call, "notForeground") }
        guard let host = requireHost(call) else { return }
        let t0 = Self.nowMs()
        do {
            let result = try host.gestureCommand(json)
            Self.noteCommand(host, json: json, result: result, gesture: true, sinceMs: t0)
            if Self.isOk(result), Self.commandType(json) == "goLive" {
                state.withLock { $0.live = true }
            }
            done(result)
        } catch {
            reject(call, Self.errorCode(error), detail: "\(error)")
        }
    }

    private func resolveSugar(_ call: CAPPluginCall, result: String) {
        if Self.isOk(result) {
            call.resolve()
        } else {
            reject(call, Self.resultCode(result) ?? "internal")
        }
    }

    // MARK: Helpers

    /// The host, or a rejection explaining why the engine is not available.
    func requireHost(_ call: CAPPluginCall) -> EngineHost? {
        if let host { return host }
        switch hostFailure {
        case .apiVersionMismatch?: reject(call, "apiVersionMismatch")
        case .engine?: reject(call, "engineCrashed")
        default: reject(call, "notSupported")
        }
        return nil
    }

    /// `call.reject(code, code)`: the contract `ErrorCode` as message and code.
    func reject(_ call: CAPPluginCall, _ code: String, detail: String? = nil) {
        if let detail { Self.log.notice("\(call.methodName, privacy: .public) rejected: \(code, privacy: .public) (\(detail, privacy: .public))") }
        call.reject(code, code)
    }

    /// `UIApplication.applicationState == .active`, read on the main thread.
    static func isForegroundActive() -> Bool {
        onMain { UIApplication.shared.applicationState == .active }
    }

    /// Runs `body` on the main thread and waits (inline when already there).
    static func onMain<T>(_ body: @MainActor () -> T) -> T {
        if Thread.isMainThread { return MainActor.assumeIsolated(body) }
        return DispatchQueue.main.sync { MainActor.assumeIsolated(body) }
    }

    /// `EngineHostError` → mesh-api `ErrorCode`.
    static func errorCode(_ error: any Error) -> String {
        guard let e = error as? EngineHostError else { return "internal" }
        switch e {
        case .notForeground: return "notForeground"
        case .apiVersionMismatch: return "apiVersionMismatch"
        case .engine(let detail):
            // `FfiError` as described by Swift: "UnknownGroup", "InvalidStart(detail: …)".
            if detail.hasPrefix("UnknownGroup") { return "unknownGroup" }
            if detail.hasPrefix("InvalidStart") || detail.hasPrefix("InvalidConfig") { return "invalidParams" }
            if detail.hasPrefix("DemoUnavailable") { return "notSupported" }
            if detail.hasPrefix("Crashed") { return "engineCrashed" }
            return "internal"
        }
    }

    static func object(_ json: String) -> [String: Any]? {
        guard let d = json.data(using: .utf8) else { return nil }
        return (try? JSONSerialization.jsonObject(with: d)) as? [String: Any]
    }

    static func isOk(_ result: String) -> Bool { object(result)?["ok"] as? Bool == true }
    static func resultCode(_ result: String) -> String? { object(result)?["code"] as? String }
    static func commandType(_ json: String) -> String? { object(json)?["type"] as? String }

    static func nowMs() -> Double { Date().timeIntervalSince1970 * 1000 }
}

/// Weak reference for the host's `@Sendable` event callback.
final class WeakPlugin: @unchecked Sendable {
    // Read from the engine's event thread; `notifyListeners` hops to the main thread itself.
    weak var plugin: MeshIntercomPlugin?
    init(_ p: MeshIntercomPlugin) { plugin = p }
}

// MARK: - Screen, QR, sharing, settings

extension MeshIntercomPlugin {
    /// `isIdleTimerDisabled`; `IdleTimerController` resets it on every background
    /// transition, so the visible screen must ask again when it returns.
    @objc func setKeepScreenOn(_ call: CAPPluginCall) {
        guard let on = call.getBool("on") else { return reject(call, "invalidParams") }
        Self.onMain { self.idleTimer?.setKeepScreenOn(on) }
        call.resolve()
    }

    /// In-app invite scanner (VisionKit, QR only, §8.1). The payload is returned verbatim
    /// for `parseInvite`; nothing is stored here.
    @objc func scanInviteQr(_ call: CAPPluginCall) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated { self.presentScanner(call) }
        }
    }

    @MainActor
    private func presentScanner(_ call: CAPPluginCall) {
        guard InviteScanner.isSupported else { return reject(call, "notSupported", detail: "no DataScanner") }
        guard let presenter = bridge?.viewController else { return reject(call, "internal") }
        scanner?.cancel()
        Task { @MainActor in
            if AVCaptureDevice.authorizationStatus(for: .video) == .notDetermined {
                _ = await AVCaptureDevice.requestAccess(for: .video)
            }
            let scan = InviteScanner()
            self.scanner = scan
            // The plugin lives as long as the bridge: strong captures are fine here.
            scan.present(from: presenter) { result in
                if self.scanner === scan { self.scanner = nil }
                switch result {
                case .payload(let url): call.resolve(["url": url, "cancelled": false])
                case .cancelled: call.resolve(["cancelled": true])
                case .unsupported: self.reject(call, "notSupported", detail: "scanner unsupported")
                case .unavailable: self.reject(call, "notSupported", detail: "camera unavailable or denied")
                }
            }
            ScannerChrome.addCloseButton(to: presenter.presentedViewController) { [weak scan] in scan?.cancel() }
        }
    }

    /// Lab export through the share sheet (AirDrop to the Mac, §12.3). Only files of the
    /// Lab directory (`Documents/lab`) can be shared; `path` may be the bare file name the
    /// core reports or carry a directory prefix.
    @objc func shareFile(_ call: CAPPluginCall) {
        guard let path = call.getString("path"), let url = Self.labFileURL(path) else {
            return reject(call, "invalidParams", detail: "path")
        }
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let presenter = self.bridge?.viewController else { return self.reject(call, "internal") }
                let sheet = UIActivityViewController(activityItems: [url], applicationActivities: nil)
                sheet.popoverPresentationController?.sourceView = presenter.view
                sheet.completionWithItemsHandler = { _, _, _, error in
                    if let error {
                        self.reject(call, "storageFailed", detail: error.localizedDescription)
                    } else {
                        // Dismissed or completed: the run stays on the phone either way.
                        call.resolve()
                    }
                }
                presenter.present(sheet, animated: true)
            }
        }
    }

    /// The existing Lab file named by `path`, or nil (unknown, escaping the directory).
    static func labFileURL(_ path: String) -> URL? {
        let name = (path as NSString).lastPathComponent
        guard LabFileStore.validate(name), name.hasSuffix(".jsonl") || name.hasSuffix(".csv"),
              let dir = try? LabFileStore().directory else { return nil }
        let url = dir.appending(path: name, directoryHint: .notDirectory)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    @objc func openAppSettings(_ call: CAPPluginCall) {
        DispatchQueue.main.async {
            guard let url = URL(string: UIApplication.openSettingsURLString) else { return call.resolve() }
            UIApplication.shared.open(url) { _ in call.resolve() }
        }
    }

    /// Android only; iOS has no battery-optimisation exemption to ask for.
    @objc func requestBatteryExemption(_ call: CAPPluginCall) {
        call.resolve(["granted": true])
    }
}

// MARK: - Lifecycle

extension MeshIntercomPlugin {
    /// Background: detach the UI (snapshots stop; WebView timers are not trusted) and close
    /// an open scanner. Foreground-active after a background stay: attach and request a
    /// full snapshot (§11.1). The host follows the app state too; both are idempotent.
    @MainActor
    func observeLifecycle() {
        let center = NotificationCenter.default
        let box = WeakPlugin(self)
        observers.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification,
                                            object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { box.plugin?.didEnterBackground() }
        })
        observers.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification,
                                            object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { box.plugin?.didBecomeActive() }
        })
    }

    @MainActor
    func didEnterBackground() {
        state.withLock { $0.backgrounded = true }
        idleTimer?.appDidEnterBackground()
        scanner?.cancel()
        host?.setUiAttached(false)
    }

    @MainActor
    func didBecomeActive() {
        let returning = state.withLock { s -> Bool in
            defer { s.backgrounded = false }
            return s.backgrounded
        }
        guard returning, let host else { return }
        host.setUiAttached(true)
        let status = host.info().status
        guard status == "running" || status == "audioCheck" else { return }
        // Off the main thread: the engine answers on its own thread.
        DispatchQueue.global(qos: .userInitiated).async {
            _ = host.send(#"{"type":"requestSnapshot"}"#)
        }
    }
}

// MARK: - Permissions

extension MeshIntercomPlugin {
    /// The six intercom permissions, in the onboarding order (C6 `IntercomPermission`).
    static let permissionNames = ["microphone", "bluetooth", "notifications", "camera", "location", "nearbyWifi"]

    @objc override public func checkPermissions(_ call: CAPPluginCall) {
        Task { @MainActor in
            let out = await self.permissions.statuses(asking: [])
            self.notePermissions(out, asked: [])
            call.resolve(out)
        }
    }

    /// Asks for the requested permissions one prompt at a time (all six when none given).
    @objc override public func requestPermissions(_ call: CAPPluginCall) {
        let wanted = call.getArray("permissions", String.self) ?? Self.permissionNames
        guard wanted.allSatisfy(Self.permissionNames.contains) else { return reject(call, "invalidParams") }
        Task { @MainActor in
            let out = await self.permissions.statuses(asking: Set(wanted))
            self.notePermissions(out, asked: Set(wanted))
            call.resolve(out)
        }
    }

    /// The exact API behind each permission state (diagnostics `perm` notes).
    static let permissionApis = [
        "microphone": "AVAudioApplication.recordPermission",
        "bluetooth": "CBManager.authorization",
        "notifications": "UNNotificationSettings.authorizationStatus",
        "camera": "AVCaptureDevice.authorizationStatus",
        "location": "CLLocationManager.authorizationStatus",
        "nearbyWifi": "none",
    ]

    /// One `perm` diagnostics note per permission answered to the UI.
    func notePermissions(_ states: [String: String], asked: Set<String>) {
        guard let host else { return }
        for name in Self.permissionNames {
            guard let state = states[name] else { continue }
            host.diagNote(.perm(name: name, state: state, via: asked.contains(name) ? "request" : "check",
                                api: Self.permissionApis[name] ?? "unknown"))
        }
    }
}

/// Runtime permission state and prompts (main thread). States use Capacitor's
/// `PermissionState` strings: "granted", "denied", "prompt".
@MainActor
final class PermissionCenter: NSObject, CBCentralManagerDelegate, CLLocationManagerDelegate {
    private var bluetoothManager: CBCentralManager?
    private var bluetoothWaiters: [CheckedContinuation<Void, Never>] = []
    private let locationManager = CLLocationManager()
    private var locationWaiters: [CheckedContinuation<Void, Never>] = []

    override init() {
        super.init()
        locationManager.delegate = self
    }

    /// Every permission's state, prompting first for those in `asking` that are still
    /// undetermined (in declaration order).
    func statuses(asking: Set<String>) async -> [String: String] {
        var out: [String: String] = [:]
        for name in MeshIntercomPlugin.permissionNames {
            var s = await status(name)
            if s == "prompt", asking.contains(name) {
                await request(name)
                s = await status(name)
            }
            out[name] = s
        }
        return out
    }

    func status(_ name: String) async -> String {
        switch name {
        case "microphone":
            switch AVAudioApplication.shared.recordPermission {
            case .granted: return "granted"
            case .undetermined: return "prompt"
            default: return "denied"
            }
        case "bluetooth":
            switch CBManager.authorization {
            case .allowedAlways: return "granted"
            case .notDetermined: return "prompt"
            default: return "denied"
            }
        case "notifications":
            switch await UNUserNotificationCenter.current().notificationSettings().authorizationStatus {
            case .authorized, .provisional, .ephemeral: return "granted"
            case .notDetermined: return "prompt"
            default: return "denied"
            }
        case "camera":
            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .authorized: return "granted"
            case .notDetermined: return "prompt"
            default: return "denied"
            }
        case "location":
            switch locationManager.authorizationStatus {
            case .authorizedWhenInUse, .authorizedAlways: return "granted"
            case .notDetermined: return "prompt"
            default: return "denied"
            }
        default:
            // nearbyWifi: no runtime permission on iOS (Wi-Fi Aware pairs through the
            // system UI, Phase 4).
            return "granted"
        }
    }

    private func request(_ name: String) async {
        switch name {
        case "microphone":
            _ = await AVAudioApplication.requestRecordPermission()
        case "bluetooth":
            // Creating a manager raises the prompt; the first state update follows the answer.
            await withCheckedContinuation { c in
                bluetoothWaiters.append(c)
                if bluetoothManager == nil {
                    bluetoothManager = CBCentralManager(delegate: self, queue: .main,
                                                        options: [CBCentralManagerOptionShowPowerAlertKey: false])
                }
            }
        case "notifications":
            // Time-sensitive delivery comes from the entitlement, not an option here.
            _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound])
        case "camera":
            _ = await AVCaptureDevice.requestAccess(for: .video)
        case "location":
            await withCheckedContinuation { c in
                locationWaiters.append(c)
                locationManager.requestWhenInUseAuthorization()
            }
        default:
            break
        }
    }

    nonisolated func centralManagerDidUpdateState(_ central: CBCentralManager) {
        MainActor.assumeIsolated {
            guard CBManager.authorization != .notDetermined else { return }
            let waiters = bluetoothWaiters
            bluetoothWaiters = []
            waiters.forEach { $0.resume() }
        }
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        MainActor.assumeIsolated {
            guard manager.authorizationStatus != .notDetermined else { return }
            let waiters = locationWaiters
            locationWaiters = []
            waiters.forEach { $0.resume() }
        }
    }
}

// MARK: - Scanner chrome

/// A close button over VisionKit's scanner, which has no cancel control of its own.
@MainActor
enum ScannerChrome {
    static func addCloseButton(to presented: UIViewController?, onClose: @escaping () -> Void) {
        guard let scanner = presented as? DataScannerViewController else { return }
        var config = UIButton.Configuration.filled()
        config.image = UIImage(systemName: "xmark")
        config.cornerStyle = .capsule
        config.baseBackgroundColor = UIColor.black.withAlphaComponent(0.6)
        config.baseForegroundColor = .white
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in onClose() })
        button.accessibilityLabel = BuiltinNotificationCatalog.isSpanish ? "Cancelar" : "Cancel"
        button.translatesAutoresizingMaskIntoConstraints = false
        let overlay = scanner.overlayContainerView
        overlay.addSubview(button)
        NSLayoutConstraint.activate([
            button.topAnchor.constraint(equalTo: overlay.safeAreaLayoutGuide.topAnchor, constant: 12),
            button.trailingAnchor.constraint(equalTo: overlay.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            button.widthAnchor.constraint(equalToConstant: 44),
            button.heightAnchor.constraint(equalToConstant: 44),
        ])
    }
}

// MARK: - Notification fallback

/// Built-in text for the capture-lost notification until the UI hands its catalogs over
/// (`setNotificationCatalog`). Same wording as the UI's `notices` catalog (ES/EN).
enum BuiltinNotificationCatalog {
    static var isSpanish: Bool {
        (Locale.preferredLanguages.first ?? "en").lowercased().hasPrefix("es")
    }

    static func current() -> NotificationCatalog {
        NotificationCatalog(title: "convLink Voice", templates: [
            "captureLost": isSpanish
                ? "Micrófono en pausa. Toque Reanudar para volver a hablar."
                : "Microphone paused. Tap Resume to talk again.",
        ])
    }
}

// MARK: - Diagnostics (C6 1.1, ridelog/1)

extension MeshIntercomPlugin {
    /// A `cmd` diagnostics note for a command the UI sent: id, gesture, answer code and
    /// round trip only (never the parameters, which may hold names or invites).
    static func noteCommand(_ host: EngineHost, json: String, result: String, gesture: Bool, sinceMs: Double) {
        guard let type = commandType(json), type != "requestSnapshot" else { return }
        var fields: [String: Any] = ["name": type, "gesture": gesture, "ok": isOk(result),
                                     "ms": Int(max(0, nowMs() - sinceMs))]
        fields["err"] = resultCode(result) ?? NSNull()
        guard let data = try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys]) else { return }
        host.diagUiNote(ev: "cmd", fieldsJson: String(decoding: data, as: UTF8.self))
    }

    @objc func diagStatus(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        let s = host.diagStatus()
        call.resolve([
            "enabled": s.enabled,
            "sessionId": s.sessionId ?? NSNull(),
            "rideKey": s.rideKey ?? NSNull(),
            "node": s.node.map { Int($0) as Any } ?? NSNull(),
            "bytes": s.bytes,
            "sessions": s.sessions,
            "oldestMs": s.oldest.map { $0.timeIntervalSince1970 * 1000 as Any } ?? NSNull(),
        ])
    }

    @objc func diagList(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        let sessions = host.diagList().map { s -> [String: Any] in
            ["sessionId": s.sessionId, "bytes": s.bytes, "segments": s.segments.count,
             "modifiedMs": s.modified.timeIntervalSince1970 * 1000]
        }
        call.resolve(["sessions": sessions])
    }

    /// Packs the last session (or all) into a zip and opens the share sheet with it.
    @objc func diagExport(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        guard let raw = call.getString("scope"), let scope = DiagFileStore.ExportScope(rawValue: raw) else {
            return reject(call, "invalidParams", detail: "scope")
        }
        let sessions = scope == .last ? min(1, host.diagList().count) : host.diagList().count
        let url: URL
        do {
            url = try host.diagExport(scope: scope)
        } catch {
            return reject(call, "storageFailed", detail: "\(error)")
        }
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let presenter = self.bridge?.viewController else { return self.reject(call, "internal") }
                let sheet = UIActivityViewController(activityItems: [url], applicationActivities: nil)
                sheet.popoverPresentationController?.sourceView = presenter.view
                sheet.completionWithItemsHandler = { _, _, _, error in
                    if let error {
                        self.reject(call, "storageFailed", detail: error.localizedDescription)
                    } else {
                        // Shared or dismissed: the files stay on the phone either way.
                        call.resolve(["path": url.path, "mime": "application/zip", "sessions": sessions])
                    }
                }
                presenter.present(sheet, animated: true)
            }
        }
    }

    @objc func diagClear(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        host.diagClear()
        call.resolve()
    }

    @objc func diagSetEnabled(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        guard let on = call.getBool("enabled") else { return reject(call, "invalidParams", detail: "enabled") }
        host.diagSetEnabled(on)
        call.resolve()
    }

    @objc func diagMark(_ call: CAPPluginCall) {
        guard let host = requireHost(call) else { return }
        guard let label = call.getString("label"), host.diagMark(label) else {
            return reject(call, "invalidParams", detail: "label")
        }
        call.resolve()
    }
}
