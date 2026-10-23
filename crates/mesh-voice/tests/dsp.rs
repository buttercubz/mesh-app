//! HPF, wind guard, mixer/limiter, earcons, tones, prompt bank and audio check.

mod common;

use common::{blocks, read_wav, sine};
use mesh_voice::earcon::EARCON_LEVEL_DBFS;
use mesh_voice::prompt::{PROMPT_BANK_BYTES, PROMPT_MIN_INTERVAL_US};
use mesh_voice::{
    AudioCheck, AudioCheckMode, AudioCheckPhase, BLOCK_SAMPLES, Block, CaptureChain, Earcon,
    Earcons, EnergyVad, HighPass80, Limiter, Mixer, PromptBank, PromptBankError, PromptId,
    PromptPlayer, VadSensitivity, WindGuard, rms_dbfs,
};

fn db(x: f32) -> f32 {
    20.0 * x.log10()
}

#[test]
fn hpf_is_2nd_order_butterworth_at_80hz() {
    let h = HighPass80::new();
    let bq = h.biquad();
    let fs = 16_000.0;
    assert!(
        (db(bq.magnitude_at(80.0, fs)) + 3.01).abs() < 0.1,
        "−3 dB at the cutoff"
    );
    // 2nd order: −12 dB/octave well below the cutoff.
    let a20 = db(bq.magnitude_at(20.0, fs));
    let a40 = db(bq.magnitude_at(40.0, fs));
    assert!(a20 < -23.0 && a20 > -26.0, "{a20}");
    assert!((a40 - a20 - 12.0).abs() < 1.0);
    assert!(
        db(bq.magnitude_at(300.0, fs)).abs() < 0.3,
        "voice band untouched"
    );
    assert!(db(bq.magnitude_at(3000.0, fs)).abs() < 0.01);
}

#[test]
fn hpf_removes_rumble_in_time_domain() {
    let mut h = HighPass80::new();
    let mut low = blocks(&sine(30.0, -6.0, 16_000));
    let mut voice = blocks(&sine(500.0, -6.0, 16_000));
    for b in low.iter_mut().chain(voice.iter_mut()) {
        h.process(b);
    }
    let tail = |v: &[Block]| rms_dbfs(&v[50..].concat());
    assert!(
        tail(&low) < -6.0 - 3.0 - 15.0,
        "30 Hz attenuated: {}",
        tail(&low)
    );
    assert!(
        (tail(&voice) - (-6.0 - 3.01)).abs() < 0.3,
        "500 Hz kept: {}",
        tail(&voice)
    );
}

#[test]
fn wind_guard_fires_on_wind_not_on_speech() {
    let mut g = WindGuard::new();
    let wind = blocks(&read_wav("wind.wav"));
    let windy = wind.iter().filter(|b| g.process(b)).count();
    assert!(
        windy as f32 > 0.9 * wind.len() as f32,
        "wind flagged {windy}/{}",
        wind.len()
    );

    let mut g = WindGuard::new();
    let speech = read_wav("speech.wav");
    let labels = common::speech_labels();
    let mut flagged = 0;
    let mut total = 0;
    for (a, b) in labels {
        for blk in blocks(&speech[a..b]) {
            if rms_dbfs(&blk) < -45.0 {
                continue;
            }
            total += 1;
            flagged += usize::from(g.process(&blk));
        }
    }
    // Voiced speech with a strong first harmonic can trip the 6 dB rule now and then; the
    // VAD still opens there when p ≥ 0.9.
    assert!(
        (flagged as f32) < 0.15 * total as f32,
        "speech flagged {flagged}/{total}"
    );
}

#[test]
fn limiter_bounds_output_and_settles_at_threshold() {
    let mut l = Limiter::new();
    let mut peak_tail = 0f32;
    for i in 0..16_000 {
        let x = 2.0 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin();
        let y = l.process(x);
        assert!(y.abs() <= 1.0);
        if i > 8_000 {
            peak_tail = peak_tail.max(y.abs());
        }
    }
    let thr = 10f32.powf(-3.0 / 20.0);
    // No look-ahead and a 1 ms attack: the steady peak sits at the threshold within 0.6 dB.
    assert!(
        peak_tail >= thr * 0.99 && peak_tail <= thr * 1.072,
        "steady peak {peak_tail} vs {thr}"
    );
    // Below threshold: transparent.
    let mut l = Limiter::new();
    for i in 0..1_000 {
        let x = 0.5 * (i as f32 * 0.1).sin();
        assert!((l.process(x) - x).abs() < 1e-6);
    }
    // Release: after the loud part, gain recovers within a few release constants.
    let mut l = Limiter::new();
    for _ in 0..1_600 {
        l.process(1.0);
    }
    assert!(l.gain_db() < -2.5);
    for _ in 0..(16 * 250) {
        l.process(0.1);
    }
    assert!(l.gain_db() > -0.1);
}

#[test]
fn mixer_attenuates_3db_with_two_streams_and_adds_cues() {
    let mut m = Mixer::new();
    let a: Block = [3000; BLOCK_SAMPLES];
    let b: Block = [1000; BLOCK_SAMPLES];
    let cue: Block = [500; BLOCK_SAMPLES];
    let mut out = [0; BLOCK_SAMPLES];
    m.mix(&[&a], &[], &mut out);
    assert_eq!(out[10], 3000);
    m.mix(&[&a, &b], &[&cue], &mut out);
    let expect = (4000.0 * 10f32.powf(-3.0 / 20.0) + 500.0).round() as i16;
    assert!((out[10] - expect).abs() <= 1, "{} vs {expect}", out[10]);
    // A fifth stream is ignored (≤ 4).
    let loud: Block = [20_000; BLOCK_SAMPLES];
    let mut m2 = Mixer::new();
    m2.mix(&[&b, &b, &b, &b, &loud], &[], &mut out);
    let four = (4000.0 * 10f32.powf(-3.0 / 20.0)).round() as i16;
    assert!((out[10] - four).abs() <= 1);
    // Overload is limited, never wraps.
    let mut m3 = Mixer::new();
    for _ in 0..20 {
        m3.mix(&[&loud, &loud, &loud, &loud], &[&loud], &mut out);
        assert!(out.iter().all(|&s| s > 0));
    }
    assert!(rms_dbfs(&out) < -2.0);
}

fn render_all(e: &mut Earcons) -> Vec<i16> {
    let mut v = Vec::new();
    let mut b = [0; BLOCK_SAMPLES];
    while e.render(&mut b) {
        v.extend_from_slice(&b);
    }
    v
}

fn dominant_hz(x: &[i16]) -> f32 {
    // Zero-crossing frequency estimate on a pure tone.
    let zc = x.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    zc as f32 * 16_000.0 / (2.0 * x.len() as f32)
}

#[test]
fn earcons_match_cue_table() {
    // (cue, expected total duration ms)
    let durations = [
        (Earcon::Busy, 500),
        (Earcon::Preempted, 240),
        (Earcon::GoAhead, 40),
        (Earcon::RideStarted, 240),
        (Earcon::MemberJoined, 160),
        (Earcon::MemberLost, 160),
        (Earcon::MicLost, 650),
        (Earcon::HeadsetLost, 400),
        (Earcon::BluetoothOff, 300),
    ];
    for (cue, ms) in durations {
        let total: u32 = cue
            .segments()
            .iter()
            .map(|s| u32::from(s.on_ms + s.off_ms))
            .sum();
        assert_eq!(total, ms, "{cue:?}");
        let mut e = Earcons::new();
        assert!(e.trigger(0, cue));
        let pcm = render_all(&mut e);
        assert_eq!(
            pcm.len(),
            (ms as usize).div_ceil(10) * BLOCK_SAMPLES,
            "{cue:?}"
        );
        let peak = pcm.iter().map(|s| i32::from(*s).abs()).max().unwrap();
        let expect = 32767.0 * 10f32.powf(EARCON_LEVEL_DBFS / 20.0);
        assert!(
            (peak as f32 - expect).abs() < 0.02 * expect,
            "{cue:?} level"
        );
    }
    // Busy: 480 Hz; Bluetooth off: three descending steps 600, 450, 300 Hz.
    let mut e = Earcons::new();
    e.trigger(0, Earcon::Busy);
    let busy = render_all(&mut e);
    assert!((dominant_hz(&busy[160..2240]) - 480.0).abs() < 10.0);
    let mut e = Earcons::new();
    e.trigger(0, Earcon::BluetoothOff);
    let bt = render_all(&mut e);
    let f: Vec<f32> = (0..3)
        .map(|i| dominant_hz(&bt[i * 1600 + 200..i * 1600 + 1400]))
        .collect();
    assert!(
        (f[0] - 600.0).abs() < 15.0 && (f[1] - 450.0).abs() < 15.0 && (f[2] - 300.0).abs() < 15.0,
        "{f:?}"
    );
}

#[test]
fn earcon_rate_limits() {
    let mut e = Earcons::new();
    assert!(e.trigger(0, Earcon::MemberJoined));
    render_all(&mut e);
    assert!(
        !e.trigger(1_000_000, Earcon::MemberJoined),
        "aggregated per 5 s"
    );
    assert!(e.trigger(5_000_000, Earcon::MemberJoined));
    assert!(
        e.trigger(9_000_000, Earcon::MemberLost),
        "each cue has its own limiter"
    );
    assert!(e.trigger(0, Earcon::BluetoothOff));
    render_all(&mut e);
    assert!(!e.trigger(59_000_000, Earcon::BluetoothOff));
    assert!(
        e.trigger(60_000_000, Earcon::BluetoothOff),
        "repeated every 60 s while off"
    );
    assert!(e.trigger(0, Earcon::RideStarted));
    assert!(!e.trigger(3_600_000_000, Earcon::RideStarted), "once");
    e.reset_rate_limits();
    e.clear();
    assert!(e.trigger(0, Earcon::RideStarted));
    // Cues queue and play back to back.
    let mut e = Earcons::new();
    e.trigger(0, Earcon::GoAhead);
    e.trigger(0, Earcon::Preempted);
    let both = render_all(&mut e);
    assert_eq!(both.len(), (4 + 24) * BLOCK_SAMPLES);
    assert!(
        (dominant_hz(&both[200..600]) - 880.0).abs() < 25.0,
        "go-ahead first"
    );
    assert!(e.is_idle());
}

#[test]
fn prompt_bank_is_bounded_with_lru_eviction() {
    let mut bank = PromptBank::new();
    assert_eq!(bank.capacity_bytes(), PROMPT_BANK_BYTES);
    let clip = vec![7i16; 100_000]; // 200 KB
    bank.insert(PromptId(1), &clip, true, 0).unwrap();
    bank.insert(PromptId(2), &clip, false, 10).unwrap();
    assert_eq!(bank.used_bytes(), 400_000);
    bank.touch(PromptId(2), 20);
    // Third clip does not fit: the unpinned LRU (2) is evicted, pinned (1) stays.
    bank.insert(PromptId(3), &clip, false, 30).unwrap();
    assert!(
        bank.contains(PromptId(1)) && !bank.contains(PromptId(2)) && bank.contains(PromptId(3))
    );
    assert!(bank.used_bytes() <= PROMPT_BANK_BYTES);
    // Pinned clips are never evicted: a too-large request fails cleanly.
    bank.insert(PromptId(4), &clip, true, 40).unwrap();
    assert_eq!(
        bank.insert(PromptId(5), &clip, false, 50),
        Err(PromptBankError::Full)
    );
    assert_eq!(
        bank.insert(PromptId(6), &vec![0; 300_000], false, 50),
        Err(PromptBankError::TooLarge)
    );
    assert_eq!(
        bank.insert(PromptId(7), &[], false, 50),
        Err(PromptBankError::Empty)
    );
    // Compaction keeps clip contents intact.
    let mut b = PromptBank::with_capacity_samples(1000);
    b.insert(PromptId(1), &[1; 300], false, 0).unwrap();
    b.insert(PromptId(2), &[2; 300], false, 1).unwrap();
    b.insert(PromptId(3), &[3; 300], false, 2).unwrap();
    assert!(b.remove(PromptId(2)));
    assert!(b.get(PromptId(3)).unwrap().iter().all(|&s| s == 3));
    assert!(b.get(PromptId(1)).unwrap().iter().all(|&s| s == 1));
}

#[test]
fn prompt_player_rate_limits_and_merges() {
    let mut bank = PromptBank::new();
    bank.insert(PromptId(1), &[100; 400], true, 0).unwrap();
    bank.insert(PromptId(2), &[200; 160], true, 0).unwrap();
    let mut p = PromptPlayer::new();
    assert!(p.enqueue(PromptId(1)));
    assert!(p.enqueue(PromptId(1)), "identical prompts merge");
    assert!(p.enqueue(PromptId(2)));
    let mut out = [0; BLOCK_SAMPLES];
    let mut t = 0;
    let mut played = Vec::new();
    while t < 10_000_000 {
        if p.render(t, &bank, &mut out) {
            played.push((t, out[0]));
        }
        t += 10_000;
    }
    // Prompt 1: 3 blocks (400 samples), then prompt 2 only 4 s after prompt 1 started.
    assert_eq!(played.len(), 4);
    assert_eq!(played[0], (0, 100));
    assert_eq!(played[2].1, 100);
    assert_eq!(played[3], (PROMPT_MIN_INTERVAL_US, 200));
}

fn run_check(
    mode: AudioCheckMode,
    input: &[i16],
) -> (Vec<i16>, Vec<mesh_voice::AudioCheckSample>, AudioCheck) {
    let mut chain = CaptureChain::new(Box::new(EnergyVad::new()), VadSensitivity::Medium);
    let mut check = AudioCheck::new(mode);
    let mut played = Vec::new();
    let mut samples = Vec::new();
    for mut b in blocks(input) {
        let frame = chain.process(&mut b);
        let mut out = [0; BLOCK_SAMPLES];
        if let Some(s) = check.process(&b, &frame, &mut out) {
            samples.push(s);
        }
        played.extend_from_slice(&out);
    }
    (played, samples, check)
}

#[test]
fn audio_check_tone_then_record() {
    let (played, samples, check) = run_check(AudioCheckMode::Tone, &vec![0; 16_000 * 4]);
    assert!(rms_dbfs(&played[..16_000]) > -16.0, "tone audible");
    assert!(
        played[16_000..].iter().all(|&s| s == 0),
        "record phase is silent"
    );
    assert!(check.is_finished());
    // 20 Hz samples for 3 s.
    assert_eq!(samples.len(), 60);
    assert!(
        samples[..20]
            .iter()
            .all(|s| s.phase == AudioCheckPhase::Tone)
    );
    assert!(
        samples[20..]
            .iter()
            .all(|s| s.phase == AudioCheckPhase::Record)
    );
}

#[test]
fn audio_check_echo_is_one_second_late() {
    let mut input = vec![0i16; 16_000 * 3];
    let click = 16_000 / 2;
    input[click..click + 160].copy_from_slice(&sine(1000.0, -10.0, 160));
    let (played, samples, check) = run_check(AudioCheckMode::Echo, &input);
    let first = played.iter().position(|&s| s != 0).unwrap();
    // HPF and ±1 sample of ringing aside, the echo appears exactly 16 000 samples later.
    assert!(
        (first as i64 - (click + 16_000) as i64).abs() <= 1,
        "{first}"
    );
    assert!(!check.is_finished());
    assert!(samples.iter().all(|s| s.phase == AudioCheckPhase::Echo));
}

#[test]
fn audio_check_vox_meter_reports_p_and_gate() {
    let mut input = common::Noise::new(3).pcm(16_000, -60.0);
    input.extend(sine(300.0, -12.0, 16_000));
    let (played, samples, _) = run_check(AudioCheckMode::VoxMeter, &input);
    assert!(played.iter().all(|&s| s == 0));
    assert_eq!(samples.len(), 40);
    assert!(samples[..15].iter().all(|s| !s.gate_open && s.vad_p < 0.5));
    assert!(
        samples[25..]
            .iter()
            .all(|s| s.gate_open && s.vad_p > 0.9 && s.level > 0.1)
    );
}
