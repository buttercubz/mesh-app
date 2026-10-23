//! Encode/decode behaviour of the mesh-app profiles (DESIGN §7.4): BLE 16 kbps 40 ms
//! without LBRR, Wi-Fi 24 kbps 20 ms with LBRR, narrow-band routes, and concealment.

use opus_sys::{Application, Bandwidth, Bitrate, Channels, Decoder, Encoder, Vbr};

const FS: u32 = 16_000;

/// Deterministic voiced-speech-like test signal: a 120-180 Hz gliding pitch with eight
/// harmonics under a slow syllable envelope.
fn speech_like(samples: usize) -> Vec<i16> {
    let mut phase = 0.0f64;
    (0..samples)
        .map(|n| {
            let t = n as f64 / f64::from(FS);
            let f0 = 150.0 + 30.0 * (2.0 * std::f64::consts::PI * 1.5 * t).sin();
            phase += 2.0 * std::f64::consts::PI * f0 / f64::from(FS);
            let mut v = 0.0;
            for k in 1..=8 {
                v += (k as f64 * phase).sin() / k as f64;
            }
            let env = 0.6 + 0.4 * (2.0 * std::f64::consts::PI * 3.0 * t).sin();
            (v * env * 6000.0) as i16
        })
        .collect()
}

fn ble_encoder() -> Encoder {
    let mut e = Encoder::new(FS, Channels::Mono, Application::Voip).unwrap();
    e.set_bitrate(Bitrate::BitsPerSecond(16_000)).unwrap();
    e.set_vbr(Vbr::Constrained).unwrap();
    e.set_signal_voice().unwrap();
    e.set_inband_fec(false).unwrap();
    e.set_packet_loss_perc(0).unwrap();
    e.set_dtx(false).unwrap();
    e.set_dred_duration(0).unwrap();
    e.set_max_bandwidth(Bandwidth::Wideband).unwrap();
    e
}

/// Normalised cross-correlation of `a` and `b` (same length).
fn correlation(a: &[i16], b: &[i16]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (f64::from(x), f64::from(y));
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa.sqrt() * bb.sqrt()).max(1.0)
}

#[test]
fn ble_profile_round_trip_is_wideband_and_faithful() {
    const FRAME: usize = 640; // 40 ms
    let input = speech_like(FRAME * 50);
    let mut enc = ble_encoder();
    let mut dec = Decoder::new(FS, Channels::Mono).unwrap();
    dec.set_complexity(7).unwrap();
    let delay = enc.lookahead().unwrap();

    let mut decoded = Vec::with_capacity(input.len());
    let mut bytes = 0usize;
    let mut packet = [0u8; 400];
    for frame in input.as_chunks::<FRAME>().0 {
        let n = enc.encode(frame, &mut packet).unwrap();
        let p = &packet[..n];
        assert_eq!(opus_sys::packet_nb_samples(p, FS).unwrap(), FRAME);
        assert_eq!(opus_sys::packet_bandwidth(p).unwrap(), Bandwidth::Wideband);
        assert!(
            !opus_sys::packet_has_lbrr(p).unwrap(),
            "no LBRR on the BLE profile"
        );
        bytes += n;
        let mut pcm = [0i16; FRAME];
        assert_eq!(dec.decode(Some(p), &mut pcm, false).unwrap(), FRAME);
        decoded.extend_from_slice(&pcm);
    }
    // 16 kbps × 40 ms = 80 bytes per packet. Constrained VBR may spend less on this
    // very predictable signal but never much more than the target.
    let avg = bytes as f64 / 50.0;
    assert!((40.0..=90.0).contains(&avg), "average packet {avg} B");

    let skip = FRAME * 5; // let the codec settle
    let got = &decoded[skip + delay..];
    let want = &input[skip..input.len() - delay];
    let c = correlation(want, got);
    assert!(c > 0.8, "decoded signal correlation {c}");
}

#[test]
fn narrowband_route_limits_the_encoded_bandwidth() {
    let mut enc = ble_encoder();
    enc.set_max_bandwidth(Bandwidth::Narrowband).unwrap();
    let input = speech_like(640 * 5);
    let mut packet = [0u8; 400];
    for frame in input.as_chunks::<640>().0 {
        let n = enc.encode(frame, &mut packet).unwrap();
        assert_eq!(
            opus_sys::packet_bandwidth(&packet[..n]).unwrap(),
            Bandwidth::Narrowband
        );
    }
}

#[test]
fn wifi_fec_profile_carries_lbrr_and_recovers_a_lost_frame() {
    const FRAME: usize = 320; // 20 ms
    let mut enc = Encoder::new(FS, Channels::Mono, Application::Voip).unwrap();
    enc.set_bitrate(Bitrate::BitsPerSecond(24_000)).unwrap();
    enc.set_vbr(Vbr::Constrained).unwrap();
    enc.set_signal_voice().unwrap();
    enc.set_inband_fec(true).unwrap();
    enc.set_packet_loss_perc(5).unwrap();
    let input = speech_like(FRAME * 30);
    let packets: Vec<Vec<u8>> = input
        .as_chunks::<FRAME>()
        .0
        .iter()
        .map(|f| {
            let mut p = [0u8; 400];
            let n = enc.encode(f, &mut p).unwrap();
            p[..n].to_vec()
        })
        .collect();
    let with_lbrr = packets
        .iter()
        .filter(|p| opus_sys::packet_has_lbrr(p).unwrap())
        .count();
    assert!(with_lbrr >= 20, "only {with_lbrr}/30 packets carry LBRR");

    // Lose packet 20; recover it from packet 21's LBRR, then decode 21 normally.
    let mut dec = Decoder::new(FS, Channels::Mono).unwrap();
    let mut pcm = [0i16; FRAME];
    for p in &packets[..20] {
        dec.decode(Some(p), &mut pcm, false).unwrap();
    }
    assert!(opus_sys::packet_has_lbrr(&packets[21]).unwrap());
    assert_eq!(
        dec.decode(Some(&packets[21]), &mut pcm, true).unwrap(),
        FRAME
    );
    let energy: f64 = pcm.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / FRAME as f64;
    assert!(
        energy > 1.0e5,
        "LBRR recovery produced near-silence ({energy})"
    );
    assert_eq!(
        dec.decode(Some(&packets[21]), &mut pcm, false).unwrap(),
        FRAME
    );
}

#[test]
fn concealment_fills_lost_frames_with_deep_plc() {
    const FRAME: usize = 640;
    let mut enc = ble_encoder();
    let mut dec = Decoder::new(FS, Channels::Mono).unwrap();
    dec.set_complexity(5).unwrap();
    let input = speech_like(FRAME * 10);
    let mut packet = [0u8; 400];
    let mut pcm = [0i16; FRAME];
    for frame in input.as_chunks::<FRAME>().0 {
        let n = enc.encode(frame, &mut packet).unwrap();
        dec.decode(Some(&packet[..n]), &mut pcm, false).unwrap();
    }
    assert_eq!(dec.decode(None, &mut pcm, false).unwrap(), FRAME);
    assert_eq!(dec.last_packet_duration().unwrap(), FRAME);
    let energy: f64 = pcm.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / FRAME as f64;
    assert!(
        energy > 1.0e4,
        "PLC after voiced speech should not be silent ({energy})"
    );

    dec.reset().unwrap();
    assert_eq!(
        dec.complexity().unwrap(),
        5,
        "reset keeps the decoder complexity"
    );
}

#[test]
fn burst_reset_gives_identical_packets_for_identical_input() {
    let input = speech_like(640 * 4);
    let mut enc = ble_encoder();
    let run = |enc: &mut Encoder| -> Vec<Vec<u8>> {
        input
            .as_chunks::<640>()
            .0
            .iter()
            .map(|f| {
                let mut p = [0u8; 400];
                let n = enc.encode(f, &mut p).unwrap();
                p[..n].to_vec()
            })
            .collect()
    };
    let first = run(&mut enc);
    enc.reset().unwrap();
    let second = run(&mut enc);
    assert_eq!(
        first, second,
        "OPUS_RESET_STATE must restore the initial state"
    );
}
