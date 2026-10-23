import AVFAudio
import Foundation

/// One spoken prompt to render (`FfiPromptRequest {key, text, lang}`, DESIGN §7.7).
public struct PromptRequest: Sendable, Equatable {
    public var key: String
    public var text: String
    /// BCP 47 language ("es", "en").
    public var lang: String

    public init(key: String, text: String, lang: String) {
        self.key = key
        self.text = text
        self.lang = lang
    }
}

/// Renders prompts with `AVSpeechSynthesizer.write(_:toBufferCallback:)` into 16 kHz mono
/// Int16 PCM for `on_prompt_pcm` (§7.7). Requests are rendered one at a time, in order. A
/// failure (no voice, TTS unavailable in the background, timeout) is reported per key so
/// the core falls back to that prompt's earcon.
public final class PromptRenderer: @unchecked Sendable {
    public typealias PcmHandler = @Sendable (_ key: String, _ pcm: [Int16]) -> Void
    public typealias FailureHandler = @Sendable (_ key: String) -> Void

    // `synth`, `pending`, `current` and `collected` are confined to `queue`.
    private let queue = DispatchQueue(label: "mesh.prompts", qos: .utility)
    private let synth = AVSpeechSynthesizer()
    private var pending: [PromptRequest] = []
    private var current: PromptRequest?
    private var collected: [Float] = []
    private var collectedRate: Double = 0
    private var generation = 0
    private let onPcm: PcmHandler
    private let onFailure: FailureHandler
    private let timeout: TimeInterval

    public init(timeout: TimeInterval = 15, onPcm: @escaping PcmHandler,
                onFailure: @escaping FailureHandler) {
        self.timeout = timeout
        self.onPcm = onPcm
        self.onFailure = onFailure
    }

    /// Queues `requests` (`render_prompts`).
    public func render(_ requests: [PromptRequest]) {
        queue.async { [self] in
            pending += requests
            next()
        }
    }

    private func next() {
        guard current == nil, !pending.isEmpty else { return }
        let req = pending.removeFirst()
        current = req
        collected = []
        collectedRate = 0
        generation += 1
        let gen = generation
        let utterance = AVSpeechUtterance(string: req.text)
        guard let voice = Self.voice(for: req.lang) else {
            finish(gen, ok: false)
            return
        }
        utterance.voice = voice
        synth.write(utterance) { [weak self] buffer in
            guard let self, let pcm = buffer as? AVAudioPCMBuffer else { return }
            let samples = PcmConverter.floatMono(pcm)
            let rate = pcm.format.sampleRate
            let end = pcm.frameLength == 0
            self.queue.async { self.append(gen, samples: samples, rate: rate, end: end) }
        }
        queue.asyncAfter(deadline: .now() + timeout) { [weak self] in self?.finish(gen, ok: false) }
    }

    /// A voice whose primary language matches (`es` text is never read by an `en` voice).
    static func voice(for lang: String) -> AVSpeechSynthesisVoice? {
        let primary = { (s: String) in s.split(whereSeparator: { $0 == "-" || $0 == "_" }).first.map(String.init)?.lowercased() }
        guard let want = primary(lang), let v = AVSpeechSynthesisVoice(language: lang),
              primary(v.language) == want else { return nil }
        return v
    }

    private func append(_ gen: Int, samples: [Float], rate: Double, end: Bool) {
        guard gen == generation, current != nil else { return }
        if !samples.isEmpty {
            collectedRate = rate
            collected += samples
        }
        if end { finish(gen, ok: !collected.isEmpty) }
    }

    private func finish(_ gen: Int, ok: Bool) {
        guard gen == generation, let req = current else { return }
        current = nil
        let pcm = ok ? PcmConverter.toInt16Mono16k(collected, sampleRate: collectedRate) : []
        collected = []
        if pcm.isEmpty {
            PlatformLog.audio.notice("prompt \(req.key, privacy: .public) not rendered; earcon fallback")
            onFailure(req.key)
        } else {
            onPcm(req.key, pcm)
        }
        next()
    }
}

/// PCM helpers for prompt rendering.
public enum PcmConverter {
    public static let targetRate: Double = 16_000

    /// Channel 0 of a PCM buffer as Float in [-1, 1] (Float32, Int16 or Int32 formats).
    public static func floatMono(_ b: AVAudioPCMBuffer) -> [Float] {
        let n = Int(b.frameLength)
        guard n > 0 else { return [] }
        if let f = b.floatChannelData {
            return Array(UnsafeBufferPointer(start: f[0], count: n))
        }
        if let i = b.int16ChannelData {
            return UnsafeBufferPointer(start: i[0], count: n).map { Float($0) / 32768 }
        }
        if let i = b.int32ChannelData {
            return UnsafeBufferPointer(start: i[0], count: n).map { Float($0) / 2_147_483_648 }
        }
        return []
    }

    /// Resamples mono Float samples to 16 kHz Int16 with AVAudioConverter.
    public static func toInt16Mono16k(_ samples: [Float], sampleRate: Double) -> [Int16] {
        guard !samples.isEmpty, sampleRate > 0,
              let inFmt = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: sampleRate,
                                        channels: 1, interleaved: false),
              let outFmt = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: targetRate,
                                         channels: 1, interleaved: false),
              let conv = AVAudioConverter(from: inFmt, to: outFmt),
              let inBuf = AVAudioPCMBuffer(pcmFormat: inFmt, frameCapacity: AVAudioFrameCount(samples.count))
        else { return [] }
        inBuf.frameLength = AVAudioFrameCount(samples.count)
        if let dst = inBuf.floatChannelData?[0] {
            samples.withUnsafeBufferPointer { src in
                if let base = src.baseAddress { dst.update(from: base, count: samples.count) }
            }
        }
        let cap = AVAudioFrameCount(Double(samples.count) * targetRate / sampleRate) + 1024
        guard let outBuf = AVAudioPCMBuffer(pcmFormat: outFmt, frameCapacity: cap) else { return [] }
        // The input block runs synchronously inside `convert`, on this thread.
        final class Fed: @unchecked Sendable { var done = false }
        let fed = Fed()
        var err: NSError?
        let status = conv.convert(to: outBuf, error: &err) { _, outStatus in
            if !fed.done {
                fed.done = true
                outStatus.pointee = .haveData
                return inBuf
            }
            outStatus.pointee = .endOfStream
            return nil
        }
        guard status != .error, err == nil, let out = outBuf.int16ChannelData else { return [] }
        return Array(UnsafeBufferPointer(start: out[0], count: Int(outBuf.frameLength)))
    }
}
