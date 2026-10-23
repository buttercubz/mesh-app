@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform
import UIKit

/// The main-actor half of `EngineHost`: Live Activity and notifications, the location
/// anchor, app-state and lock observers, and the 1 Hz platform sampler (§12.1).
@MainActor
final class HostMainSide {
    let notify: NotifyAdapter
    let location: KeepAliveLocation
    let device: DeviceSampler
    let lifecycle: LifecycleMonitor
    private let engine: EngineRef
    private let audio: any AudioControl
    private let radio: RadioAdapter
    private let calls: any CallStatePort
    private let foreground: ForegroundState
    private let interval: TimeInterval
    private var sampler: Task<Void, Never>?
    /// Last power/thermal state noted (diagnostics: `power` only on change).
    private var lastPower: String?
    private let observers = ObserverBag()
    /// Called on every visibility change (the host forwards UI attach and resumes).
    var onVisibility: ((AppVisibility) -> Void)?
    private(set) var samplesSent = 0

    init(deps: EngineHostDependencies, engine: EngineRef, radio: RadioAdapter, interval: TimeInterval,
         log: @escaping @Sendable (String) -> Void) {
        notify = NotifyAdapter(activity: deps.activity, notifier: deps.notifier, log: log)
        location = deps.location
        device = deps.device
        lifecycle = deps.lifecycle
        self.engine = engine
        self.audio = deps.audio
        self.radio = radio
        self.calls = deps.calls
        self.foreground = deps.foreground
        self.interval = interval
        foreground.set(lifecycle.visibility == .foreground)
        lifecycle.start { [weak self] e in self?.lifecycleEvent(e) }
        notify.diag = { [engine] n in engine.note(n) }
        engine.note(.os("coldStart"))
        let nc = NotificationCenter.default
        observers.add(nc.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification,
                                        object: nil, queue: .main) { [engine] _ in
            engine.note(.os("memoryWarning"))
        })
        observers.add(nc.addObserver(forName: UIApplication.willTerminateNotification,
                                        object: nil, queue: .main) { [engine] _ in
            engine.note(.os("terminate"))
            engine.engine?.diagTerminate()
        })
    }


    // MARK: Session

    func sessionStarted() {
        guard sampler == nil else { return }
        sampleNow()
        let interval = self.interval
        sampler = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(interval))
                guard !Task.isCancelled, let self else { return }
                self.sampleNow()
            }
        }
    }

    func sessionStopped() {
        sampler?.cancel()
        sampler = nil
        audioSessionClosed()
    }

    /// The audio session closed (core `CloseSession` or stop): anchors end.
    func audioSessionClosed() {
        location.stop()
        if notify.hasActivity { notify.endActivity() }
    }

    func startLocationAnchor() {
        guard !location.isActive else { return }
        location.start { [weak self] item in
            // A first fix or a denial is worth an immediate sample (distance list, §12.1).
            if case .idle = item { return }
            self?.sampleNow()
        }
    }

    // MARK: Platform samples (§3.2 `PlatformEvent::Platform`, §12.1)

    func sample() -> FfiPlatformSample {
        let d = device.sample()
        let power = "\(d.lowPowerMode)|\(d.thermal)"
        if power != lastPower {
            lastPower = power
            engine.note(.power(lowPower: d.lowPowerMode, thermal: d.thermal.rawValue, battery: d.battery))
        }
        return FfiPlatformSample(
            wallMs: UInt64(max(0, Date().timeIntervalSince1970 * 1000)),
            gps: location.isActive ? location.lastFix.map(FfiGpsFix.init) : nil,
            battery: d.battery,
            charging: d.charging,
            thermal: FfiThermal(d.thermal),
            background: lifecycle.visibility != .foreground,
            locked: lifecycle.deviceLocked || d.locked,
            onCall: calls.onCall,
            anchorSession: audio.isLive,
            anchorLocation: location.isActive,
            anchorLiveActivity: notify.activity.isActive,
            // Unknown until CoreBluetooth answers; the real state follows within ms and
            // triggers an immediate sample.
            ble: radio.bluetooth.map(FfiRadioAvail.init) ?? .on,
            aware: .unsupported
        )
    }

    func sampleNow() {
        guard let e = engine.engine else { return }
        e.onPlatformSample(s: sample())
        samplesSent += 1
    }

    // MARK: App state and lock observers

    private func lifecycleEvent(_ e: LifecycleEvent) {
        switch e {
        case .visibility(let v):
            engine.note(.app(v, locked: lifecycle.deviceLocked))
            foreground.set(v == .foreground)
            radio.setForeground(v == .foreground)
            if v == .foreground {
                audio.appDidBecomeActive()
                Task { [activity = notify.activity] in await activity.appWillEnterForeground() }
            }
            onVisibility?(v)
        case .deviceLocked(let locked):
            engine.note(DiagNote("lock", [("locked", .bool(locked))]))
        }
        if sampler != nil { sampleNow() }
    }
}

/// NotificationCenter block observers, removed when the main side goes away.
final class ObserverBag: @unchecked Sendable {
    // Appended on the main actor during init only; read once in deinit.
    private var tokens: [NSObjectProtocol] = []

    func add(_ t: NSObjectProtocol) {
        tokens.append(t)
    }

    deinit {
        for t in tokens { NotificationCenter.default.removeObserver(t) }
    }
}
