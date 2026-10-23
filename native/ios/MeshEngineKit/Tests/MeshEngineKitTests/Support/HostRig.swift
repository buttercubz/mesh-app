@preconcurrency import MeshCoreBindings
import Foundation
@testable import MeshEngineKit
import MeshPlatform
import Synchronization
import XCTest

/// The real Rust engine with every UI command recorded in the shared sequence log.
final class RecordingEngine: MeshEngineProtocol, @unchecked Sendable {
    let inner: MeshEngine
    let log: SequenceLog

    init(_ inner: MeshEngine, log: SequenceLog) {
        self.inner = inner
        self.log = log
    }

    func uiCommand(json: String) -> String {
        let type = CommandJSON.object(json)?["type"] as? String ?? "?"
        log.add("engine.uiCommand:\(type)")
        return inner.uiCommand(json: json)
    }

    func onAudioIo(s: FfiAudioIoState) {
        log.add("engine.onAudioIo:\(s)")
        inner.onAudioIo(s: s)
    }

    func onAudioRoute(r: FfiAudioRoute) { inner.onAudioRoute(r: r) }
    func onConnectFailed(discId: UInt32, reason: FfiLinkDownReason) { inner.onConnectFailed(discId: discId, reason: reason) }
    func onDiscovered(d: FfiDiscovery) { inner.onDiscovered(d: d) }
    func onHelloRead(discId: UInt32, bytes: Data) { inner.onHelloRead(discId: discId, bytes: bytes) }
    func onL2capPublished(psm: UInt16) { inner.onL2capPublished(psm: psm) }
    func onLinkData(link: UInt32, data: Data) { inner.onLinkData(link: link, data: data) }
    func onLinkDown(link: UInt32, reason: FfiLinkDownReason) { inner.onLinkDown(link: link, reason: reason) }
    func onLinkMetrics(link: UInt32, rssi: Int16?, attMtu: UInt16?) {
        inner.onLinkMetrics(link: link, rssi: rssi, attMtu: attMtu)
    }

    func onLinkUp(link: UInt32, info: FfiLinkInfo) {
        log.add("engine.onLinkUp:\(link)")
        inner.onLinkUp(link: link, info: info)
    }

    func onLinkWritable(link: UInt32) { inner.onLinkWritable(link: link) }
    func onPlatformSample(s: FfiPlatformSample) { inner.onPlatformSample(s: s) }
    func onPromptPcm(key: String, pcm: [Int16]) {
        log.add("engine.onPromptPcm:\(key):\(pcm.count)")
        inner.onPromptPcm(key: key, pcm: pcm)
    }
    func setUiAttached(attached: Bool) { inner.setUiAttached(attached: attached) }

    func start(start: FfiSessionStart) throws {
        log.add("engine.start")
        try inner.start(start: start)
    }

    func status() -> FfiRuntimeStatus { inner.status() }

    func diagConfigure(enabled: Bool, debugBuild: Bool) { inner.diagConfigure(enabled: enabled, debugBuild: debugBuild) }
    func diagFlush() { inner.diagFlush() }
    func diagMark(label: String) -> Bool { inner.diagMark(label: label) }
    func diagNote(src: String, ev: String, fieldsJson: String) -> Bool {
        inner.diagNote(src: src, ev: ev, fieldsJson: fieldsJson)
    }
    func diagSnapshotRing() -> [String] { inner.diagSnapshotRing() }
    func diagStatus() -> FfiDiagStatus { inner.diagStatus() }
    func diagTerminate() { inner.diagTerminate() }

    func stop() {
        log.add("engine.stop")
        inner.stop()
    }
}

/// Collected UI events.
final class EventBox: Sendable {
    private let m = Mutex<[String]>([])
    func append(_ s: String) { m.withLock { $0.append(s) } }
    var all: [String] { m.withLock { $0 } }
}

/// One `EngineHost` built over fakes: fake radio stack on a shared `FakeAir`, a recording
/// audio control, an in-memory keychain and a temporary Lab directory.
@MainActor
final class HostRig {
    let log = SequenceLog()
    let audio: FakeAudioControl
    let keychain = FakeKeychain()
    let foreground = ForegroundState(true)
    let calls = FakeCallState()
    let activityPort = FakeActivityPort()
    let notifications = FakeNotificationPort()
    let anchor = FakeLocationAnchor()
    let prompts = FakePrompts()
    let labDir: URL
    let labStore: LabFileStore
    let diagStore: DiagFileStore
    let host: EngineHost
    private(set) var engine: RecordingEngine?
    private var collected: [String] = []
    private let events = EventBox()

    init(air: FakeAir, seed: UInt8, foregroundActive: Bool = true) throws {
        audio = FakeAudioControl(log: log)
        labDir = FileManager.default.temporaryDirectory
            .appending(path: "meshenginekit-\(UUID().uuidString)", directoryHint: .isDirectory)
        labStore = try LabFileStore(directory: labDir)
        diagStore = try DiagFileStore(directory: labDir.appending(path: "diag", directoryHint: .isDirectory))
        foreground.set(foregroundActive)
        let fg = foreground
        let log = self.log
        let box = Mutex<RecordingEngine?>(nil)
        let deps = EngineHostDependencies(
            engineFactory: { cfg, hooks in
                let e = RecordingEngine(try MeshEngine(cfg: cfg, hooks: hooks), log: log)
                box.withLock { $0 = e }
                return e
            },
            radioFactory: { air.stack($0) },
            audio: audio,
            secureStore: SecureStore(service: "test.\(UUID().uuidString)", keychain: keychain),
            labStore: labStore,
            diagStore: diagStore,
            foreground: fg,
            foregroundCheck: { fg.isForegroundActive },
            calls: calls,
            promptFactory: { [prompts] onPcm, onFailure in
                prompts.bind(onPcm: onPcm, onFailure: onFailure)
                return prompts
            },
            activity: IntercomActivityController(port: activityPort, autoFlush: false),
            notifier: ResumeNotifier(port: notifications),
            location: KeepAliveLocation(port: anchor),
            device: DeviceSampler(port: FakeDeviceInfo()),
            lifecycle: LifecycleMonitor(center: NotificationCenter(),
                                        initialVisibility: foregroundActive ? .foreground : .background,
                                        initiallyLocked: false))
        let config = EngineHostConfig(appVersion: 12, appLabel: "0.1.0 (12)", deviceModel: "iPhone15,2",
                                      deviceOs: "iOS 27.0", rngSeed: Data(repeating: seed, count: 32),
                                      sampleInterval: 0.25, registerActivityActions: false)
        host = try EngineHost(config: config, deps: deps)
        engine = box.withLock { $0 }
        let ev = events
        host.setEventHandler { json in ev.append(json) }
    }

    deinit {
        try? FileManager.default.removeItem(at: labDir)
    }

    /// Every UI event so far (parsed).
    var uiEvents: [[String: Any]] {
        events.all.compactMap { CommandJSON.object($0) }
    }

    func events(ofType t: String) -> [[String: Any]] {
        uiEvents.filter { $0["type"] as? String == t }
    }

    /// `mesh.links` of the newest snapshot (requests one first).
    func snapshotLinks() -> Int? {
        _ = host.send(#"{"type":"requestSnapshot"}"#)
        let s = events(ofType: "snapshot").last?["s"] as? [String: Any]
        return (s?["mesh"] as? [String: Any])?["links"] as? Int
    }

    func labLines(_ runId: String) -> [[String: Any]] {
        ((try? labStore.lines(runId: runId)) ?? []).compactMap { CommandJSON.object($0) }
    }
}
