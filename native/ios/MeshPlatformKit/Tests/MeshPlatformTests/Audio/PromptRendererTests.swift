import AVFAudio
import Foundation
@testable import MeshPlatform
import XCTest

/// Prompt PCM path: TTS buffers (any rate/format) → 16 kHz mono Int16 for `on_prompt_pcm`.
final class PcmConverterTests: XCTestCase {
    private func sine(_ hz: Double, rate: Double, seconds: Double, amp: Float = 0.5) -> [Float] {
        (0..<Int(rate * seconds)).map { amp * Float(sin(2 * .pi * hz * Double($0) / rate)) }
    }

    private func rmsDb(_ x: [Int16]) -> Double {
        let r = sqrt(x.reduce(0.0) { $0 + Double($1) * Double($1) } / Double(max(1, x.count)))
        return 20 * log10(r / 32768)
    }

    func testResamplesTypicalTtsRatesTo16k() {
        for rate in [22_050.0, 24_000, 48_000, 16_000] {
            let out = PcmConverter.toInt16Mono16k(sine(440, rate: rate, seconds: 1), sampleRate: rate)
            XCTAssertEqual(Double(out.count), 16_000, accuracy: 400, "\(rate) Hz → 1 s at 16 kHz")
            XCTAssertEqual(rmsDb(Array(out.dropFirst(800).dropLast(800))), -9.03, accuracy: 0.6,
                           "level kept (0.5 peak sine = -9 dBFS RMS) at \(rate) Hz")
        }
    }

    func testFloatMonoReadsChannelZeroOfEachFormat() throws {
        let f32 = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 22_050, channels: 2,
                                              interleaved: false))
        let b = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: f32, frameCapacity: 4))
        b.frameLength = 4
        for i in 0..<4 {
            b.floatChannelData?[0][i] = Float(i) / 4
            b.floatChannelData?[1][i] = -1
        }
        XCTAssertEqual(PcmConverter.floatMono(b), [0, 0.25, 0.5, 0.75])
        let i16 = try XCTUnwrap(AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16_000, channels: 1,
                                              interleaved: false))
        let c = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: i16, frameCapacity: 2))
        c.frameLength = 2
        c.int16ChannelData?[0][0] = 16384
        c.int16ChannelData?[0][1] = -32768
        XCTAssertEqual(PcmConverter.floatMono(c), [0.5, -1])
        b.frameLength = 0
        XCTAssertEqual(PcmConverter.floatMono(b), [], "the end-of-utterance marker buffer")
    }

    func testEmptyInputGivesNoPcm() {
        XCTAssertEqual(PcmConverter.toInt16Mono16k([], sampleRate: 22_050), [])
        XCTAssertEqual(PcmConverter.toInt16Mono16k([0.1], sampleRate: 0), [])
    }

    func testUnknownVoiceFailsThatKeyOnly() {
        let failed = expectation(description: "failure reported")
        let r = PromptRenderer(timeout: 2, onPcm: { _, _ in XCTFail("no voice for this language") },
                               onFailure: { key in
                                   XCTAssertEqual(key, "alone")
                                   failed.fulfill()
                               })
        r.render([PromptRequest(key: "alone", text: "x", lang: "zz-ZZ")])
        wait(for: [failed], timeout: 8)
        XCTAssertNil(PromptRenderer.voice(for: "zz-ZZ"))
    }
}
