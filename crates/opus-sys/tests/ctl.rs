//! Each fixed-arity ctl wrapper sets a value and reads it back (WP-06 acceptance), using
//! two different non-default values so a default cannot pass by coincidence.

use opus_sys::{
    Application, Bandwidth, Bitrate, Channels, Decoder, Encoder, MAX_DRED_DURATION, Signal, Vbr,
};

fn voip16k() -> Encoder {
    Encoder::new(16_000, Channels::Mono, Application::Voip).expect("16 kHz mono VOIP encoder")
}

#[test]
fn library_is_libopus_1_6_1() {
    assert!(
        opus_sys::version().starts_with("libopus 1.6.1"),
        "{}",
        opus_sys::version()
    );
}

#[test]
fn bitrate_round_trips() {
    let mut e = voip16k();
    for bps in [16_000, 24_000, 12_000] {
        e.set_bitrate(Bitrate::BitsPerSecond(bps)).unwrap();
        assert_eq!(e.bitrate().unwrap(), bps);
    }
}

#[test]
fn complexity_round_trips() {
    let mut e = voip16k();
    for c in [3, 10, 0, 7] {
        e.set_complexity(c).unwrap();
        assert_eq!(e.complexity().unwrap(), c);
    }
    assert!(e.set_complexity(11).is_err(), "out of range is rejected");
    assert_eq!(
        e.complexity().unwrap(),
        7,
        "a rejected set leaves the value alone"
    );
}

#[test]
fn vbr_and_constraint_round_trip() {
    let mut e = voip16k();
    for v in [
        Vbr::Off,
        Vbr::Constrained,
        Vbr::Unconstrained,
        Vbr::Constrained,
    ] {
        e.set_vbr(v).unwrap();
        assert_eq!(e.vbr().unwrap(), v);
    }
}

#[test]
fn inband_fec_round_trips() {
    let mut e = voip16k();
    for on in [true, false, true] {
        e.set_inband_fec(on).unwrap();
        assert_eq!(e.inband_fec().unwrap(), on);
    }
}

#[test]
fn packet_loss_perc_round_trips() {
    let mut e = voip16k();
    for p in [5, 0, 15] {
        e.set_packet_loss_perc(p).unwrap();
        assert_eq!(e.packet_loss_perc().unwrap(), p);
    }
    assert!(e.set_packet_loss_perc(101).is_err());
}

#[test]
fn dtx_round_trips() {
    let mut e = voip16k();
    for on in [true, false] {
        e.set_dtx(on).unwrap();
        assert_eq!(e.dtx().unwrap(), on);
    }
}

#[test]
fn max_bandwidth_round_trips() {
    let mut e = voip16k();
    for bw in [
        Bandwidth::Narrowband,
        Bandwidth::Wideband,
        Bandwidth::Mediumband,
    ] {
        e.set_max_bandwidth(bw).unwrap();
        assert_eq!(e.max_bandwidth().unwrap(), bw);
    }
}

#[test]
fn signal_round_trips() {
    let mut e = voip16k();
    assert_eq!(e.signal().unwrap(), Signal::Auto);
    e.set_signal_voice().unwrap();
    assert_eq!(e.signal().unwrap(), Signal::Voice);
    e.set_signal(Signal::Music).unwrap();
    assert_eq!(e.signal().unwrap(), Signal::Music);
}

#[test]
fn dred_duration_round_trips_so_dred_is_compiled_in() {
    let mut e = voip16k();
    // Without ENABLE_DRED libopus answers OPUS_UNIMPLEMENTED here.
    for d in [10, MAX_DRED_DURATION, 0] {
        e.set_dred_duration(d).unwrap();
        assert_eq!(e.dred_duration().unwrap(), d);
    }
    assert!(e.set_dred_duration(MAX_DRED_DURATION + 1).is_err());
}

#[test]
fn lookahead_is_6_5_ms_at_16_khz() {
    // Fs/400 + delay compensation Fs/250 = 40 + 64 samples (DESIGN §7.9: 46.5 ms with
    // a 40 ms frame).
    assert_eq!(voip16k().lookahead().unwrap(), 104);
}

#[test]
fn reset_keeps_every_setting() {
    let mut e = voip16k();
    e.set_bitrate(Bitrate::BitsPerSecond(16_000)).unwrap();
    e.set_complexity(5).unwrap();
    e.set_vbr(Vbr::Constrained).unwrap();
    e.set_max_bandwidth(Bandwidth::Wideband).unwrap();
    e.set_signal_voice().unwrap();
    e.reset().unwrap();
    assert_eq!(e.bitrate().unwrap(), 16_000);
    assert_eq!(e.complexity().unwrap(), 5);
    assert_eq!(e.vbr().unwrap(), Vbr::Constrained);
    assert_eq!(e.max_bandwidth().unwrap(), Bandwidth::Wideband);
    assert_eq!(e.signal().unwrap(), Signal::Voice);
}

#[test]
fn decoder_complexity_round_trips() {
    let mut d = Decoder::new(16_000, Channels::Mono).unwrap();
    for c in [5, 7, 0, 10] {
        d.set_complexity(c).unwrap();
        assert_eq!(d.complexity().unwrap(), c);
    }
    assert!(d.set_complexity(11).is_err());
}

#[test]
fn bad_arguments_are_errors_not_crashes() {
    let err = Encoder::new(44_100, Channels::Mono, Application::Voip).unwrap_err();
    assert_eq!(err.code(), opus_sys::ffi::OPUS_BAD_ARG);
    assert!(err.to_string().contains("invalid argument"), "{err}");
    assert!(Decoder::new(11_025, Channels::Stereo).is_err());

    let mut e = voip16k();
    let mut out = [0u8; 400];
    assert!(
        e.encode(&[0i16; 100], &mut out).is_err(),
        "100 samples is not a valid frame"
    );
    assert!(e.encode(&[], &mut out).is_err());
    let mut d = Decoder::new(16_000, Channels::Stereo).unwrap();
    assert!(
        d.decode(None, &mut [0i16; 641], false).is_err(),
        "odd length with 2 channels"
    );
    assert!(d.decode(Some(&[]), &mut [0i16; 640], false).is_err());
}

#[test]
fn codec_handles_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Encoder>();
    assert_send::<Decoder>();
}
