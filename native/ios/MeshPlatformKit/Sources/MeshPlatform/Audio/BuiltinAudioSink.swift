import CMeshAudio
import Foundation

/// Swift access to the C host's built-in sinks, used by Lab benches and the audio check
/// before (or without) the Rust core: meter, loopback, bench tone and the T5a click train
/// with capture (DESIGN §10.1.1, §12.2). An external (Rust) sink always takes precedence.
public enum BuiltinAudioSink {
    public enum Kind: Sendable, Equatable {
        case silence
        case meter
        case loopback
        case tone(hz: Float, dbfs: Float)
        /// One click every `periodMs`, capture recorded for `captureSeconds` (≤ 20 s).
        case click(periodMs: UInt32, captureSeconds: UInt32, dbfs: Float)
    }

    public static func select(_ kind: Kind) {
        switch kind {
        case .silence:
            cmesh_use_builtin(CMESH_BUILTIN_SILENCE)
        case .meter:
            cmesh_use_builtin(CMESH_BUILTIN_METER)
        case .loopback:
            cmesh_use_builtin(CMESH_BUILTIN_LOOPBACK)
        case .tone(let hz, let db):
            cmesh_tone_configure(hz, db)
            cmesh_use_builtin(CMESH_BUILTIN_TONE)
        case .click(let periodMs, let seconds, let db):
            let rate = CMESH_SAMPLE_RATE
            cmesh_click_configure(periodMs * rate / 1000, seconds * rate, db)
            cmesh_use_builtin(CMESH_BUILTIN_CLICK)
        }
    }

    /// Current capture and render block levels in dBFS (-120 = silence).
    public static func levels() -> (capture: Float, render: Float) {
        var c: Float = -120, r: Float = -120
        cmesh_meter_read(&c, &r)
        return (c, r)
    }

    /// T5a result: median click round trip in milliseconds and the number of clicks
    /// found, or nil when none was found above `thresholdDbfs`.
    public static func clickRoundTripMs(thresholdDbfs: Float = -30) -> (ms: Double, found: Int)? {
        var found: UInt32 = 0
        let frames = cmesh_click_estimate_latency_frames(thresholdDbfs, &found)
        guard frames >= 0 else { return nil }
        return (Double(frames) * 1000 / Double(CMESH_SAMPLE_RATE), Int(found))
    }

    /// Host IO counters (callbacks, frames, AudioUnitRender errors).
    public static func counters() -> CMeshIoCounters {
        var c = CMeshIoCounters()
        cmesh_io_counters(&c)
        return c
    }
}
