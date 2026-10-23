@preconcurrency import MeshCoreBindings
import Foundation
import MeshActivityShared
import MeshPlatform
import Synchronization

/// Owns the Rust engine (`MeshEngine`) and implements its platform side on iOS (DESIGN
/// §3.5, §10.1.1): `PlatformHooks` (radio, audio, UI events, persist, notifications, Lab
/// files, prompts) and the drivers' `LinkSink`. Consumed by the app-local Capacitor bridge
/// (WP-19).
///
/// Threading: every public method may be called from any thread except where noted; the
/// engine's hooks run on the engine thread and never block on the main thread.
public final class EngineHost: Sendable {
    /// UI contract major this bridge speaks (mesh-api `API_VERSION` 1.x, §3.7).
    public static let apiMajor: UInt16 = 1

    let engineRef: EngineRef
    let radio: RadioAdapter
    let audio: AudioAdapter
    let hooks: HooksBridge
    let mainSide: HostMainSide
    private let hub: EventHub
    private let engine: any EngineHandle
    private let config: EngineHostConfig
    private let foregroundCheck: @Sendable () -> Bool
    let state: HostState

    /// Builds the host and its engine (cheap: the engine thread starts lazily).
    @MainActor
    public init(config: EngineHostConfig, deps: EngineHostDependencies) throws(EngineHostError) {
        let api = apiVersion()
        guard api >> 8 == Self.apiMajor else { throw .apiVersionMismatch(engine: api) }
        self.config = config
        foregroundCheck = deps.foregroundCheck
        let ref = EngineRef()
        engineRef = ref
        let hub = EventHub()
        self.hub = hub
        let logSink = deps.logSink
        let log: @Sendable (FfiLogLevel, String) -> Void = { level, msg in
            PlatformLog.host.log("\(msg, privacy: .public)")
            logSink?(level, msg)
        }
        let constants = deps.bleConstants ?? BleConstants(bleConstants()) ?? .design
        let mainBox = MainSideBox()
        radio = RadioAdapter(engine: ref, constants: constants, factory: deps.radioFactory) { state in
            ref.note(.bt(state))
            MainHop.async { mainBox.side?.sampleNow() }
        }
        let stateRef = HostState()
        state = stateRef
        audio = AudioAdapter(
            audio: deps.audio, engine: ref, foreground: deps.foreground, calls: deps.calls,
            callbacks: AudioAdapter.Callbacks(
                vehicle: { stateRef.withLock { $0.vehicle } },
                wantsLocationAnchor: { MainHop.async { mainBox.side?.startLocationAnchor() } },
                sessionClosed: {
                    stateRef.withLock { $0.rideLive = false }
                    MainHop.async { mainBox.side?.audioSessionClosed() }
                },
                event: { e in MainHop.async { mainBox.side?.notify.audioEvent(e) } },
                log: log))
        let prompts = deps.promptFactory(
            { key, pcm in ref.engine?.onPromptPcm(key: key, pcm: pcm) },
            { key in log(.info, "prompt \(key) not rendered; the core keeps its earcon") })
        hooks = HooksBridge(
            radio: radio, audio: audio,
            persistence: PersistAdapter(store: deps.secureStore, log: log),
            lab: LabAdapter(store: deps.labStore, log: log),
            diag: DiagAdapter(store: deps.diagStore, log: log),
            hub: hub, prompts: prompts,
            main: HooksBridge.MainSink(
                uiEvent: { json in MainHop.async { mainBox.side?.notify.uiEvent(json) } },
                notify: { key, params, isError in
                    MainHop.async { mainBox.side?.notify.notify(key: key, paramsJson: params, isError: isError) }
                }),
            logSink: logSink)
        let cfg = FfiEngineConfig(platform: .ios, appVersion: config.appVersion,
                                  rngSeed: config.rngSeed ?? EngineHostConfig.randomSeed(),
                                  deviceModel: config.deviceModel, deviceOs: config.deviceOs,
                                  appLabel: config.appLabel, persisted: hooks.persistence.load())
        do {
            engine = try deps.engineFactory(cfg, hooks)
        } catch {
            throw .engine("\(error)")
        }
        ref.set(engine)
        engine.diagConfigure(enabled: DiagPreference.load(appLabel: config.appLabel, debug: DiagPreference.isDebugBuild),
                             debugBuild: DiagPreference.isDebugBuild)
        mainSide = HostMainSide(deps: deps, engine: ref, radio: radio, interval: config.sampleInterval,
                                log: { log(.warn, $0) })
        mainBox.side = mainSide
        mainSide.onVisibility = { [weak self] v in self?.visibilityChanged(v) }
        if config.registerActivityActions { registerLiveActivityActions() }
    }

    deinit {
        hub.finishAll()
    }

    // MARK: Engine lifecycle

    /// Radios + engine thread for a group lobby, Lab mode or the Demo ride. Audio stays
    /// off until a gesture command. Call inside the user's tap (§1.3 F3).
    ///
    /// **Demo ride.** `.demo` is forwarded unchanged to `MeshEngine.start(.demo)`, where
    /// the runtime attaches mesh-sim's demo world (bots on virtual links on the engine
    /// thread); this host adds nothing fake:
    /// - no radio drivers are created (the world uses virtual links inside the runtime),
    ///   so the Demo never raises the Bluetooth permission prompt;
    /// - the bots' `demo.*` lines arrive through the ordinary `render_prompts` hook and are
    ///   rendered by `PromptRenderer` (platform TTS) back into `on_prompt_pcm`;
    /// - audio starts exactly like a group ride: the UI calls `goLive()` inside the tap,
    ///   which activates the session and VPIO muted before the engine sees the command.
    /// Covered by `EngineHostDemoTests` ("Demo ride plays audio in the simulator").
    public func start(_ s: SessionStart) throws(EngineHostError) {
        if s != .demo { radio.ensureStack() }
        do {
            try engine.start(start: s.ffi)
        } catch {
            throw .engine("\(error)")
        }
        MainHop.async { [mainSide] in mainSide.sessionStarted() }
    }

    /// Stops the session and returns to Idle (F8).
    ///
    /// Shutdown is **ordered** (core `Engine::stop`): the session floods its signed LEAVE,
    /// then every established link closes gracefully (queued P0 first, then
    /// `CLOSE{leaving}`) and the platform close follows as `RadioCommand.close` once
    /// drained, bounded by the core's flush/drain timers that the runtime keeps serving in
    /// Idle. This host therefore never tears links down itself: the radio stack, the link
    /// registry and the link I/O thread stay up after `stop()` and keep executing the
    /// engine's `Close` (and `SetDiscovery(off)`) commands. Only the audio session, the
    /// anchors and the platform sampler end here.
    public func stop() {
        engine.stop()
        state.withLock { $0.rideLive = false }
        // The engine's CloseSession already closed it; this only covers a session the
        // core never opened (a gesture whose command was still in flight).
        if audio.audio.isLive { audio.audio.close() }
        MainHop.async { [mainSide] in mainSide.sessionStopped() }
    }

    /// Runtime state.
    public var status: FfiRuntimeStatus { engine.status() }

    // MARK: Events

    /// Every UI event (JSON) from now on.
    public var events: AsyncStream<String> { hub.stream() }

    /// A callback for every UI event (the bridge's `notifyListeners`), or nil.
    public func setEventHandler(_ handler: (@Sendable (String) -> Void)?) {
        hub.setHandler(handler)
    }

    /// The WebView is attached: gates snapshot emission (§11.1).
    public func setUiAttached(_ attached: Bool) {
        state.withLock { $0.uiAttached = attached }
        engine.setUiAttached(attached: attached)
    }

    /// Localised notification text from the UI catalogs (see `NotificationCatalog`).
    public func setNotificationCatalog(_ catalog: NotificationCatalog?) {
        MainHop.async { [mainSide] in mainSide.notify.setCatalog(catalog) }
    }

    /// The vehicle profile used for session options (`.car` adds `.defaultToSpeaker`). Also
    /// tracked from accepted `createGroup` / `setVehicle` commands.
    public var vehicle: VehicleProfile {
        get { state.withLock { $0.vehicle } }
        set { state.withLock { $0.vehicle = newValue } }
    }

    // MARK: Info

    public struct Info: Sendable, Equatable {
        public var apiVersion: String
        public var protocolVersion: UInt8
        public var rtAbiVersion: UInt32
        public var appVersion: String
        /// Always "full": this package links the engine (§2.4 `getInfo`).
        public var engine: String
        public var status: String
        public var uiAttached: Bool
    }

    public func info() -> Info {
        let api = apiVersion()
        let status: String
        switch engine.status() {
        case .idle: status = "idle"
        case .running: status = "running"
        case .audioCheck: status = "audioCheck"
        case .crashed: status = "crashed"
        }
        return Info(apiVersion: "\(api >> 8).\(api & 0xFF)", protocolVersion: protocolVersion(),
                    rtAbiVersion: rtAbiVersion(), appVersion: config.appLabel, engine: "full",
                    status: status, uiAttached: state.withLock { $0.uiAttached })
    }

    // MARK: Internals shared with the extensions

    var engineHandle: any EngineHandle { engine }
    var gestureForegroundCheck: @Sendable () -> Bool { foregroundCheck }

    private func visibilityChanged(_ v: AppVisibility) {
        if config.followAppStateForUi, v != .inactive {
            setUiAttached(v == .foreground)
        }
        if v == .foreground, state.withLock({ $0.pendingResume }) {
            state.withLock { $0.pendingResume = false }
            _ = try? resumeMic()
        }
    }
}

/// Mutable host state behind a lock (shared with closures created during init).
final class HostState: Sendable {
    struct Values {
        var vehicle: VehicleProfile = .motorcycle
        var uiAttached = false
        var pendingResume = false
        /// An accepted `goLive` holds the ride's audio session (until CloseSession/stop).
        var rideLive = false
    }

    private let m = Mutex(Values())

    func withLock<R>(_ body: (inout Values) -> R) -> R {
        m.withLock { body(&$0) }
    }
}

/// Late binding of the main side for closures created before it exists. Weak: the main
/// side references the radio adapter, whose radio-state listener captures this box.
final class MainSideBox: @unchecked Sendable {
    // Written once on the main actor during init, read on the main actor afterwards.
    weak var side: HostMainSide?
}
