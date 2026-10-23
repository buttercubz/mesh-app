//! What the Demo bots say (DESIGN §3.4): the `demo.line1…8` prompts rendered by platform
//! TTS, each encoded **once** with the real encoder of the session's codec (the PCM is
//! dropped and only the encoded frames are kept, ≈ 80 KB for 8 × 5 s), or, when no PCM
//! is supplied (iOS Simulator without voices, WASM), the deterministic speech-like tone
//! pattern of [`crate::synth`]. No audio asset is bundled.
//!
//! A bot speaks an utterance through its own engine: the kept frames are decoded back
//! into 10 ms blocks at talk time and pushed into the bot's capture path, so its real
//! capture chain, talk FSM, floor and VOICE path run exactly as on a phone.

use std::collections::VecDeque;

use mesh_api::PromptKey;
use mesh_core::media::demo::{DEMO_LINES, DemoLines};
use mesh_core::{BLOCK_SAMPLES, Platform};
use mesh_voice::{Codec, Profile, SimCodec, VoiceCfg};

use crate::synth::SpeechSynth;

/// 16 kHz mono PCM of one rendered line.
pub type Pcm = Vec<i16>;

/// One 10 ms block.
pub type Block = [i16; BLOCK_SAMPLES];

/// The codec the engines of `platform` use (mirrors mesh-core's media module: libopus on
/// devices with the `opus` feature, SimCodec in the browser and without libopus).
pub fn codec_for(platform: Platform) -> Box<dyn Codec> {
    #[cfg(feature = "opus")]
    if platform != Platform::WebSim {
        return Box::new(mesh_voice::OpusCodec::new());
    }
    let _ = platform;
    Box::new(SimCodec::new())
}

/// The bots' voice material.
pub enum DemoVoice {
    /// Encoded TTS lines and the codec to decode them at talk time.
    Lines {
        lines: DemoLines,
        codec: Box<dyn Codec>,
    },
    /// No PCM: synthesized speech-like utterances.
    Synth { synth: Box<SpeechSynth> },
}

impl DemoVoice {
    /// Encodes `pcm` (line `k` is `demo.line{k+1}`) once with `platform`'s codec, or
    /// falls back to the synth when `pcm` is `None`, empty or does not encode.
    pub fn new(platform: Platform, pcm: Option<Vec<Pcm>>, seed: u64) -> DemoVoice {
        let synth = || DemoVoice::Synth {
            synth: Box::new(SpeechSynth::new(seed, 0xDE30)),
        };
        let Some(pcm) = pcm.filter(|v| v.iter().any(|p| !p.is_empty())) else {
            return synth();
        };
        let keyed: Vec<(PromptKey, Pcm)> = DEMO_LINES
            .iter()
            .copied()
            .zip(pcm)
            .filter(|(_, p)| !p.is_empty())
            .collect();
        let codec = codec_for(platform);
        match DemoLines::encode(codec.as_ref(), Profile::Ble16, keyed) {
            Ok(lines) if !lines.is_empty() => DemoVoice::Lines { lines, codec },
            _ => synth(),
        }
    }

    /// `true` when the bots speak rendered TTS lines.
    pub fn has_lines(&self) -> bool {
        matches!(self, DemoVoice::Lines { .. })
    }

    /// Encoded bytes kept (0 for the synth).
    pub fn encoded_bytes(&self) -> usize {
        match self {
            DemoVoice::Lines { lines, .. } => lines.encoded_bytes(),
            DemoVoice::Synth { .. } => 0,
        }
    }

    /// The `n`-th utterance as 10 ms blocks (lines cycle; the synth makes 2.5-4.5 s).
    pub fn utterance(&mut self, n: usize, synth_ms: u32) -> VecDeque<Block> {
        match self {
            DemoVoice::Lines { lines, codec } => {
                let l = &lines.lines()[n % lines.len()];
                decode_line(codec.as_ref(), l)
            }
            DemoVoice::Synth { synth } => {
                synth.restart();
                let pcm = synth.render_ms(synth_ms);
                to_blocks(&pcm)
            }
        }
    }
}

fn to_blocks(pcm: &[i16]) -> VecDeque<Block> {
    pcm.chunks(BLOCK_SAMPLES)
        .map(|c| {
            let mut b = [0i16; BLOCK_SAMPLES];
            b[..c.len()].copy_from_slice(c);
            b
        })
        .collect()
}

/// Decodes the kept frames of one line back into blocks (placed at their `ts`).
fn decode_line(codec: &dyn Codec, line: &mesh_core::media::demo::DemoLine) -> VecDeque<Block> {
    let Ok(mut dec) = codec.new_decoder() else {
        return VecDeque::new();
    };
    let total = line.duration_ms() as usize * 16;
    let mut pcm = vec![0i16; total];
    let mut buf = vec![0i16; mesh_voice::MAX_FRAME_SAMPLES];
    for f in &line.frames {
        // The decoder renders exactly the frame's duration (the output buffer's size).
        let dur = (VoiceCfg(f.cfg).duration().ms() as usize * 16).min(buf.len());
        let Ok(n) = dec.decode(&f.payload, &mut buf[..dur]) else {
            continue;
        };
        let at = usize::from(f.ts) * BLOCK_SAMPLES;
        let n = n.min(dur).min(total.saturating_sub(at));
        pcm[at..at + n].copy_from_slice(&buf[..n]);
    }
    to_blocks(&pcm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(ms: usize) -> Pcm {
        (0..ms * 16)
            .map(|i| ((i as f32 * 0.09).sin() * 7_000.0) as i16)
            .collect()
    }

    #[test]
    fn lines_are_kept_encoded_and_decode_back() {
        let mut v = DemoVoice::new(Platform::WebSim, Some(vec![tone(1_000), tone(500)]), 1);
        assert!(v.has_lines());
        let kept = v.encoded_bytes();
        // ≈ 16 kbps: 1.5 s ≈ 3 KB, far below the 48 KB of PCM.
        assert!(kept > 1_000 && kept < 6_000, "{kept}");
        let u = v.utterance(0, 0);
        assert_eq!(u.len(), 100);
        let loud = u
            .iter()
            .filter(|b| b.iter().any(|s| s.unsigned_abs() > 1_000))
            .count();
        assert!(loud > 80, "{loud}");
        // 500 ms in whole 40 ms frames.
        assert_eq!(v.utterance(1, 0).len(), 52);
        assert_eq!(v.utterance(2, 0).len(), 100, "lines cycle");
    }

    #[test]
    fn without_pcm_the_bots_use_the_synth() {
        let mut v = DemoVoice::new(Platform::Ios, None, 3);
        assert!(!v.has_lines());
        assert_eq!(v.encoded_bytes(), 0);
        let u = v.utterance(0, 3_000);
        assert_eq!(u.len(), 300);
        let mut w = DemoVoice::new(Platform::Ios, Some(vec![Vec::new()]), 3);
        assert!(!w.has_lines(), "empty renders fall back to the synth");
        assert_eq!(u, w.utterance(0, 3_000), "deterministic");
    }
}
