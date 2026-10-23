@preconcurrency import MeshCoreBindings
import Foundation
import MeshActivityShared
@testable import MeshEngineKit
import MeshPlatform
import Security
import Synchronization
import UIKit
import XCTest

/// One ordered log shared by the fakes of a test, to assert cross-component ordering
/// ("the session is activated before the command reaches the engine").
final class SequenceLog: Sendable {
    private let entries = Mutex<[String]>([])

    func add(_ e: String) { entries.withLock { $0.append(e) } }
    var all: [String] { entries.withLock { $0 } }

    func index(of prefix: String) -> Int? {
        all.firstIndex { $0.hasPrefix(prefix) }
    }

    func count(_ prefix: String) -> Int {
        all.filter { $0.hasPrefix(prefix) }.count
    }
}

/// A thread-safe counter.
final class Counter: Sendable {
    private let m = Mutex(0)
    func bump() { m.withLock { $0 += 1 } }
    var value: Int { m.withLock { $0 } }
}

/// Polls `cond` while the main run loop keeps turning (main-actor tasks run meanwhile).
@MainActor
func waitUntil(_ what: String, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line,
               _ cond: () -> Bool) {
    let deadline = Date().addingTimeInterval(timeout)
    while !cond() {
        if Date() > deadline {
            XCTFail("timed out waiting for: \(what)", file: file, line: line)
            return
        }
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    }
}

/// In-memory keychain (hostless xctest bundles have no keychain entitlement).
final class FakeKeychain: KeychainPort, @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String: Data] = [:]

    private func account(_ q: [String: Any]) -> String? { q[kSecAttrAccount as String] as? String }

    func add(_ attrs: [String: Any]) -> OSStatus {
        lock.withLock {
            guard let a = account(attrs), let d = attrs[kSecValueData as String] as? Data else { return errSecParam }
            if items[a] != nil { return errSecDuplicateItem }
            items[a] = d
            return errSecSuccess
        }
    }

    func update(_ query: [String: Any], _ attrs: [String: Any]) -> OSStatus {
        lock.withLock {
            guard let a = account(query), items[a] != nil else { return errSecItemNotFound }
            items[a] = attrs[kSecValueData as String] as? Data
            return errSecSuccess
        }
    }

    func copyMatching(_ query: [String: Any]) -> (OSStatus, AnyObject?) {
        lock.withLock {
            if let a = account(query) {
                guard let d = items[a] else { return (errSecItemNotFound, nil) }
                return (errSecSuccess, d as NSData)
            }
            if items.isEmpty { return (errSecItemNotFound, nil) }
            let list = items.keys.map { [kSecAttrAccount as String: $0] as NSDictionary }
            return (errSecSuccess, list as NSArray)
        }
    }

    func delete(_ query: [String: Any]) -> OSStatus {
        lock.withLock {
            if let a = account(query) {
                return items.removeValue(forKey: a) == nil ? errSecItemNotFound : errSecSuccess
            }
            items.removeAll()
            return errSecSuccess
        }
    }

    var keys: [String] { lock.withLock { items.keys.sorted() } }
}

/// Records every audio call; behaves like `PlatformAudio` for liveness.
final class FakeAudioControl: AudioControl, @unchecked Sendable {
    private let lock = NSLock()
    private var live = false
    private var lost = false
    private var listener: (@Sendable (PlatformAudioEvent) -> Void)?
    let log: SequenceLog
    var failActivation = false

    init(log: SequenceLog) { self.log = log }

    func activateAndStartMuted(profile: VehicleProfile, path: AudioPath, mixable: Bool) throws(AudioPlatformError) {
        if lock.withLock({ failActivation }) {
            log.add("audio.activateFailed")
            throw .session("setActive", 561_145_187)
        }
        let wasLive = lock.withLock { () -> Bool in
            let w = live
            live = true
            return w
        }
        log.add(wasLive ? "audio.resume" : "audio.activate:\(path.rawValue):\(profile.rawValue):mixable=\(mixable)")
    }

    func connectRtSink() { log.add("audio.connectRtSink") }
    func disconnectRtSink() { log.add("audio.disconnectRtSink") }
    func unmute() { log.add("audio.unmute") }
    func mute() { log.add("audio.mute") }
    func setOutputOnlyFallback(_ on: Bool) { log.add("audio.outputOnly:\(on)") }
    func setRoutePreference(_ pref: RoutePreference) { log.add("audio.route:\(pref.rawValue)") }
    func resume() { log.add("audio.resume") }

    func close() {
        lock.withLock {
            live = false
            lost = false
        }
        log.add("audio.close")
    }

    func appDidBecomeActive() { log.add("audio.appDidBecomeActive") }
    var isLive: Bool { lock.withLock { live } }
    var isCaptureLost: Bool { lock.withLock { lost } }

    func setListener(_ l: (@Sendable (PlatformAudioEvent) -> Void)?) {
        lock.withLock { listener = l }
    }

    /// Simulates a platform event (mic-loss state machine output).
    func emit(_ e: PlatformAudioEvent) {
        if case .io(.captureLost) = e { lock.withLock { lost = true } }
        let l = lock.withLock { listener }
        l?(e)
    }
}

final class FakeCallState: CallStatePort, @unchecked Sendable {
    var onCall = false
}

@MainActor
final class FakeDeviceInfo: DeviceInfoPort {
    var batteryLevel: Float = 0.8
    var batteryState: UIDevice.BatteryState = .unplugged
    var thermalState: ProcessInfo.ThermalState = .nominal
    var isLowPowerModeEnabled = false
    var isProtectedDataAvailable = true
    func enableBatteryMonitoring() {}
}

@MainActor
final class FakeActivityPort: LiveActivityPort {
    var activitiesEnabled = true
    var requests: [(IntercomAttributes, IntercomAttributes.ContentState)] = []
    var updates: [IntercomAttributes.ContentState] = []
    var ends = 0

    func request(_ attributes: IntercomAttributes, _ state: IntercomAttributes.ContentState) throws -> String {
        requests.append((attributes, state))
        return "act-\(requests.count)"
    }

    func update(id: String, _ state: IntercomAttributes.ContentState) async { updates.append(state) }
    func end(id: String, _ state: IntercomAttributes.ContentState?) async { ends += 1 }
}

final class FakeNotificationPort: LocalNotificationPort, @unchecked Sendable {
    private let lock = NSLock()
    private var added: [LocalNotificationSpec] = []
    private var removed: [String] = []

    func add(_ spec: LocalNotificationSpec) async throws { lock.withLock { added.append(spec) } }
    func remove(ids: [String]) { lock.withLock { removed += ids } }
    var specs: [LocalNotificationSpec] { lock.withLock { added } }
    var removedIds: [String] { lock.withLock { removed } }
}

final class FakeToken: BackgroundActivityToken, @unchecked Sendable {
    var invalidated = false
    func invalidate() { invalidated = true }
}

@MainActor
final class FakeLocationAnchor: LocationAnchorPort {
    var begun = 0
    var tokens: [FakeToken] = []

    func beginBackgroundActivity() -> any BackgroundActivityToken {
        begun += 1
        let t = FakeToken()
        tokens.append(t)
        return t
    }

    func liveUpdates() -> AsyncStream<LocationUpdateItem> {
        AsyncStream { c in
            c.yield(.fix(LocationFix(latitude: 18.4861, longitude: -69.9312, speed: 0, accuracy: 5,
                                     timestamp: Date())))
        }
    }
}

/// Records `render_prompts` requests and keeps the host's result callbacks, so a test can
/// answer like `PromptRenderer` (PCM per key, or a failure that keeps the earcon).
final class FakePrompts: PromptRendering, @unchecked Sendable {
    private let lock = NSLock()
    private var reqs: [PromptRequest] = []
    private var onPcm: PromptRenderer.PcmHandler?
    private var onFailure: PromptRenderer.FailureHandler?

    func bind(onPcm: @escaping PromptRenderer.PcmHandler, onFailure: @escaping PromptRenderer.FailureHandler) {
        lock.withLock {
            self.onPcm = onPcm
            self.onFailure = onFailure
        }
    }

    func render(_ r: [PromptRequest]) { lock.withLock { reqs += r } }
    var requests: [PromptRequest] { lock.withLock { reqs } }

    /// Delivers rendered PCM for `key` through the host (→ `on_prompt_pcm`).
    func deliver(_ key: String, _ pcm: [Int16]) {
        let h = lock.withLock { onPcm }
        h?(key, pcm)
    }

    func fail(_ key: String) {
        let h = lock.withLock { onFailure }
        h?(key)
    }
}
