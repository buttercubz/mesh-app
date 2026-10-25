//! Packetizer (burst-relative ts, FIRST/PREROLL, flush) and the SimCodec end to end.

mod common;

use common::{blocks, log_spectral_distance, read_wav, sine, snr_db};
use mesh_voice::codec::sim::{SimDecoder, SimEncoder};
use mesh_voice::{
    BLOCK_SAMPLES, BLOCK_US, Codec, EncoderConfig, FrameDuration, JitterBuffer, Packetizer,
    PreRoll, Profile, PullStatus, SimCodec, VoiceDecoder, VoiceEncoder, VoiceFrame,
};

fn collect(p: &mut Packetizer, pcm: &[i16]) -> Vec<VoiceFrame> {
    let mut v = Vec::new();
    for b in blocks(pcm) {
        p.push_block(&b, &mut |f| v.push(f.clone())).unwrap();
    }
    v
}

fn preroll_of(n: usize) -> PreRoll {
    let mut pr = PreRoll::new();
    for i in 0..n {
        pr.push(&[i as i16 + 1; BLOCK_SAMPLES]);
    }
    pr
}

#[test]
fn preroll_ring_keeps_last_40ms() {
    let pr = preroll_of(6);
    assert_eq!(pr.len(), 4);
    let firsts: Vec<i16> = (0..4).map(|i| pr.get(i).unwrap()[0]).collect();
    assert_eq!(firsts, vec![3, 4, 5, 6], "oldest first");
}

#[test]
fn ts_is_burst_relative_frame_n_of_40ms_is_4n() {
    let codec = SimCodec::new();
    let mut p = Packetizer::new(codec.new_encoder().unwrap());
    let cfg = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
    let mut frames = Vec::new();
    p.start_burst(9, &cfg, &preroll_of(4), &mut |f| frames.push(f.clone()))
        .unwrap();
    assert_eq!(frames.len(), 1, "40 ms of pre-roll = one 40 ms frame");
    frames.extend(collect(&mut p, &sine(400.0, -20.0, 16_000)));
    assert_eq!(frames.len(), 26);
    for (n, f) in frames.iter().enumerate() {
        assert_eq!(f.burst, 9);
        assert_eq!(f.ts, 4 * n as u16);
        assert_eq!(f.cfg.duration(), FrameDuration::Ms40);
        assert_eq!(f.cfg.first(), n == 0);
        assert_eq!(f.cfg.preroll(), n == 0);
        assert!(!f.cfg.lbrr() && !f.cfg.dred());
        assert!(!f.opus().is_empty() && f.opus().len() <= 400);
    }
    assert_eq!(p.elapsed_ticks(), 104);
    // Partial frame flushed with silence at the end of the burst.
    let mut tail = Vec::new();
    p.push_block(&[5; BLOCK_SAMPLES], &mut |f| tail.push(f.clone()))
        .unwrap();
    assert!(tail.is_empty());
    p.finish_burst(&mut |f| tail.push(f.clone())).unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].ts, 104);
    assert!(!p.is_active());
    // Blocks after the end are ignored.
    p.push_block(&[5; BLOCK_SAMPLES], &mut |_| {
        panic!("no frames after the burst")
    })
    .unwrap();
}

#[test]
fn wifi_profile_preroll_spans_two_20ms_frames_and_abort_drops() {
    let codec = SimCodec::new();
    let mut p = Packetizer::new(codec.new_encoder().unwrap());
    let cfg = EncoderConfig::for_profile(Profile::Wifi24Fec, 16_000, 4.0);
    let mut frames = Vec::new();
    p.start_burst(3, &cfg, &preroll_of(4), &mut |f| frames.push(f.clone()))
        .unwrap();
    frames.extend(collect(&mut p, &sine(400.0, -20.0, 1_600)));
    let flags: Vec<(u16, bool, bool, bool)> = frames
        .iter()
        .map(|f| (f.ts, f.cfg.first(), f.cfg.preroll(), f.cfg.lbrr()))
        .collect();
    assert_eq!(flags[0], (0, true, true, true));
    assert_eq!(flags[1], (2, false, true, true));
    assert_eq!(flags[2], (4, false, false, true));
    p.push_block(&[1; BLOCK_SAMPLES], &mut |_| {}).unwrap();
    p.abort_burst();
    p.finish_burst(&mut |_| panic!("abort drops the partial frame"))
        .unwrap();
}

#[test]
fn sim_codec_sizes_track_bitrate() {
    let codec = SimCodec::new();
    for (profile, lo, hi) in [
        (Profile::Ble16, 70, 90),
        (Profile::Wifi20, 40, 70),
        (Profile::Wifi24Fec, 60, 110),
    ] {
        let mut e = codec.new_encoder().unwrap();
        e.configure(&EncoderConfig::for_profile(profile, 16_000, 5.0))
            .unwrap();
        let n = EncoderConfig::for_profile(profile, 16_000, 5.0)
            .frame
            .samples();
        let pcm = sine(300.0, -20.0, n);
        let mut out = [0u8; 400];
        e.encode(&pcm, &mut out).unwrap();
        let len = e.encode(&pcm, &mut out).unwrap();
        assert!((lo..=hi).contains(&len), "{profile:?}: {len} B");
    }
}

#[test]
fn sim_codec_round_trip_quality_and_fec() {
    let speech = read_wav("speech.wav");
    let mut enc = SimEncoder::new();
    let cfg = EncoderConfig::for_profile(Profile::Wifi24Fec, 16_000, 5.0);
    enc.configure(&cfg).unwrap();
    let mut dec = SimDecoder::new();
    let n = cfg.frame.samples();
    let mut out = vec![0i16; speech.len()];
    let mut pkts = Vec::new();
    for (i, chunk) in speech.chunks_exact(n).enumerate() {
        let mut buf = [0u8; 400];
        let len = enc.encode(chunk, &mut buf).unwrap();
        pkts.push(buf[..len].to_vec());
        dec.decode(&pkts[i], &mut out[i * n..(i + 1) * n]).unwrap();
    }
    let delay = enc.lookahead();
    let lsd = log_spectral_distance(&speech, &out[delay..], 3400.0);
    assert!(lsd < 12.0, "SimCodec LSD {lsd:.1} dB");
    // FEC: frame k recovered from packet k+1 is close to the original frame.
    let k = 300;
    let mut d2 = SimDecoder::new();
    let mut rec = vec![0i16; n];
    d2.decode(&pkts[k - 1], &mut rec).unwrap();
    d2.decode_fec(&pkts[k + 1], &mut rec).unwrap();
    let mut plc = vec![0i16; n];
    let mut d3 = SimDecoder::new();
    d3.decode(&pkts[k - 1], &mut plc).unwrap();
    d3.conceal(&mut plc).unwrap();
    let reference = &out[k * n..(k + 1) * n];
    assert!(
        snr_db(reference, &rec) > snr_db(reference, &plc) + 3.0,
        "FEC beats concealment"
    );
}

#[test]
fn sim_pipeline_through_jitter_buffer() {
    let codec = SimCodec::new();
    let mut p = Packetizer::new(codec.new_encoder().unwrap());
    let cfg = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
    let tone = sine(440.0, -15.0, 32_000);
    let mut frames = Vec::new();
    p.start_burst(1, &cfg, &PreRoll::new(), &mut |f| frames.push(f.clone()))
        .unwrap();
    frames.extend(collect(&mut p, &tone));
    p.finish_burst(&mut |f| frames.push(f.clone())).unwrap();
    let mut jb = JitterBuffer::new();
    let mut dec = codec.new_decoder().unwrap();
    let mut played = Vec::new();
    let mut b = [0i16; BLOCK_SAMPLES];
    for t in 0..260u64 {
        let now = t * BLOCK_US;
        for f in frames.iter().filter(|f| u64::from(f.ts) + 4 == t) {
            jb.push(now, f);
        }
        if t == 210 {
            jb.end_burst(1);
        }
        if jb.pull(now, dec.as_mut(), &mut b) == PullStatus::Normal {
            played.extend_from_slice(&b);
        }
    }
    assert_eq!(played.len(), 32_000);
    assert!(jb.is_idle());
    let d = cfg_delay();
    let (a, b) = (&tone[1_000..31_000], &played[1_000 + d..31_000 + d]);
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    assert!(dot / (na * nb) > 0.95, "correlation {}", dot / (na * nb));
    assert!((mesh_voice::rms_dbfs(a) - mesh_voice::rms_dbfs(b)).abs() < 1.5);
}

fn cfg_delay() -> usize {
    SimEncoder::new().lookahead()
}

#[test]
fn mid_burst_downshift_keeps_the_ts_grid_and_refuses_a_frame_change() {
    let codec = SimCodec::new();
    let mut p = Packetizer::new(codec.new_encoder().unwrap());
    let ble16 = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
    let ble12 = EncoderConfig::for_profile(Profile::Ble12, 16_000, 0.0);
    // No burst: refused.
    assert!(p.reconfigure(&ble12).is_err());
    p.start_burst(3, &ble16, &PreRoll::new(), &mut |_| {})
        .unwrap();
    let a = collect(&mut p, &sine(440.0, -20.0, 3_200));
    p.reconfigure(&ble12).unwrap();
    assert_eq!(p.config().bitrate_bps, 12_000);
    let b = collect(&mut p, &sine(440.0, -20.0, 3_200));
    let ts: Vec<u16> = a.iter().chain(&b).map(|f| f.ts).collect();
    let expect: Vec<u16> = (0..ts.len() as u16).map(|n| 4 * n).collect();
    assert_eq!(ts, expect, "same burst, same 40 ms grid");
    assert!(b.iter().all(|f| f.burst == 3));
    // A frame-duration change would break the grid.
    let wifi = EncoderConfig::for_profile(Profile::Wifi20, 16_000, 0.0);
    assert!(p.reconfigure(&wifi).is_err());
}
