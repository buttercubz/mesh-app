//! Voice pipeline (contract C8, DESIGN §6.4-§6.5, §7).
//!
//! Every processing step works on [`Block`]s: 10 ms of 16 kHz mono `i16` (160 samples),
//! the unit the engine tick moves between the real-time rings and the codecs.
//!
//! ```text
//! capture:  Block ─► CaptureChain (wind guard on raw, HPF 80 Hz, VAD + gate + 300 ms hangover)
//!                 ─► TalkFsm (pure; floor decisions injected) ─► PreRoll ─► Packetizer ─► VoiceFrame
//! playout:  VoiceFrame ─► ReceivePath (per-origin JitterBuffer + DecoderPool of 6)
//!                 ─► Mixer (≤ 4 streams, −3 dB when ≥ 2) + Earcons + PromptPlayer ─► Limiter ─► Block
//! ```
//!
//! * Codecs: [`Codec`] with [`SimCodec`] (always available, wasm/sim) and `OpusCodec`
//!   (feature `opus`, libopus 1.6.1 through `opus-sys`).
//! * Encoder settings: [`EncoderConfig::for_profile`] implements the §7.4 table.
//! * Audio check (onboarding / settings): [`AudioCheck`] with tone, 1 s echo and VOX meter.
//! * No allocation happens on the 10 ms paths (capture, packetize, receive, mix); a
//!   counting-allocator integration test proves it.
//!
//! Sans-IO crate: the determinism lints below are denied here (clippy.toml holds the
//! list). Time is always a caller-supplied `u64` in microseconds.
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]

pub mod agc;
pub mod audio_check;
pub mod capture;
pub mod codec;
pub mod dsp;
pub mod earcon;
pub mod fsm;
pub mod jitter;
pub mod mixer;
pub mod packetizer;
pub mod profile;
pub mod prompt;
pub mod receive;
pub mod tone;
pub mod vad;

pub use agc::Agc;
pub use audio_check::{
    AudioCheck, AudioCheckMode, AudioCheckPhase, AudioCheckSample, Echo, Tone, VoxMeter,
};
pub use capture::{CaptureChain, CaptureFrame, PreRoll};
#[cfg(feature = "opus")]
pub use codec::opus::OpusCodec;
pub use codec::sim::SimCodec;
pub use codec::{Codec, CodecError, VoiceDecoder, VoiceEncoder};
pub use dsp::{Biquad, HighPass80, WindGuard};
pub use earcon::{Earcon, Earcons};
pub use fsm::{TalkEnv, TalkFsm, TalkInput, TalkMode, TalkOutput, TalkOutputs, TalkState};
pub use jitter::{JitterBuffer, JitterStats, PullStatus};
pub use mixer::{Limiter, Mixer};
pub use packetizer::{Packetizer, VoiceCfg, VoiceFrame};
pub use profile::{
    AudioBandwidth, EncoderConfig, FrameDuration, Profile, ProfileInputs, ProfileSelector,
};
pub use prompt::{PromptBank, PromptBankError, PromptId, PromptPlayer};
pub use receive::{DecoderPool, ReceivePath, StreamKey};
pub use vad::{EnergyVad, RnnVad, Vad, VadEvent, VadGate, VadSensitivity};

/// Sample rate of every PCM buffer in this crate (DESIGN §3.6, `MESH_RT_SAMPLE_RATE`).
pub const SAMPLE_RATE: u32 = 16_000;
/// Samples in one 10 ms block.
pub const BLOCK_SAMPLES: usize = 160;
/// Duration of one block in microseconds.
pub const BLOCK_US: u64 = 10_000;
/// One 10 ms block of 16 kHz mono PCM.
pub type Block = [i16; BLOCK_SAMPLES];
/// Largest frame the pipeline ever builds (60 ms at 16 kHz). v1 profiles use 20/40 ms.
pub const MAX_FRAME_SAMPLES: usize = 960;
/// Largest Opus payload carried by a VOICE packet (DESIGN §4.6: `opus[1..=400]`).
pub const MAX_PAYLOAD: usize = 400;

/// Converts linear amplitude (full scale 1.0) to dBFS, floored at −120 dBFS.
pub fn to_dbfs(linear: f32) -> f32 {
    if linear <= 1e-6 {
        -120.0
    } else {
        20.0 * linear.log10()
    }
}

/// Converts dBFS to linear amplitude.
pub fn from_dbfs(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// RMS of a PCM slice in dBFS (full scale = 32768).
pub fn rms_dbfs(pcm: &[i16]) -> f32 {
    to_dbfs(rms_linear(pcm))
}

/// RMS of a PCM slice, normalised so full scale is 1.0.
pub fn rms_linear(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let sum: f64 = pcm.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    ((sum / pcm.len() as f64).sqrt() / 32768.0) as f32
}

/// Saturating float → i16 conversion (input in sample units).
#[inline]
pub fn sat_i16(x: f32) -> i16 {
    if x >= 32767.0 {
        32767
    } else if x <= -32768.0 {
        -32768
    } else {
        x.round() as i16
    }
}
