//! Demo-ride voice lines (DESIGN §3.4, §7.7). No audio asset ships: at Demo start the
//! core asks platform TTS to render the `demo.line1…8` prompts; each line's PCM is then
//! encoded **once** with the real encoder of the session's profile and the PCM is
//! dropped (it never counts against the 512 KB prompt bank). The demo world's bots
//! replay the encoded frames on their talk schedule through the normal VOICE path.

use std::collections::BTreeMap;

use mesh_api::{Lang, PromptKey};
use mesh_voice::{
    BLOCK_SAMPLES, Codec, CodecError, EncoderConfig, Packetizer, PreRoll, Profile, VoiceCfg,
    VoiceFrame,
};

use super::prompts::fill_template;
use crate::effects::PromptRequest;

/// The eight bot lines, in order.
pub const DEMO_LINES: [PromptKey; 8] = [
    PromptKey::DemoLine1,
    PromptKey::DemoLine2,
    PromptKey::DemoLine3,
    PromptKey::DemoLine4,
    PromptKey::DemoLine5,
    PromptKey::DemoLine6,
    PromptKey::DemoLine7,
    PromptKey::DemoLine8,
];

/// Render requests for every demo line that has a template.
pub fn demo_prompt_requests(
    templates: &BTreeMap<PromptKey, String>,
    lang: Lang,
) -> Vec<PromptRequest> {
    DEMO_LINES
        .iter()
        .filter_map(|k| {
            templates.get(k).map(|t| PromptRequest {
                key: *k,
                text: fill_template(t, None),
                lang,
            })
        })
        .collect()
}

/// One encoded frame of a line (burst-relative `ts`, VOICE `cfg`, codec payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoFrame {
    pub ts: u16,
    pub cfg: u8,
    pub payload: Vec<u8>,
}

impl DemoFrame {
    /// The frame as a VOICE payload of burst `burst`.
    pub fn voice(&self, burst: u8) -> Option<VoiceFrame> {
        VoiceFrame::from_parts(burst, self.ts, VoiceCfg(self.cfg), &self.payload)
    }
}

/// One encoded line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoLine {
    pub key: PromptKey,
    pub profile: Profile,
    pub frames: Vec<DemoFrame>,
}

impl DemoLine {
    /// Duration in ms.
    pub fn duration_ms(&self) -> u32 {
        self.frames.last().map_or(0, |f| {
            u32::from(f.ts) * 10 + VoiceCfg(f.cfg).duration().ms()
        })
    }
}

/// Every encoded line of a demo ride.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DemoLines {
    lines: Vec<DemoLine>,
}

impl DemoLines {
    /// Encodes each line once with `codec` at `profile` (WB capture), consuming and
    /// dropping the PCM.
    pub fn encode(
        codec: &dyn Codec,
        profile: Profile,
        pcm_lines: Vec<(PromptKey, Vec<i16>)>,
    ) -> Result<Self, CodecError> {
        let cfg = EncoderConfig::for_profile(profile, mesh_voice::SAMPLE_RATE, 0.0);
        let mut packetizer = Packetizer::new(codec.new_encoder()?);
        let mut lines = Vec::with_capacity(pcm_lines.len());
        for (key, pcm) in pcm_lines {
            let mut frames = Vec::new();
            let mut emit = |f: &VoiceFrame| {
                frames.push(DemoFrame {
                    ts: f.ts,
                    cfg: f.cfg.0,
                    payload: f.opus().to_vec(),
                });
            };
            packetizer.start_burst(0, &cfg, &PreRoll::new(), &mut emit)?;
            for chunk in pcm.chunks(BLOCK_SAMPLES) {
                let mut block = [0i16; BLOCK_SAMPLES];
                block[..chunk.len()].copy_from_slice(chunk);
                packetizer.push_block(&block, &mut emit)?;
            }
            packetizer.finish_burst(&mut emit)?;
            drop(pcm);
            lines.push(DemoLine {
                key,
                profile,
                frames,
            });
        }
        Ok(DemoLines { lines })
    }

    pub fn lines(&self) -> &[DemoLine] {
        &self.lines
    }

    pub fn line(&self, key: PromptKey) -> Option<&DemoLine> {
        self.lines.iter().find(|l| l.key == key)
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Encoded bytes kept (≈ 80 KB for 8 × 5 s at 16 kbps).
    pub fn encoded_bytes(&self) -> usize {
        self.lines
            .iter()
            .flat_map(|l| l.frames.iter())
            .map(|f| f.payload.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_voice::SimCodec;

    #[test]
    fn lines_are_encoded_once_and_requests_follow_templates() {
        let mut t = BTreeMap::new();
        t.insert(PromptKey::DemoLine1, "Hola {{name}}".to_string());
        t.insert(PromptKey::DemoLine4, "Curva".to_string());
        t.insert(PromptKey::Alone, "Solo".to_string());
        let r = demo_prompt_requests(&t, Lang::Es);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].key, PromptKey::DemoLine1);
        assert_eq!(r[0].text, "Hola ");
        let pcm: Vec<i16> = (0..16_000 * 2)
            .map(|i| ((i as f32 * 0.07).sin() * 8_000.0) as i16)
            .collect();
        let lines = DemoLines::encode(
            &SimCodec::new(),
            Profile::Ble16,
            vec![
                (PromptKey::DemoLine1, pcm.clone()),
                (PromptKey::DemoLine4, pcm),
            ],
        )
        .unwrap();
        assert_eq!(lines.len(), 2);
        let l = lines.line(PromptKey::DemoLine1).unwrap();
        assert_eq!(l.frames.len(), 50);
        assert_eq!(l.duration_ms(), 2_000);
        assert!(VoiceCfg(l.frames[0].cfg).first());
        assert_eq!(l.frames[3].ts, 12);
        let v = l.frames[1].voice(9).unwrap();
        assert_eq!((v.burst, v.ts), (9, 4));
        // ≈ 16 kbps: 2 s ≈ 4 KB per line (SimCodec tracks the profile bitrate).
        let per_line = lines.encoded_bytes() / 2;
        assert!((3_000..6_000).contains(&per_line), "{per_line}");
    }
}
