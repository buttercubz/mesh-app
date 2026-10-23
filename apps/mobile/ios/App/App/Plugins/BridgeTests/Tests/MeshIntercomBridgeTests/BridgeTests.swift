import Capacitor
import Foundation
import MeshEngineKit
import MeshPlatform
import UIKit
import XCTest
@testable import MeshIntercomBridge

/// Bridge contract tests (DESIGN §11.1, WP-19): `send` rejects every ★ command,
/// `gestureCommand` rejects when the app is not foreground-active, and
/// `setKeepScreenOn(false)` runs on every background transition.
final class BridgeTests: XCTestCase {
    /// Outcome of one plugin call.
    enum Outcome: Equatable {
        case resolved([String: String])
        case rejected(code: String?, message: String)
    }

    /// Calls `method` with `options` and returns how the call settled (synchronously
    /// settled calls only; every gate under test answers before any asynchronous work).
    func invoke(_ plugin: MeshIntercomPlugin, _ name: String, _ options: [String: Any],
                _ method: (MeshIntercomPlugin) -> (CAPPluginCall) -> Void) -> Outcome? {
        var outcome: Outcome?
        let call = CAPPluginCall(callbackId: "t", methodName: name, options: options,
                                 success: { result, _ in
                                     var flat: [String: String] = [:]
                                     for (k, v) in result?.data ?? [:] { flat[k] = "\(v)" }
                                     outcome = .resolved(flat)
                                 },
                                 error: { error in
                                     outcome = .rejected(code: error?.code, message: error?.message ?? "")
                                 })!
        method(plugin)(call)
        return outcome
    }

    /// `{name, requiresGesture, command}` of the shared mesh-api fixtures.
    func fixtures() throws -> [(name: String, gesture: Bool, json: String)] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        var repo = root
        for _ in 0..<7 { repo = repo.deletingLastPathComponent() }
        let url = repo.appending(path: "crates/mesh-api/fixtures/commands.json")
        let data = try Data(contentsOf: url)
        let list = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        return try list.map { e in
            let cmd = try XCTUnwrap(e["command"])
            let json = String(decoding: try JSONSerialization.data(withJSONObject: cmd), as: UTF8.self)
            return (e["name"] as? String ?? "?", e["requiresGesture"] as? Bool ?? false, json)
        }
    }

    // MARK: send

    func testSendRejectsEveryGestureCommandWithUseGestureCommand() throws {
        let plugin = MeshIntercomPlugin()
        let all = try fixtures()
        let gated = all.filter(\.gesture)
        XCTAssertGreaterThanOrEqual(gated.count, 4, "fixtures cover goLive, resumeMic, labStart, audioCheck")
        for f in gated {
            let out = invoke(plugin, "send", ["command": f.json], MeshIntercomPlugin.send)
            XCTAssertEqual(out, .rejected(code: "useGestureCommand", message: "useGestureCommand"), f.name)
        }
        // The hand-written ★ forms the UI sends, including the conditional ones.
        for json in [#"{"type":"goLive"}"#, #"{"type":"resumeMic"}"#,
                     #"{"type":"labStart","bench":"T0","params":{"audio":"full"}}"#,
                     #"{"type":"labStart","bench":"T0","params":{"audio":"renderOnly"}}"#,
                     #"{"type":"audioCheck","op":"start","mode":"tone"}"#] {
            let out = invoke(plugin, "send", ["command": json], MeshIntercomPlugin.send)
            XCTAssertEqual(out, .rejected(code: "useGestureCommand", message: "useGestureCommand"), json)
        }
    }

    func testSendLetsPlainCommandsThroughToTheEngine() throws {
        // No engine host in this process: a plain command passes the gate and is refused
        // only for the missing engine, never as a gesture command.
        let plugin = MeshIntercomPlugin()
        for f in try fixtures() where !f.gesture {
            let out = invoke(plugin, "send", ["command": f.json], MeshIntercomPlugin.send)
            XCTAssertEqual(out, .rejected(code: "notSupported", message: "notSupported"), f.name)
        }
        for json in [#"{"type":"labStart","bench":"T1","params":{"audio":"off"}}"#,
                     #"{"type":"audioCheck","op":"stop"}"#] {
            let out = invoke(plugin, "send", ["command": json], MeshIntercomPlugin.send)
            XCTAssertNotEqual(out, .rejected(code: "useGestureCommand", message: "useGestureCommand"), json)
        }
    }

    func testSendWithoutCommandIsInvalidParams() {
        let out = invoke(MeshIntercomPlugin(), "send", [:], MeshIntercomPlugin.send)
        XCTAssertEqual(out, .rejected(code: "invalidParams", message: "invalidParams"))
    }

    // MARK: gestureCommand

    func testGestureCommandRejectsWhenNotForegroundActive() throws {
        let plugin = MeshIntercomPlugin()
        plugin.foregroundCheck = { false }
        for f in try fixtures() where f.gesture {
            let out = invoke(plugin, "gestureCommand", ["command": f.json], MeshIntercomPlugin.gestureCommand)
            XCTAssertEqual(out, .rejected(code: "notForeground", message: "notForeground"), f.name)
        }
        XCTAssertEqual(invoke(plugin, "goLive", [:], MeshIntercomPlugin.goLive),
                       .rejected(code: "notForeground", message: "notForeground"))
        XCTAssertEqual(invoke(plugin, "resumeMic", [:], MeshIntercomPlugin.resumeMic),
                       .rejected(code: "notForeground", message: "notForeground"))
        XCTAssertEqual(invoke(plugin, "startEngine", ["lab": ["code": "4711"]], MeshIntercomPlugin.startEngine),
                       .rejected(code: "notForeground", message: "notForeground"))
    }

    func testGestureCommandChecksForegroundInsideTheCall() {
        // Foreground-active: the gate passes and the call reaches the (missing) engine.
        let plugin = MeshIntercomPlugin()
        var asked = 0
        plugin.foregroundCheck = { asked += 1; return true }
        let out = invoke(plugin, "gestureCommand", ["command": #"{"type":"goLive"}"#],
                         MeshIntercomPlugin.gestureCommand)
        XCTAssertEqual(asked, 1)
        XCTAssertEqual(out, .rejected(code: "notSupported", message: "notSupported"))
    }

    // MARK: startEngine options

    func testSessionStartOptions() {
        func start(_ o: [String: Any]) -> SessionStart? {
            MeshIntercomPlugin.sessionStart(CAPPluginCall(callbackId: "t", methodName: "startEngine", options: o,
                                                         success: { _, _ in }, error: { _ in })!)
        }
        XCTAssertEqual(start(["demo": true]), .demo)
        XCTAssertEqual(start(["lab": ["code": "4711"]]), .lab(code: 4711))
        XCTAssertEqual(start(["lab": ["code": "0007"]]), .lab(code: 7))
        XCTAssertNil(start(["lab": ["code": "12345"]]))
        XCTAssertNil(start(["lab": ["code": "12a4"]]))
        XCTAssertEqual(start(["groupId": "00112233445566778899aabbccddeeff"]),
                       SessionStart.group(hex: "00112233445566778899aabbccddeeff"))
        XCTAssertNil(start(["groupId": "abc"]))
        XCTAssertNil(start([:]))
    }

    func testEngineErrorsMapToContractCodes() {
        XCTAssertEqual(MeshIntercomPlugin.errorCode(EngineHostError.notForeground), "notForeground")
        XCTAssertEqual(MeshIntercomPlugin.errorCode(EngineHostError.apiVersionMismatch(engine: 0x0200)), "apiVersionMismatch")
        XCTAssertEqual(MeshIntercomPlugin.errorCode(EngineHostError.engine("UnknownGroup")), "unknownGroup")
        XCTAssertEqual(MeshIntercomPlugin.errorCode(EngineHostError.engine(#"InvalidStart(detail: "lab")"#)), "invalidParams")
        XCTAssertEqual(MeshIntercomPlugin.errorCode(EngineHostError.engine("DemoUnavailable")), "notSupported")
        XCTAssertEqual(MeshIntercomPlugin.errorCode(EngineHostError.engine("Crashed")), "engineCrashed")
        XCTAssertEqual(MeshIntercomPlugin.errorCode(EngineHostError.engine("ThreadSpawn(detail: x)")), "internal")
    }

    // MARK: Keep screen on

    @MainActor
    final class FakeIdleTimer: IdleTimerPort {
        var isIdleTimerDisabled = false
    }

    @MainActor
    func testSetKeepScreenOnIsResetOnEveryBackgroundTransition() {
        let plugin = MeshIntercomPlugin()
        let port = FakeIdleTimer()
        let center = NotificationCenter()
        plugin.idleTimer = IdleTimerController(port: port, center: center)
        for _ in 0..<2 {
            XCTAssertEqual(invoke(plugin, "setKeepScreenOn", ["on": true], MeshIntercomPlugin.setKeepScreenOn),
                           .resolved([:]))
            XCTAssertTrue(port.isIdleTimerDisabled)
            center.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
            XCTAssertFalse(port.isIdleTimerDisabled, "background transition turns keep-screen-on off")
        }
        // The bridge's own background handler resets it as well.
        _ = invoke(plugin, "setKeepScreenOn", ["on": true], MeshIntercomPlugin.setKeepScreenOn)
        plugin.didEnterBackground()
        XCTAssertFalse(port.isIdleTimerDisabled)
        XCTAssertEqual(invoke(plugin, "setKeepScreenOn", [:], MeshIntercomPlugin.setKeepScreenOn),
                       .rejected(code: "invalidParams", message: "invalidParams"))
    }

    // MARK: Misc

    func testGetInfoWithoutEngineIsPlatformOnly() {
        let out = invoke(MeshIntercomPlugin(), "getInfo", [:], MeshIntercomPlugin.getInfo)
        guard case .resolved(let info)? = out else { return XCTFail("\(String(describing: out))") }
        XCTAssertEqual(info["platform"], "ios")
        XCTAssertEqual(info["engine"], "platform-only")
        XCTAssertEqual(info["apiVersion"], MeshIntercomPlugin.apiVersion)
    }

    func testLabFilesOutsideTheLabDirectoryCannotBeShared() {
        XCTAssertNil(MeshIntercomPlugin.labFileURL("../Library/secret.jsonl"))
        XCTAssertNil(MeshIntercomPlugin.labFileURL("/etc/passwd"))
        XCTAssertNil(MeshIntercomPlugin.labFileURL("r-missing.jsonl"))
        XCTAssertNil(MeshIntercomPlugin.labFileURL(".hidden.csv"))
    }

    func testBatteryExemptionIsANoOpOnIos() {
        XCTAssertEqual(invoke(MeshIntercomPlugin(), "requestBatteryExemption", [:],
                              MeshIntercomPlugin.requestBatteryExemption),
                       .resolved(["granted": "true"]))
    }
}
