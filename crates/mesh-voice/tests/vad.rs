//! VAD gate logic and precision/recall floors on the fixture clips (§7.3, §13.1).

mod common;

use common::{Noise, blocks, mix, read_wav, sine, speech_labels};
use mesh_voice::vad::{Upsampler3, WIND_ONSET_P};
use mesh_voice::{
    BLOCK_SAMPLES, CaptureChain, EnergyVad, RnnVad, Vad, VadEvent, VadGate, VadSensitivity,
    rms_dbfs,
};

#[test]
fn sensitivity_table() {
    use VadSensitivity::*;
    assert_eq!(
        (High.theta_on(), High.theta_off(), High.margin_db()),
        (0.50, 0.30, 4.0)
    );
    assert_eq!(
        (Medium.theta_on(), Medium.theta_off(), Medium.margin_db()),
        (0.65, 0.40, 6.0)
    );
    assert_eq!(
        (Low.theta_on(), Low.theta_off(), Low.margin_db()),
        (0.80, 0.50, 9.0)
    );
    for s in [High, Medium, Low] {
        assert_eq!(s.hangover_us(), 300_000, "single 300 ms hangover");
    }
    assert_eq!(VadSensitivity::default(), Medium);
    assert_eq!(High.raised(), Medium);
    assert_eq!(Medium.raised(), Low);
    assert_eq!(Low.raised(), Low);
}

/// Feeds `n` blocks of (p, level) and returns the events with their block index.
fn feed(g: &mut VadGate, seq: &[(f32, f32, bool)]) -> Vec<(usize, VadEvent)> {
    seq.iter()
        .enumerate()
        .filter_map(|(i, &(p, db, w))| g.update(p, db, w).map(|e| (i, e)))
        .collect()
}

#[test]
fn gate_onset_needs_two_blocks_level_margin_and_wind_rule() {
    let quiet = (0.0, -60.0, false);
    // Floor settles at −60 dBFS.
    let mut g = VadGate::new(VadSensitivity::Medium);
    let mut seq = vec![quiet; 20];
    seq.push((0.9, -30.0, false));
    seq.push(quiet);
    seq.push((0.9, -30.0, false));
    seq.push((0.9, -30.0, false));
    assert_eq!(
        feed(&mut g, &seq),
        vec![(23, VadEvent::On)],
        "one block is not enough"
    );

    // High p but the level is within the margin of the floor: no onset.
    let mut g = VadGate::new(VadSensitivity::Medium);
    let mut seq = vec![quiet; 20];
    seq.extend([(0.99, -56.0, false); 10]);
    assert!(feed(&mut g, &seq).is_empty());
    // High sensitivity has a 4 dB margin: −55 dBFS opens it.
    let mut g = VadGate::new(VadSensitivity::High);
    let mut seq = vec![quiet; 20];
    seq.extend([(0.99, -55.0, false); 2]);
    assert_eq!(feed(&mut g, &seq), vec![(21, VadEvent::On)]);

    // Wind: p = 0.85 is not enough, p ≥ 0.9 is.
    let mut g = VadGate::new(VadSensitivity::Medium);
    let mut seq = vec![quiet; 20];
    seq.extend([(0.85, -20.0, true); 10]);
    seq.extend([(WIND_ONSET_P, -20.0, true); 2]);
    assert_eq!(feed(&mut g, &seq), vec![(31, VadEvent::On)]);
}

#[test]
fn gate_release_is_exactly_the_hangover() {
    let mut g = VadGate::new(VadSensitivity::Medium);
    let mut seq = vec![(0.0, -60.0, false); 10];
    seq.extend([(0.9, -20.0, false); 20]); // On at 11
    seq.extend([(0.1, -60.0, false); 15]); // 150 ms below θ_off …
    seq.push((0.5, -30.0, false)); // … a block above θ_off resets the hangover
    seq.extend([(0.1, -60.0, false); 40]);
    let ev = feed(&mut g, &seq);
    assert_eq!(ev[0], (11, VadEvent::On));
    // Last block ≥ θ_off is 45; release on the 30th block below θ_off (45 + 30).
    assert_eq!(ev[1], (75, VadEvent::Off));
    assert_eq!(ev.len(), 2);
}

#[test]
fn noise_floor_rises_3db_per_second_and_falls_instantly() {
    let mut g = VadGate::new(VadSensitivity::Medium);
    g.update(0.0, -60.0, false);
    for _ in 0..100 {
        g.update(0.0, -20.0, false);
    }
    assert!(
        (g.noise_floor_db() - (-57.0)).abs() < 0.05,
        "{}",
        g.noise_floor_db()
    );
    g.update(0.0, -70.0, false);
    assert_eq!(g.noise_floor_db(), -70.0);
}

#[test]
fn upsampler_preserves_tone() {
    let mut up = Upsampler3::new();
    let x = blocks(&sine(1000.0, -6.0, 3200));
    let mut y = Vec::new();
    let mut buf = [0f32; 480];
    for b in &x {
        up.process(b, &mut buf);
        y.extend_from_slice(&buf);
    }
    let tail = &y[960..];
    let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
    let expect = 32767.0 * 10f32.powf(-6.0 / 20.0) / 2f32.sqrt();
    assert!((rms / expect - 1.0).abs() < 0.02, "gain {}", rms / expect);
    let zc = tail
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let f = zc as f32 * 48_000.0 / (2.0 * tail.len() as f32);
    assert!((f - 1000.0).abs() < 10.0, "{f}");
}

struct Score {
    tp: usize,
    fp: usize,
    fn_: usize,
    tn: usize,
}

impl Score {
    fn precision(&self) -> f64 {
        self.tp as f64 / (self.tp + self.fp).max(1) as f64
    }
    fn recall(&self) -> f64 {
        self.tp as f64 / (self.tp + self.fn_).max(1) as f64
    }
    fn false_open(&self) -> f64 {
        self.fp as f64 / (self.fp + self.tn).max(1) as f64
    }
}

/// Runs a capture chain over `pcm` and scores the gate against `labels` (sample spans).
/// Blocks inside a phrase are speech; the 400 ms after a phrase (hangover) and the first
/// 30 ms (onset latency) are not scored.
fn score(vad: Box<dyn Vad>, pcm: &[i16], labels: &[(usize, usize)]) -> Score {
    let mut chain = CaptureChain::new(vad, VadSensitivity::Medium);
    let mut s = Score {
        tp: 0,
        fp: 0,
        fn_: 0,
        tn: 0,
    };
    for (i, mut b) in blocks(pcm).into_iter().enumerate() {
        let open = chain.process(&mut b).gate_open;
        let (a, z) = (i * BLOCK_SAMPLES, (i + 1) * BLOCK_SAMPLES);
        let speech = labels.iter().any(|&(s0, s1)| a >= s0 + 480 && z <= s1);
        let skip = labels
            .iter()
            .any(|&(s0, s1)| (z > s0 && a < s0 + 480) || (a >= s1 && a < s1 + 6400));
        if skip {
            continue;
        }
        match (speech, open) {
            (true, true) => s.tp += 1,
            (true, false) => s.fn_ += 1,
            (false, true) => s.fp += 1,
            (false, false) => s.tn += 1,
        }
    }
    s
}

fn background(n: usize) -> Vec<i16> {
    Noise::new(7).pcm(n, -62.0)
}

#[test]
fn rnn_vad_precision_recall_on_quiet_speech() {
    let speech = read_wav("speech.wav");
    let pcm = mix(&speech, &background(speech.len()), 0.0);
    let s = score(Box::new(RnnVad::new()), &pcm, &speech_labels());
    eprintln!("quiet: P={:.3} R={:.3}", s.precision(), s.recall());
    assert!(s.precision() >= 0.97, "precision {:.3}", s.precision());
    assert!(s.recall() >= 0.90, "recall {:.3}", s.recall());
}

#[test]
fn rnn_vad_precision_recall_with_engine_noise() {
    let speech = mix(&read_wav("speech.wav"), &background(1 << 20), 0.0);
    // Engine fixture peaks at −6 dBFS; −20 dB puts it ≈ 12 dB under the speech level.
    let pcm = mix(&speech, &read_wav("engine.wav"), -20.0);
    let s = score(Box::new(RnnVad::new()), &pcm, &speech_labels());
    eprintln!("engine: P={:.3} R={:.3}", s.precision(), s.recall());
    assert!(s.precision() >= 0.95, "precision {:.3}", s.precision());
    assert!(s.recall() >= 0.85, "recall {:.3}", s.recall());
}

#[test]
fn rnn_vad_precision_recall_with_wind() {
    let speech = mix(&read_wav("speech.wav"), &background(1 << 20), 0.0);
    let pcm = mix(&speech, &read_wav("wind.wav"), -12.0);
    let s = score(Box::new(RnnVad::new()), &pcm, &speech_labels());
    eprintln!("wind: P={:.3} R={:.3}", s.precision(), s.recall());
    assert!(s.precision() >= 0.95, "precision {:.3}", s.precision());
    assert!(s.recall() >= 0.80, "recall {:.3}", s.recall());
}

#[test]
fn rnn_vad_stays_closed_on_wind_and_engine_alone() {
    for (name, gain) in [
        ("wind.wav", 0.0),
        ("engine.wav", 0.0),
        ("engine.wav", -15.0),
    ] {
        let noise = read_wav(name);
        let pcm = mix(&noise, &background(noise.len()), 0.0);
        let s = score(
            Box::new(RnnVad::new()),
            &mix(&background(pcm.len()), &pcm, gain),
            &[],
        );
        eprintln!("{name} {gain} dB: false open {:.3}", s.false_open());
        assert!(
            s.false_open() <= 0.05,
            "{name}: false open {:.3}",
            s.false_open()
        );
    }
}

#[test]
fn energy_vad_detects_tone_bursts() {
    let mut pcm = background(16_000);
    pcm.extend(sine(400.0, -20.0, 16_000));
    pcm.extend(background(16_000));
    let labels = [(16_000, 32_000)];
    let s = score(Box::new(EnergyVad::new()), &pcm, &labels);
    assert!(s.recall() > 0.95 && s.precision() > 0.95);
    assert!(rms_dbfs(&pcm[20_000..20_160]) > -25.0);
}
