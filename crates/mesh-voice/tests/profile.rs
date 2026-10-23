//! §7.4 encoder profile table, selector hysteresis and VOICE cfg bits.

use mesh_voice::packetizer::VoiceCfg;
use mesh_voice::profile::{BLE_DRED_DURATION_10MS, FEC_OFF_HOLD_US};
use mesh_voice::{
    AudioBandwidth, EncoderConfig, FrameDuration, Profile, ProfileInputs, ProfileSelector,
};
use proptest::prelude::*;

#[test]
fn table_matches_design_7_4() {
    // (profile, frame, kbps, lbrr, complexity)
    let rows = [
        (Profile::Ble16, FrameDuration::Ms40, 16_000, false, 8),
        (Profile::Ble12, FrameDuration::Ms40, 12_000, false, 8),
        (Profile::Wifi20, FrameDuration::Ms20, 20_000, false, 9),
        (Profile::Wifi24Fec, FrameDuration::Ms20, 24_000, true, 9),
        (Profile::BleDred, FrameDuration::Ms40, 20_000, false, 8),
    ];
    for (p, frame, bps, lbrr, cx) in rows {
        let c = EncoderConfig::for_profile(p, 16_000, 1.0);
        assert_eq!(c.profile, p);
        assert_eq!(c.frame, frame, "{p:?} frame");
        assert_eq!(c.bitrate_bps, bps, "{p:?} bitrate");
        assert_eq!(c.inband_fec, lbrr, "{p:?} LBRR");
        assert_eq!(c.complexity, cx, "{p:?} complexity");
        assert!(!c.dtx, "{p:?} DTX must be off");
        assert!(c.constrained_vbr, "{p:?} constrained VBR");
        assert_eq!(
            c.max_bandwidth,
            AudioBandwidth::Wb,
            "{p:?} WB on a 16 kHz route"
        );
        assert!(
            matches!(c.frame, FrameDuration::Ms20 | FrameDuration::Ms40),
            "never 10/60 ms"
        );
    }
    assert_eq!(Profile::Ble16.talk_id(), 1);
    assert_eq!(Profile::BleDred.talk_id(), 5);
    for p in Profile::ALL {
        assert_eq!(Profile::from_talk_id(p.talk_id()), Some(p));
    }
    assert_eq!(Profile::from_talk_id(0), None);
}

#[test]
fn loss_hint_is_zero_whenever_lbrr_is_off() {
    for loss in [0.0, 1.0, 2.9, 3.0, 7.5, 30.0, 100.0] {
        for p in [Profile::Ble16, Profile::Ble12, Profile::Wifi20] {
            let c = EncoderConfig::for_profile(p, 16_000, loss);
            assert!(!c.inband_fec);
            assert_eq!(c.packet_loss_perc, 0, "{p:?} at {loss} %");
            assert_eq!(c.dred_duration_10ms, 0);
        }
    }
}

#[test]
fn wifi24fec_loss_hint_is_clamped_5_to_25() {
    let hint = |loss| EncoderConfig::for_profile(Profile::Wifi24Fec, 16_000, loss).packet_loss_perc;
    assert_eq!(hint(0.0), 5);
    assert_eq!(hint(3.0), 5);
    assert_eq!(hint(12.4), 12);
    assert_eq!(hint(40.0), 25);
    assert_eq!(hint(f32::NAN), 5);
}

#[test]
fn dred_never_with_lbrr() {
    for p in Profile::ALL {
        let c = EncoderConfig::for_profile(p, 16_000, 10.0);
        assert!(!(c.inband_fec && c.dred_duration_10ms > 0), "{p:?}");
    }
    let d = EncoderConfig::for_profile(Profile::BleDred, 16_000, 6.2);
    assert_eq!(d.dred_duration_10ms, BLE_DRED_DURATION_10MS);
    assert_eq!(
        d.packet_loss_perc, 6,
        "BLE-DRED uses the measured hint (table)"
    );
}

#[test]
fn bandwidth_follows_capture_route() {
    for p in Profile::ALL {
        assert_eq!(
            EncoderConfig::for_profile(p, 8_000, 0.0).max_bandwidth,
            AudioBandwidth::Nb
        );
        assert_eq!(
            EncoderConfig::for_profile(p, 12_000, 0.0).max_bandwidth,
            AudioBandwidth::Mb
        );
        assert_eq!(
            EncoderConfig::for_profile(p, 16_000, 0.0).max_bandwidth,
            AudioBandwidth::Wb
        );
        // v1 ceiling: 32/48 kHz routes stay wideband.
        assert_eq!(
            EncoderConfig::for_profile(p, 48_000, 0.0).max_bandwidth,
            AudioBandwidth::Wb
        );
    }
}

#[test]
fn thermal_lowers_ble12_complexity_only() {
    let c = EncoderConfig::for_profile(Profile::Ble12, 16_000, 0.0).with_thermal(true);
    assert_eq!(c.complexity, 5);
    let c = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0).with_thermal(true);
    assert_eq!(c.complexity, 8);
}

#[test]
fn selector_rules_and_fec_hysteresis() {
    let mut s = ProfileSelector::new();
    let ble = ProfileInputs {
        any_ble_link: true,
        ..Default::default()
    };
    assert_eq!(s.select(0, &ble), Profile::Ble16);
    for flag in 0..3 {
        let mut i = ble;
        match flag {
            0 => i.path_hint_ble12 = true,
            1 => i.cap_pacing_one = true,
            _ => i.thermal_serious = true,
        }
        assert_eq!(s.select(0, &i), Profile::Ble12);
    }
    assert_eq!(
        s.select(
            0,
            &ProfileInputs {
                dred_allowed: true,
                ..ble
            }
        ),
        Profile::BleDred
    );
    // Constrained BLE wins over the DRED experiment.
    assert_eq!(
        s.select(
            0,
            &ProfileInputs {
                dred_allowed: true,
                cap_pacing_one: true,
                ..ble
            }
        ),
        Profile::Ble12
    );

    let wifi = |loss| ProfileInputs {
        worst_loss_pct: loss,
        ..Default::default()
    };
    let mut s = ProfileSelector::new();
    assert_eq!(s.select(0, &wifi(1.0)), Profile::Wifi20);
    assert_eq!(s.select(1_000_000, &wifi(2.9)), Profile::Wifi20);
    assert_eq!(s.select(2_000_000, &wifi(3.0)), Profile::Wifi24Fec);
    // Between 2 % and 3 %: stays on.
    assert_eq!(s.select(3_000_000, &wifi(2.5)), Profile::Wifi24Fec);
    // Below 2 %: needs 10 s.
    let t0 = 4_000_000;
    assert_eq!(s.select(t0, &wifi(1.0)), Profile::Wifi24Fec);
    assert_eq!(
        s.select(t0 + FEC_OFF_HOLD_US - 1, &wifi(1.0)),
        Profile::Wifi24Fec
    );
    assert_eq!(s.select(t0 + FEC_OFF_HOLD_US, &wifi(1.0)), Profile::Wifi20);
    // A bump above 2 % restarts the hold.
    assert_eq!(
        s.select(t0 + FEC_OFF_HOLD_US + 1, &wifi(4.0)),
        Profile::Wifi24Fec
    );
    let t1 = t0 + FEC_OFF_HOLD_US + 2;
    assert_eq!(s.select(t1, &wifi(0.5)), Profile::Wifi24Fec);
    assert_eq!(s.select(t1 + 5_000_000, &wifi(2.5)), Profile::Wifi24Fec);
    assert_eq!(s.select(t1 + 9_000_000, &wifi(0.5)), Profile::Wifi24Fec);
    assert_eq!(s.select(t1 + 19_000_000, &wifi(0.5)), Profile::Wifi20);
}

#[test]
fn voice_cfg_bits() {
    let c = VoiceCfg::new(
        FrameDuration::Ms40,
        AudioBandwidth::Wb,
        false,
        false,
        true,
        true,
    );
    assert_eq!(c.0, 0b1010_0011);
    assert_eq!(c.duration(), FrameDuration::Ms40);
    assert_eq!(c.bandwidth(), Some(AudioBandwidth::Wb));
    assert!(c.first() && c.preroll() && !c.lbrr() && !c.dred());
    assert_eq!(VoiceCfg(0b0011_0000).bandwidth(), None, "bw 3 is reserved");
}

proptest! {
    #[test]
    fn cfg_roundtrip(dur in 0u8..4, bw in 0u8..3, l: bool, d: bool, f: bool, p: bool) {
        let dur = FrameDuration::from_cfg_bits(dur);
        let bw = AudioBandwidth::from_cfg_bits(bw).unwrap();
        let c = VoiceCfg::new(dur, bw, l, d, f, p);
        prop_assert_eq!(c.duration(), dur);
        prop_assert_eq!(c.bandwidth(), Some(bw));
        prop_assert_eq!((c.lbrr(), c.dred(), c.first(), c.preroll()), (l, d, f, p));
    }

    #[test]
    fn loss_hint_zero_without_lbrr_any_input(loss in -10f32..200.0, rate in 4_000u32..96_000, pi in 0usize..5) {
        let c = EncoderConfig::for_profile(Profile::ALL[pi], rate, loss);
        if !c.inband_fec && c.dred_duration_10ms == 0 {
            prop_assert_eq!(c.packet_loss_perc, 0);
        }
        if c.inband_fec {
            prop_assert!((5..=25).contains(&c.packet_loss_perc));
            prop_assert_eq!(c.bitrate_bps, 24_000);
        }
    }
}
