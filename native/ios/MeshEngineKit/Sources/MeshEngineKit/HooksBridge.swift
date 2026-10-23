@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform
import os

/// Renders spoken prompts (platform TTS, §7.7). `PromptRenderer` in production.
public protocol PromptRendering: AnyObject, Sendable {
    func render(_ requests: [PromptRequest])
}

extension PromptRenderer: PromptRendering {}

/// Builds the renderer with its result callbacks.
public typealias PromptRendererFactory = @Sendable (
    _ onPcm: @escaping PromptRenderer.PcmHandler,
    _ onFailure: @escaping PromptRenderer.FailureHandler
) -> any PromptRendering

/// The engine → platform calls (`PlatformHooks`, contract C4). Called on the engine thread:
/// every method returns quickly, never blocks on the main thread and never calls back into
/// `ui_command` (which would wait for this very thread).
final class HooksBridge: PlatformHooks {
    struct MainSink: Sendable {
        var uiEvent: @Sendable (String) -> Void
        var notify: @Sendable (_ key: String, _ paramsJson: String, _ isError: Bool) -> Void
    }

    let radio: RadioAdapter
    let audio: AudioAdapter
    let persistence: PersistAdapter
    let lab: LabAdapter
    let diag: DiagAdapter
    let hub: EventHub
    private let prompts: any PromptRendering
    private let main: MainSink
    private let logger = Logger(subsystem: PlatformLog.subsystem, category: "engine")
    private let logSink: (@Sendable (FfiLogLevel, String) -> Void)?

    init(radio: RadioAdapter, audio: AudioAdapter, persistence: PersistAdapter, lab: LabAdapter,
         diag: DiagAdapter, hub: EventHub, prompts: any PromptRendering, main: MainSink,
         logSink: (@Sendable (FfiLogLevel, String) -> Void)?) {
        self.radio = radio
        self.audio = audio
        self.persistence = persistence
        self.lab = lab
        self.diag = diag
        self.hub = hub
        self.prompts = prompts
        self.main = main
        self.logSink = logSink
    }

    func linkSend(link: UInt32, bytes: Data) -> FfiSendOutcome {
        radio.send(link, bytes)
    }

    func radio(cmd: FfiRadioCommand) {
        radio.handle(cmd)
    }

    func audio(cmd: FfiAudioCommand) {
        audio.handle(cmd)
    }

    func uiEvent(json: String) {
        hub.emit(json)
        if json.contains(#""labFinished""#), let id = CommandJSON.object(json)?["runId"] as? String {
            lab.finish(runId: id)
        }
        main.uiEvent(json)
    }

    func persist(key: String, value: Data?) {
        persistence.persist(key, value)
    }

    func notify(n: FfiNotification) {
        main.notify(n.key, n.paramsJson, n.category == .error)
    }

    func labWrite(runId: String, line: String) {
        lab.write(runId: runId, line: line)
    }

    func labWriteSummary(runId: String, line: String) {
        lab.writeSummary(runId: runId, line: line)
    }

    func labDelete(runId: String) {
        lab.delete(runId: runId)
    }

    func diagWrite(sessionId: String, seg: UInt32, lines: [String]) {
        diag.write(sessionId: sessionId, seg: seg, lines: lines)
    }

    func renderPrompts(reqs: [FfiPromptRequest]) {
        prompts.render(reqs.map { PromptRequest(key: $0.key, text: $0.text, lang: $0.lang) })
    }

    func log(level: FfiLogLevel, msg: String) {
        switch level {
        case .debug: logger.debug("\(msg, privacy: .public)")
        case .info: logger.info("\(msg, privacy: .public)")
        case .warn: logger.warning("\(msg, privacy: .public)")
        case .error: logger.error("\(msg, privacy: .public)")
        }
        logSink?(level, msg)
    }
}
