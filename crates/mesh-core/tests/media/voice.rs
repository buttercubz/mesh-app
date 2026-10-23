//! End-to-end voice: two and four engines exchanging SimCodec voice, floor-ranked
//! mixing with near-talker rejection, PTT access time with pre-roll, TALK
//! START/KEEP/STOP with `elapsed_10ms`, burst-relative `ts`, snapshot talkers.

use mesh_api::{TalkMode, UiEvent};
use mesh_core::{Platform, rt_flags};
use mesh_voice::VoiceCfg;
use mesh_wire::mesh::{FloorRejectReason, TalkState};

use super::dsp::{concat, tone_amp, window};
use super::world::{OwnedPayload, Source, World};

// Not harmonically related (SimCodec's ADPCM adds harmonics of each tone).
const TONES: [f64; 3] = [700.0, 1_100.0, 1_900.0];

fn talk_denied(w: &World, i: usize) -> bool {
    w.ui_events(i)
        .iter()
        .any(|(_, e)| matches!(e, UiEvent::TalkDenied { .. }))
}

fn origin_of(w: &World, i: usize) -> u16 {
    w.sent_packets(i)
        .first()
        .expect("node sent packets")
        .2
        .origin
}

#[test]
fn two_engines_exchange_voice_and_snapshot_shows_the_talker() {
    let mut w = World::new(2, Platform::WebSim, 3);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(500);
    w.go_live(0, TalkMode::Ptt);
    w.go_live(1, TalkMode::Ptt);
    w.nodes[0].source = Source::tone(1_000.0, 6_000.0, 0, 60_000);
    w.run_ms(500);
    w.set_flags(0, rt_flags::PTT_DOWN);
    w.run_ms(1_500);
    let out = concat(&window(&w.nodes[1].out, 1_300, 2_400));
    let a = tone_amp(&out, 1_000.0);
    assert!(a > 3_500.0, "1 kHz heard at B: {a}");
    let s = w.snapshot(1).expect("B snapshot");
    assert_eq!(s.talkers.len(), 1);
    assert_eq!(s.talkers[0].node_id, origin_of(&w, 0));
    assert_eq!(s.talkers[0].hops, 1);
    assert!(s.talkers[0].since_ms > 800, "{:?}", s.talkers[0]);
    let me = w.snapshot(0).expect("A snapshot");
    assert_eq!(me.talkers[0].hops, 0, "A shows itself talking");
    // Release: B's stream ends, the talker leaves the snapshot.
    w.set_flags(0, 0);
    w.run_ms(1_000);
    assert!(w.snapshot(1).unwrap().talkers.is_empty());
    let tail = concat(&window(&w.nodes[1].out, 2_900, 3_000));
    assert!(tone_amp(&tail, 1_000.0) < 50.0, "silence after the burst");
}

#[test]
fn floor_ranked_mixing_rejects_the_third_simultaneous_talker() {
    let mut w = World::new(4, Platform::WebSim, 4);
    w.start_all();
    w.connect_all();
    w.run_ms(500);
    for i in 0..4 {
        w.go_live(i, TalkMode::Ptt);
    }
    for (i, hz) in TONES.iter().enumerate() {
        w.nodes[i].source = Source::tone(*hz, 6_000.0, 0, 60_000);
    }
    w.run_ms(500);
    // Three PTT presses within one engine tick: nobody sees the others first.
    for i in 0..3 {
        w.set_flags(i, rt_flags::PTT_DOWN);
    }
    w.run_ms(2_000);
    let blocked: Vec<usize> = (0..3).filter(|&i| talk_denied(&w, i)).collect();
    assert_eq!(blocked.len(), 1, "exactly one talker over the BLE cap of 2");
    let loser = blocked[0];
    let winners: Vec<usize> = (0..3).filter(|&i| i != loser).collect();
    // The listener mixed the two best-ranked bursts and never the third.
    let d = concat(&window(&w.nodes[3].out, 1_400, 2_900));
    for &i in &winners {
        let a = tone_amp(&d, TONES[i]);
        assert!(
            (2_500.0..5_500.0).contains(&a),
            "winner {i} mixed at -3 dB: {a}"
        );
    }
    // A mixed stream would show ≈ 4,200-4,900 here; SimCodec's 2-bit ADPCM puts
    // intermodulation products of the two winners at ≈ 550-600 on arbitrary bins.
    let l = tone_amp(&d, TONES[loser]);
    assert!(l < 1_000.0, "rejected talker never played at D: {l}");
    // Not even its first buffered frames after the rejection (1.04-1.1 s).
    let early = concat(&window(&w.nodes[3].out, 1_060, 1_100));
    let le = tone_amp(&early, TONES[loser]);
    assert!(le < 1_000.0, "rejected talker cut at the rejection: {le}");
    // D sent the near-talker FLOOR_REJECT{OUTRANKED} for the loser's burst.
    let loser_origin = origin_of(&w, loser);
    let rejects: Vec<_> = w
        .sent_payloads(3)
        .into_iter()
        .filter_map(|(_, _, p)| match p {
            OwnedPayload::Reject(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(rejects.len(), 1);
    assert_eq!(rejects[0].target, loser_origin);
    assert_eq!(rejects[0].reason, FloorRejectReason::Outranked);
    // The loser stopped at once (TALK STOP, no flush) and its later voice is gone.
    let stop_at = w
        .sent_payloads(loser)
        .into_iter()
        .find_map(|(t, _, p)| match p {
            OwnedPayload::Talk(t2) if t2.state == TalkState::Stop => Some(t),
            _ => None,
        })
        .expect("loser sent TALK STOP");
    assert!(stop_at < 1_600_000, "stopped within 100 ms: {stop_at}");
    // D's snapshot lists the two winners in rank order.
    let s = w.snapshot(3).expect("D snapshot");
    let mut got: Vec<u16> = s.talkers.iter().map(|t| t.node_id).collect();
    let mut want: Vec<u16> = winners.iter().map(|&i| origin_of(&w, i)).collect();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want);
    // Winners hear each other.
    let a_out = concat(&window(&w.nodes[winners[0]].out, 1_600, 2_900));
    assert!(tone_amp(&a_out, TONES[winners[1]]) > 2_500.0);
}

#[test]
fn ptt_to_first_voice_frame_within_5ms_with_preroll_and_talk_fields() {
    let mut w = World::new(2, Platform::WebSim, 5);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(500);
    w.go_live(0, TalkMode::Ptt);
    w.go_live(1, TalkMode::Ptt);
    w.nodes[0].source = Source::tone(700.0, 5_000.0, 0, 60_000);
    w.run_ms(503);
    let t0 = w.now;
    w.set_flags(0, rt_flags::PTT_DOWN);
    w.run_ms(2_500);
    w.set_flags(0, 0);
    let t_up = w.now;
    w.run_ms(300);
    let p = w.sent_payloads(0);
    let (t1, first) = p
        .iter()
        .find_map(|(t, _, x)| match x {
            OwnedPayload::Voice { ts, cfg, .. } => Some((*t, (*ts, *cfg))),
            _ => None,
        })
        .expect("voice sent");
    assert!(
        t1 >= t0 && t1 - t0 <= 5_000,
        "PTT→first VOICE {} µs",
        t1 - t0
    );
    let cfg = VoiceCfg(first.1);
    assert_eq!(first.0, 0, "ts of frame 0");
    assert!(
        cfg.first() && cfg.preroll(),
        "frame 0 is FIRST and carries the pre-roll"
    );
    // TALK START goes out in the same instant, before frame 0, with elapsed 0.
    let start = p
        .iter()
        .position(|(_, _, x)| matches!(x, OwnedPayload::Talk(t) if t.state == TalkState::Start))
        .unwrap();
    let v0 = p
        .iter()
        .position(|(_, _, x)| matches!(x, OwnedPayload::Voice { .. }))
        .unwrap();
    assert!(start < v0 && p[start].0 == t1);
    // Burst-relative ts: 4n for a 40 ms profile, contiguous.
    let ts: Vec<u16> = p
        .iter()
        .filter_map(|(_, _, x)| match x {
            OwnedPayload::Voice { ts, .. } => Some(*ts),
            _ => None,
        })
        .collect();
    assert!(ts.len() > 50);
    for (n, t) in ts.iter().enumerate() {
        assert_eq!(*t as usize, 4 * n);
    }
    // KEEP every second with elapsed_10ms; STOP twice 60 ms apart after PTT up.
    let talks: Vec<(u64, mesh_wire::mesh::Talk)> = p
        .iter()
        .filter_map(|(t, _, x)| match x {
            OwnedPayload::Talk(k) => Some((*t, *k)),
            _ => None,
        })
        .collect();
    let keeps: Vec<_> = talks
        .iter()
        .filter(|(_, k)| k.state == TalkState::Keep)
        .collect();
    assert_eq!(keeps.len(), 2);
    for (n, (t, k)) in keeps.iter().enumerate() {
        let want = 100 * (n as u16 + 1);
        assert!(k.elapsed_10ms.abs_diff(want) <= 1, "{k:?}");
        assert!((t - t1).abs_diff(1_000_000 * (n as u64 + 1)) <= 1_000);
    }
    let stops: Vec<_> = talks
        .iter()
        .filter(|(_, k)| k.state == TalkState::Stop)
        .collect();
    assert_eq!(stops.len(), 2, "STOP sent twice");
    assert!(stops[0].0 >= t_up && stops[0].0 - t_up <= 1_000);
    assert!((stops[1].0 - stops[0].0).abs_diff(60_000) <= 1_000);
    assert_eq!(stops[0].1.elapsed_10ms, stops[1].1.elapsed_10ms);
    assert!(stops[0].1.elapsed_10ms.abs_diff(250) <= 1);
}

#[cfg(feature = "opus")]
#[test]
fn opus_voice_between_device_platform_engines() {
    let mut w = World::new(2, Platform::Ios, 6);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(500);
    w.go_live(0, TalkMode::Ptt);
    w.go_live(1, TalkMode::Ptt);
    w.nodes[0].source = Source::tone(600.0, 6_000.0, 0, 60_000);
    w.run_ms(500);
    w.set_flags(0, rt_flags::PTT_DOWN);
    w.run_ms(1_500);
    let out = concat(&window(&w.nodes[1].out, 1_400, 2_400));
    let a = tone_amp(&out, 600.0);
    assert!(a > 2_500.0, "Opus 600 Hz heard at B: {a}");
}
