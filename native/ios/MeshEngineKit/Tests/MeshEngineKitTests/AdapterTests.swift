@preconcurrency import MeshCoreBindings
import AVFAudio
import CMeshAudio
import Foundation
@testable import MeshEngineKit
import MeshPlatform
import Synchronization
import XCTest

final class StorageAdapterTests: XCTestCase {
    func testPersistRoundTripsThroughTheKeychain() {
        let kc = FakeKeychain()
        let p = PersistAdapter(store: SecureStore(service: "t", keychain: kc), log: { _, _ in })
        p.persist("group/00112233445566778899aabbccddeeff", Data([1, 2, 3]))
        p.persist("lab/index", Data([9]))
        p.persist("lab/index", Data([8, 8]))
        XCTAssertEqual(p.load(), [
            FfiPersistedEntry(key: "group/00112233445566778899aabbccddeeff", value: Data([1, 2, 3])),
            FfiPersistedEntry(key: "lab/index", value: Data([8, 8])),
        ])
        p.persist("lab/index", nil)
        XCTAssertEqual(kc.keys, ["group/00112233445566778899aabbccddeeff"])
    }

    func testLabFilesAreWrittenSummarisedAndDeleted() throws {
        let dir = FileManager.default.temporaryDirectory.appending(path: "lab-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = try LabFileStore(directory: dir)
        let errors = Mutex<[String]>([])
        let lab = LabAdapter(store: store, log: { _, m in errors.withLock { $0.append(m) } })
        let id = "r-20261023-0912-ab12"
        lab.write(runId: id, line: #"{"t":"run","v":1,"runId":"r-20261023-0912-ab12"}"#)
        lab.write(runId: id, line: #"{"t":"s","ts":1.0}"#)
        lab.write(runId: id, line: #"{"t":"summary","durationS":1.0,"verdict":"go"}"# + "\n")
        XCTAssertEqual(try store.lines(runId: id).count, 3)
        XCTAssertEqual(LabAdapter.stripNewline("a\r\n"), "a")
        lab.writeSummary(runId: id, line: "runId,bench,verdict")
        lab.writeSummary(runId: id, line: "r-20261023-0912-ab12,T0,go")
        lab.writeSummary(runId: id, line: "runId,bench,verdict") // header again: ignored
        let csv = try String(contentsOf: store.summaryURL(for: id), encoding: .utf8)
        XCTAssertEqual(csv, "runId,bench,verdict\nr-20261023-0912-ab12,T0,go\n")
        XCTAssertEqual(try store.exportURLs(runId: id).count, 2)
        lab.write(runId: "../escape", line: "{}")
        lab.write(runId: id, line: "{}\n{}\n")
        XCTAssertEqual(errors.withLock { $0.count }, 2, "a bad run id or an inner newline is refused and logged")
        lab.delete(runId: id)
        XCTAssertTrue(store.list().isEmpty)
        XCTAssertTrue(LabAdapter.isSummaryLine(#"{"t":"summary"}"#))
        XCTAssertFalse(LabAdapter.isSummaryLine(#"{"t":"s","label":"summary"}"#))
    }
}

/// `AudioAdapter` against a recording audio control (core commands → platform calls).
final class AudioAdapterTests: XCTestCase {
    private final class Rig {
        let log = SequenceLog()
        let audio: FakeAudioControl
        let fg = ForegroundState(true)
        let closed = Counter()
        let anchors = Counter()
        let adapter: AudioAdapter

        init() {
            audio = FakeAudioControl(log: log)
            let closed = self.closed
            let anchors = self.anchors
            adapter = AudioAdapter(audio: audio, engine: EngineRef(), foreground: fg, calls: FakeCallState(),
                                   callbacks: .init(vehicle: { .car },
                                                    wantsLocationAnchor: { anchors.bump() },
                                                    sessionClosed: { closed.bump() },
                                                    event: { _ in }, log: { _, _ in }))
        }
    }

    private func profile(capture: Bool, location: Bool = false) -> FfiAudioSessionProfile {
        FfiAudioSessionProfile(kind: capture ? .labFull : .labRender, capture: capture, mixable: false,
                               locationAnchor: location, bypassVoiceProcessing: false)
    }

    /// StartCapture only connects the sink and unmutes; it never activates a session.
    func testProductSessionsAreAlwaysMixableAndLabHonoursTheCoreFlag() {
        func p(_ k: FfiAudioSessionKind, _ m: Bool) -> FfiAudioSessionProfile {
            FfiAudioSessionProfile(kind: k, capture: true, mixable: m, locationAnchor: false,
                                   bypassVoiceProcessing: false)
        }
        XCTAssertTrue(AudioAdapter.sessionMixable(p(.ride, false)), "music keeps playing during a ride")
        XCTAssertTrue(AudioAdapter.sessionMixable(p(.check, false)))
        XCTAssertFalse(AudioAdapter.sessionMixable(p(.labFull, false)), "T2 non-mixable variant")
        XCTAssertTrue(AudioAdapter.sessionMixable(p(.labRender, true)))
    }

    func testStartCaptureNeverActivates() {
        let r = Rig()
        r.adapter.handle(.startCapture)
        r.adapter.handle(.startCapture)
        XCTAssertEqual(r.log.all, ["audio.connectRtSink", "audio.unmute", "audio.connectRtSink", "audio.unmute"])
        XCTAssertFalse(r.audio.isLive)
    }

    func testOpenSessionAfterTheGestureOnlyConnects() throws {
        let r = Rig()
        let plan = GesturePlan.make(.labStart(.init(bench: "T1", audio: "renderOnly", anchor: "audio", mixable: false)),
                                    shareDistance: false)
        XCTAssertTrue(try r.adapter.prepareGesture(plan))
        r.adapter.handle(.openSession(profile: profile(capture: false, location: true)))
        XCTAssertEqual(r.log.all, ["audio.activate:renderOnly:car:mixable=false", "audio.connectRtSink"])
        XCTAssertEqual(r.anchors.value, 1)
        XCTAssertFalse(try r.adapter.prepareGesture(plan), "a second gesture on a live session is a resume")
        r.adapter.handle(.closeSession)
        XCTAssertEqual(r.log.all.last, "audio.close")
        XCTAssertEqual(r.closed.value, 1)
        XCTAssertNil(r.adapter.openedProfile)
    }

    func testOpenSessionInTheBackgroundIsReportedNotFaked() {
        let r = Rig()
        r.fg.set(false)
        r.adapter.handle(.openSession(profile: profile(capture: true)))
        XCTAssertEqual(r.log.all, [], "no activation and no sink without a foreground gesture")
        r.fg.set(true)
        r.adapter.handle(.openSession(profile: profile(capture: true)))
        XCTAssertEqual(r.log.all, ["audio.activate:full:car:mixable=false", "audio.connectRtSink"])
    }

    func testOutputOnlyFallbackIsIdempotentWithThePlatformFallback() {
        let r = Rig()
        r.adapter.handle(.outputOnlyFallback(enabled: true))
        r.audio.emit(.io(.captureLost))
        r.adapter.handle(.outputOnlyFallback(enabled: true))
        r.adapter.handle(.outputOnlyFallback(enabled: false))
        r.adapter.handle(.setRoutePreference(pref: .speaker))
        r.adapter.handle(.stopCapture)
        XCTAssertEqual(r.log.all, ["audio.outputOnly:true", "audio.outputOnly:false", "audio.route:speaker", "audio.mute"])
    }

    func testFailedActivationThrows() {
        let r = Rig()
        r.audio.failActivation = true
        XCTAssertThrowsError(try r.adapter.prepareGesture(GesturePlan.make(.goLive, shareDistance: false)))
        XCTAssertFalse(r.audio.isLive)
    }
}
