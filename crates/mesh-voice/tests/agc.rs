//! Capture AGC (field fix 2026-10-23: quiet built-in tablet mic). Quiet speech must leave
//! the capture chain at a sane level in PTT and open VOX-medium; loud input must pass
//! untouched; noise alone must not be pumped up.

mod common;

use common::{Noise, blocks, mix, read_wav, sine, speech_labels};
use mesh_voice::{
    Agc, BLOCK_SAMPLES, Block, CaptureChain, EnergyVad, RnnVad, Vad, VadSensitivity, rms_dbfs,
};

/// Scales `pcm` so its speech-labelled spans have `level_dbfs` RMS.
fn scale_speech(pcm: &[i16], labels: &[(usize, usize)], level_dbfs: f32) -> Vec<i16> {
    let speech: Vec<i16> = labels
        .iter()
        .flat_map(|&(a, z)| pcm[a..z.min(pcm.len())].iter().copied())
        .collect();
    let g = 10f32.powf((level_dbfs - rms_dbfs(&speech)) / 20.0);
    pcm.iter()
        .map(|&x| (f32::from(x) * g).clamp(-32768.0, 32767.0) as i16)
        .collect()
}

struct Run {
    /// RMS (dBFS) of the chain output over speech blocks in the second half.
    speech_out_db: f32,
    /// Fraction of scored speech blocks with the VOX gate open.
    vox_recall: f32,
    /// Chain output, concatenated (what PTT/VOX hand to the encoder).
    out: Vec<i16>,
}

fn run(vad: Box<dyn Vad>, pcm: &[i16], labels: &[(usize, usize)], agc: bool) -> Run {
    let mut chain = CaptureChain::new(vad, VadSensitivity::Medium);
    chain.set_agc_enabled(agc);
    let mut out = Vec::with_capacity(pcm.len());
    let mut speech_out = Vec::new();
    let (mut open, mut total) = (0usize, 0usize);
    let half = pcm.len() / 2;
    for (i, mut b) in blocks(pcm).into_iter().enumerate() {
        let f = chain.process(&mut b);
        out.extend_from_slice(&b);
        let (a, z) = (i * BLOCK_SAMPLES, (i + 1) * BLOCK_SAMPLES);
        if labels.iter().any(|&(s0, s1)| a >= s0 + 480 && z <= s1) {
            total += 1;
            open += usize::from(f.gate_open);
            if a >= half {
                speech_out.extend_from_slice(&b);
            }
        }
    }
    Run {
        speech_out_db: rms_dbfs(&speech_out),
        vox_recall: open as f32 / total.max(1) as f32,
        out,
    }
}

fn quiet_speech(level_dbfs: f32) -> (Vec<i16>, Vec<(usize, usize)>) {
    let labels = speech_labels();
    let speech = scale_speech(&read_wav("speech.wav"), &labels, level_dbfs);
    let noise = Noise::new(11).pcm(speech.len(), level_dbfs - 22.0);
    (mix(&speech, &noise, 0.0), labels)
}

#[test]
fn quiet_speech_reaches_a_sane_level_for_ptt() {
    for level in [-40.0, -35.0] {
        let (pcm, labels) = quiet_speech(level);
        let off = run(Box::new(EnergyVad::new()), &pcm, &labels, false);
        let on = run(Box::new(EnergyVad::new()), &pcm, &labels, true);
        eprintln!(
            "{level} dBFS: out {:.1} (no AGC {:.1})",
            on.speech_out_db, off.speech_out_db
        );
        assert!(off.speech_out_db < level + 1.0, "baseline stays quiet");
        assert!(
            (-28.0..=-16.0).contains(&on.speech_out_db),
            "{level} dBFS speech left at {:.1} dBFS",
            on.speech_out_db
        );
        let peak = on
            .out
            .iter()
            .map(|s| f32::from(*s).abs())
            .fold(0.0, f32::max);
        assert!(peak <= Agc::PEAK_LIMIT + 1.0, "boost clipped: peak {peak}");
    }
}

#[test]
fn quiet_speech_opens_vox_medium() {
    for level in [-40.0, -35.0] {
        let (pcm, labels) = quiet_speech(level);
        let rnn = run(Box::new(RnnVad::new()), &pcm, &labels, true);
        let energy = run(Box::new(EnergyVad::new()), &pcm, &labels, true);
        eprintln!(
            "{level} dBFS: VOX recall rnn {:.3} energy {:.3}",
            rnn.vox_recall, energy.vox_recall
        );
        assert!(rnn.vox_recall >= 0.85, "rnn recall {:.3}", rnn.vox_recall);
        assert!(
            energy.vox_recall >= 0.85,
            "energy recall {:.3}",
            energy.vox_recall
        );
    }
}

#[test]
fn loud_input_passes_untouched_and_never_clips() {
    let tone = sine(440.0, -3.0, 16_000);
    let mut agc = Agc::new();
    let mut max_in = 0i32;
    let mut max_out = 0i32;
    for b in blocks(&tone) {
        let mut out: Block = b;
        agc.process(&mut out, false);
        max_in = max_in.max(b.iter().map(|s| i32::from(*s).abs()).max().unwrap());
        max_out = max_out.max(out.iter().map(|s| i32::from(*s).abs()).max().unwrap());
        assert_eq!(out, b, "loud blocks are not modified");
    }
    assert_eq!(max_out, max_in);
    assert_eq!(agc.gain_db(), 0.0);

    // Quiet speech then a sudden loud burst: the boost backs off instead of clipping.
    let (quiet, _) = quiet_speech(-40.0);
    let loud = sine(1_000.0, -6.0, 8_000);
    let mut agc = Agc::new();
    for mut b in blocks(&quiet) {
        agc.process(&mut b, false);
    }
    assert!(agc.gain_db() > 10.0, "learnt a boost: {}", agc.gain_db());
    for mut b in blocks(&loud) {
        agc.process(&mut b, false);
        let peak = b.iter().map(|s| f32::from(*s).abs()).fold(0.0, f32::max);
        assert!(peak <= Agc::PEAK_LIMIT + 1.0, "peak {peak}");
    }
}

#[test]
fn noise_alone_is_not_pumped() {
    let noise = Noise::new(5).pcm(16_000 * 5, -60.0);
    let mut agc = Agc::new();
    let mut out = Vec::new();
    for mut b in blocks(&noise) {
        agc.process(&mut b, false);
        out.extend_from_slice(&b);
    }
    assert!(agc.gain_db() < 1.0, "gain {}", agc.gain_db());
    assert!(rms_dbfs(&out) < -58.0);
}

#[test]
fn gain_holds_in_pauses_and_respects_the_noise_ceiling() {
    let (pcm, labels) = quiet_speech(-40.0);
    let mut agc = Agc::new();
    let last_end = labels.last().unwrap().1;
    for mut b in blocks(&pcm[..last_end]) {
        agc.process(&mut b, false);
    }
    let learnt = agc.gain_db();
    // The boosted noise floor stays under the ceiling.
    assert!(agc.noise_floor_db() + learnt <= Agc::NOISE_CEIL_DBFS + 0.5);
    let silence = Noise::new(3).pcm(16_000 * 2, -62.0);
    for mut b in blocks(&silence) {
        agc.process(&mut b, false);
    }
    assert_eq!(agc.gain_db(), learnt, "no pumping during a pause");
}

#[test]
fn windy_blocks_do_not_train_and_bypass_is_transparent() {
    let (pcm, _) = quiet_speech(-40.0);
    let mut agc = Agc::new();
    for mut b in blocks(&pcm) {
        agc.process(&mut b, true);
    }
    assert_eq!(agc.gain_db(), 0.0);
    let mut agc = Agc::new();
    agc.set_enabled(false);
    for b in blocks(&pcm) {
        let mut o = b;
        agc.process(&mut o, false);
        assert_eq!(o, b);
    }
}
