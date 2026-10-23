@preconcurrency import MeshCoreBindings
import Foundation
@testable import MeshEngineKit
import MeshPlatform
import XCTest

/// The ridelog/1 recorder through the real Rust engine: platform notes (app state,
/// Bluetooth, audio open with the gesture), a Lab session with its ride key and 1 Hz
/// samples, marks, and an export zip that `lab-analyze ride` reads (WP-15 diagnostics).
@MainActor
final class EngineHostDiagTests: XCTestCase {
    private var air: FakeAir!
    private var rig: HostRig!

    override func setUp() async throws {
        air = FakeAir()
        rig = try HostRig(air: air, seed: 41)
        rig.host.diagSetEnabled(true)
    }

    override func tearDown() async throws {
        // The preference lives in UserDefaults: leave it on for the next test.
        rig.host.diagSetEnabled(true)
        rig.host.stop()
        rig = nil
        air = nil
    }

    private func lines() -> [[String: Any]] {
        _ = rig.host.diagList()
        return rig.diagStore.sessions().flatMap { s in
            s.segments.flatMap { u -> [[String: Any]] in
                let text = (try? String(contentsOf: u, encoding: .utf8)) ?? ""
                return text.split(separator: "\n").compactMap { CommandJSON.object(String($0)) }
            }
        }
    }

    func testNotesSessionMarkAndExport() throws {
        rig.host.diagNote(.perm(name: "microphone", state: "granted", via: "check",
                                api: "AVAudioApplication.recordPermission"))
        try rig.host.start(.lab(code: 1357))
        XCTAssertTrue(rig.host.diagMark("crackle"))
        XCTAssertFalse(rig.host.diagMark("free text is never accepted"))
        XCTAssertTrue(rig.host.diagUiNote(ev: "cmd", fieldsJson: #"{"name":"goLive","gesture":true,"ok":true}"#))
        waitUntil("two 1 Hz samples on disk", timeout: 10) {
            self.lines().filter { $0["t"] as? String == "s" }.count >= 2
        }
        let all = lines()
        let headers = all.filter { $0["t"] as? String == "hdr" }
        XCTAssertTrue(headers.allSatisfy { $0["schema"] as? String == "ridelog/1" })
        let lab = headers.first { ($0["rideKey"] as? String)?.hasPrefix("lab-") == true }
        XCTAssertNotNil(lab, "the Lab session's header carries its ride key")
        XCTAssertEqual((lab?["device"] as? [String: Any])?["platform"] as? String, "ios")
        XCTAssertTrue(all.contains { $0["ev"] as? String == "perm" && $0["src"] as? String == "plat" })
        XCTAssertTrue(all.contains { $0["ev"] as? String == "engineStart" })
        XCTAssertTrue(all.contains { $0["t"] as? String == "mark" && $0["label"] as? String == "crackle" })
        XCTAssertTrue(all.contains { $0["ev"] as? String == "cmd" && $0["src"] as? String == "ui" })
        let st = rig.host.diagStatus()
        XCTAssertTrue(st.enabled)
        XCTAssertGreaterThan(st.bytes, 0)
        XCTAssertEqual(st.rideKey?.hasPrefix("lab-"), true)

        let zip = try rig.host.diagExport(scope: .last)
        let names = DiagZip.entries(try Data(contentsOf: zip)).map(\.name)
        XCTAssertEqual(names.first, "device.json")
        XCTAssertTrue(names.dropFirst().allSatisfy { $0.hasPrefix("diag/d-") && $0.hasSuffix(".jsonl") })
        XCTAssertTrue(zip.lastPathComponent.hasPrefix("convlink-diag-lab-"))

        rig.host.stop()
        waitUntil("engineStop written", timeout: 5) {
            self.lines().contains { $0["ev"] as? String == "engineStop" }
        }
        rig.host.diagClear()
        XCTAssertEqual(rig.host.diagStatus().sessions, 0)
    }

    func testDisabledRecordingKeepsTheRingOnlyAndReenablingStartsASegment() throws {
        rig.host.diagSetEnabled(false)
        let before = lines().count
        rig.host.diagNote(.bt(.off))
        XCTAssertEqual(lines().count, before, "nothing new reaches the disk while off")
        XCTAssertFalse(rig.host.diagStatus().enabled)
        XCTAssertTrue(rig.engine?.diagSnapshotRing().contains { $0.contains(#""ev":"bt""#) } == true)
        // Without files the export carries the RAM ring.
        rig.host.diagClear()
        let zip = try rig.host.diagExport(scope: .last)
        let entries = DiagZip.entries(try Data(contentsOf: zip))
        let ring = try XCTUnwrap(entries.first { $0.name == "diag/ring.jsonl" })
        XCTAssertTrue(String(decoding: ring.data, as: UTF8.self).contains(#""ev":"bt""#))
        // Back on: a new segment with its header, so later notes reach the disk.
        rig.host.diagSetEnabled(true)
        rig.host.diagNote(.bt(.on))
        let after = lines()
        XCTAssertEqual(after.first?["t"] as? String, "hdr")
        XCTAssertTrue(after.contains { $0["ev"] as? String == "bt" && $0["state"] as? String == "on" })
    }

    func testDefaultPreferenceFollowsTheFieldTestPeriod() {
        XCTAssertTrue(DiagPreference.defaultEnabled(appLabel: "0.1.0 (12)", debug: false))
        XCTAssertFalse(DiagPreference.defaultEnabled(appLabel: "1.0.0 (40)", debug: false))
        XCTAssertTrue(DiagPreference.defaultEnabled(appLabel: "1.2.0 (40)", debug: true))
    }
}
