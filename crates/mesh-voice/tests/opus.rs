//! libopus backend (feature `opus`): §7.4 ctls applied, WB/NB round trip quality, LBRR
//! recovery and deep PLC through the jitter buffer.
#![cfg(feature = "opus")]

mod common;

use common::{band_energy_db, log_spectral_distance, read_wav, snr_db};
use mesh_voice::codec::opus::{DECODER_COMPLEXITY, OpusVoiceDecoder, OpusVoiceEncoder};
use mesh_voice::{
    BLOCK_SAMPLES, BLOCK_US, Codec, EncoderConfig, JitterBuffer, OpusCodec, Packetizer, PreRoll,
    Profile, PullStatus, VoiceDecoder, VoiceEncoder, VoiceFrame,
};
use opus_sys::{Bandwidth, Signal, Vbr};

/// Wideband quality floor: mean log-spectral distance over 0-7 kHz (dB; measured 6.6-7.6
/// for WIFI20/BLE16/BLE12 on the fixture, a silent or NB-only output scores far above).
const WB_LSD_MAX: f64 = 9.0;
/// Narrowband quality floor over 0-3.4 kHz (dB).
const NB_LSD_MAX: f64 = 6.0;

fn encode_all(cfg: &EncoderConfig, pcm: &[i16]) -> Vec<Vec<u8>> {
    let mut e = OpusVoiceEncoder::new().unwrap();
    e.reset().unwrap();
    e.configure(cfg).unwrap();
    let n = cfg.frame.samples();
    pcm.chunks_exact(n)
        .map(|c| {
            let mut buf = [0u8; 400];
            let len = e.encode(c, &mut buf).unwrap();
            buf[..len].to_vec()
        })
        .collect()
}

fn decode_all(pkts: &[Vec<u8>], n: usize, lost: impl Fn(usize) -> bool, fec: bool) -> Vec<i16> {
    let mut d = OpusVoiceDecoder::new().unwrap();
    let mut out = vec![0i16; pkts.len() * n];
    for (i, p) in pkts.iter().enumerate() {
        let o = &mut out[i * n..(i + 1) * n];
        if !lost(i) {
            d.decode(p, o).unwrap();
        } else if fec && i + 1 < pkts.len() && !lost(i + 1) {
            d.decode_fec(&pkts[i + 1], o).unwrap();
        } else {
            d.conceal(o).unwrap();
        }
    }
    out
}

fn lookahead() -> usize {
    OpusVoiceEncoder::new().unwrap().lookahead()
}

#[test]
fn version_is_pinned() {
    assert_eq!(OpusCodec::version(), "libopus 1.6.1");
    assert_eq!(OpusCodec::new().name(), "opus");
}

#[test]
fn every_profile_applies_the_7_4_ctls() {
    for p in Profile::ALL {
        for (rate, bw) in [
            (16_000, Bandwidth::Wideband),
            (8_000, Bandwidth::Narrowband),
        ] {
            for loss in [0.0, 12.0] {
                let cfg = EncoderConfig::for_profile(p, rate, loss);
                let mut e = OpusVoiceEncoder::new().unwrap();
                e.configure(&cfg).unwrap();
                let o = e.inner();
                assert_eq!(o.bitrate().unwrap(), cfg.bitrate_bps as i32, "{p:?}");
                assert_eq!(o.complexity().unwrap(), cfg.complexity);
                assert_eq!(o.vbr().unwrap(), Vbr::Constrained);
                assert!(!o.dtx().unwrap(), "DTX off");
                assert_eq!(o.signal().unwrap(), Signal::Voice);
                assert_eq!(o.max_bandwidth().unwrap(), bw);
                assert_eq!(o.inband_fec().unwrap(), p == Profile::Wifi24Fec);
                assert_eq!(o.packet_loss_perc().unwrap(), cfg.packet_loss_perc);
                if !o.inband_fec().unwrap() && p != Profile::BleDred {
                    assert_eq!(
                        o.packet_loss_perc().unwrap(),
                        0,
                        "loss hint 0 with LBRR off"
                    );
                }
                assert_eq!(o.dred_duration().unwrap(), cfg.dred_duration_10ms);
            }
        }
    }
    let d = OpusVoiceDecoder::new().unwrap();
    assert_eq!(
        d.inner().complexity().unwrap(),
        DECODER_COMPLEXITY,
        "deep PLC + NoLACE"
    );
}

#[test]
fn wideband_round_trip_quality() {
    let speech = read_wav("speech.wav");
    let la = lookahead();
    for p in [Profile::Ble16, Profile::Ble12, Profile::Wifi20] {
        let cfg = EncoderConfig::for_profile(p, 16_000, 0.0);
        let pkts = encode_all(&cfg, &speech);
        let wb = pkts
            .iter()
            .skip(5)
            .filter(|k| opus_sys::packet_bandwidth(k).unwrap() == Bandwidth::Wideband)
            .count();
        assert!(
            wb as f64 >= 0.95 * (pkts.len() - 5) as f64,
            "{p:?}: WB packets {wb}/{}",
            pkts.len()
        );
        let out = decode_all(&pkts, cfg.frame.samples(), |_| false, false);
        let lsd = log_spectral_distance(&speech, &out[la..], 7000.0);
        eprintln!("{p:?} WB LSD {lsd:.2} dB");
        assert!(lsd < WB_LSD_MAX, "{p:?}: LSD {lsd:.2}");
        // Content above 4 kHz survives (wideband, not narrowband).
        let hi_ref = band_energy_db(&speech, 4000.0, 7000.0);
        let hi_out = band_energy_db(&out, 4000.0, 7000.0);
        assert!(
            (hi_out - hi_ref).abs() < 6.0,
            "{p:?}: 4-7 kHz {hi_out:.1} vs {hi_ref:.1}"
        );
        let mean = pkts.iter().map(Vec::len).sum::<usize>() as f64 / pkts.len() as f64;
        let nominal = cfg.nominal_frame_bytes() as f64;
        assert!(
            mean < 1.25 * nominal,
            "{p:?}: mean {mean:.1} B vs {nominal}"
        );
    }
}

#[test]
fn narrowband_route_round_trip_quality() {
    let speech = read_wav("speech.wav");
    let cfg = EncoderConfig::for_profile(Profile::Ble16, 8_000, 0.0);
    let pkts = encode_all(&cfg, &speech);
    assert!(
        pkts.iter()
            .all(|k| opus_sys::packet_bandwidth(k).unwrap() == Bandwidth::Narrowband)
    );
    let out = decode_all(&pkts, cfg.frame.samples(), |_| false, false);
    let lsd = log_spectral_distance(&speech, &out[lookahead()..], 3400.0);
    eprintln!("NB LSD {lsd:.2} dB");
    assert!(lsd < NB_LSD_MAX, "NB LSD {lsd:.2}");
    assert!(
        band_energy_db(&out, 4500.0, 8000.0) < -35.0,
        "nothing above 4 kHz"
    );
    // The WB threshold is discriminating: this NB decode fails it over 0-7 kHz.
    let lsd7 = log_spectral_distance(&speech, &out[lookahead()..], 7000.0);
    eprintln!("NB decode scored as WB: LSD {lsd7:.2} dB");
    assert!(lsd7 > WB_LSD_MAX, "{lsd7:.2}");
}

#[test]
fn lbrr_recovery_beats_plc() {
    let speech = read_wav("speech.wav");
    let cfg = EncoderConfig::for_profile(Profile::Wifi24Fec, 16_000, 10.0);
    let pkts = encode_all(&cfg, &speech);
    let with_lbrr = pkts
        .iter()
        .skip(1)
        .filter(|k| opus_sys::packet_has_lbrr(k).unwrap())
        .count();
    assert!(
        with_lbrr as f64 > 0.5 * pkts.len() as f64,
        "LBRR present in {with_lbrr}/{}",
        pkts.len()
    );
    let n = cfg.frame.samples();
    let lost = |i: usize| i > 10 && i.is_multiple_of(7);
    let clean = decode_all(&pkts, n, |_| false, false);
    let fec = decode_all(&pkts, n, lost, true);
    let plc = decode_all(&pkts, n, lost, false);
    // Compare only the lost frames (and the one after), against the loss-free decode.
    let (mut s_fec, mut s_plc, mut k) = (0.0, 0.0, 0);
    for i in (0..pkts.len()).filter(|&i| lost(i)) {
        let r = i * n..(i + 2).min(pkts.len()) * n;
        if common::rms_db(&clean[r.clone()]) < -45.0 {
            continue;
        }
        s_fec += snr_db(&clean[r.clone()], &fec[r.clone()]);
        s_plc += snr_db(&clean[r.clone()], &plc[r]);
        k += 1;
    }
    let (s_fec, s_plc) = (s_fec / k as f64, s_plc / k as f64);
    eprintln!("lost frames: FEC {s_fec:.1} dB, PLC {s_plc:.1} dB over {k}");
    assert!(k > 20);
    assert!(
        s_fec > s_plc + 2.0,
        "LBRR recovery {s_fec:.1} dB vs PLC {s_plc:.1} dB"
    );
}

#[test]
fn deep_plc_continues_speech_then_jb_fades() {
    let speech = read_wav("speech.wav");
    let codec = OpusCodec::new();
    let mut p = Packetizer::new(codec.new_encoder().unwrap());
    let cfg = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
    // A voiced stretch of the first phrase.
    let start = 1_200 * 16;
    let mut frames: Vec<VoiceFrame> = Vec::new();
    p.start_burst(4, &cfg, &PreRoll::new(), &mut |f| frames.push(f.clone()))
        .unwrap();
    for b in common::blocks(&speech[start..start + 16_000]) {
        p.push_block(&b, &mut |f| frames.push(f.clone())).unwrap();
    }
    let mut jb = JitterBuffer::new();
    let mut dec = codec.new_decoder().unwrap();
    let mut statuses = Vec::new();
    let mut levels = Vec::new();
    let mut b = [0i16; BLOCK_SAMPLES];
    // Frames 0..=9 arrive on time, then the link drops for good.
    for t in 0..120u64 {
        let now = t * BLOCK_US;
        for f in frames
            .iter()
            .filter(|f| f.ts < 40 && u64::from(f.ts) + 4 == t)
        {
            jb.push(now, f);
        }
        let st = jb.pull(now, dec.as_mut(), &mut b);
        statuses.push(st);
        levels.push(mesh_voice::rms_dbfs(&b));
    }
    let first_plc = statuses.iter().position(|s| *s == PullStatus::Plc).unwrap();
    assert!(
        levels[first_plc] > -45.0,
        "deep PLC keeps the voice going: {:.1} dBFS",
        levels[first_plc]
    );
    assert!(
        statuses[first_plc..first_plc + 10]
            .iter()
            .all(|s| *s == PullStatus::Plc)
    );
    let faded = statuses
        .iter()
        .position(|s| *s == PullStatus::Faded)
        .unwrap();
    assert_eq!(faded, first_plc + 12, "100 ms of PLC + 20 ms fade");
    assert!(levels[faded..faded + 10].iter().all(|l| *l <= -119.0));
}

#[test]
fn dtx_off_keeps_the_frame_cadence_in_silence() {
    let silence = vec![0i16; 16_000];
    let cfg = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
    let pkts = encode_all(&cfg, &silence);
    assert_eq!(pkts.len(), 25);
    assert!(
        pkts.iter().all(|k| k.len() >= 3),
        "no 1-2 byte DTX frames: {:?}",
        pkts.iter().map(Vec::len).collect::<Vec<_>>()
    );
}

#[test]
fn encoder_is_reset_at_every_burst_start() {
    let speech = read_wav("speech.wav");
    let codec = OpusCodec::new();
    let mut p = Packetizer::new(codec.new_encoder().unwrap());
    let cfg = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
    let mut runs = Vec::new();
    for burst in 0..2u8 {
        let mut v: Vec<Vec<u8>> = Vec::new();
        p.start_burst(burst, &cfg, &PreRoll::new(), &mut |f| {
            v.push(f.opus().to_vec())
        })
        .unwrap();
        for b in common::blocks(&speech[16_000..16_000 + 6_400]) {
            p.push_block(&b, &mut |f| v.push(f.opus().to_vec()))
                .unwrap();
        }
        p.finish_burst(&mut |f| v.push(f.opus().to_vec())).unwrap();
        runs.push(v);
    }
    assert_eq!(
        runs[0], runs[1],
        "identical input after a reset encodes identically"
    );
}
