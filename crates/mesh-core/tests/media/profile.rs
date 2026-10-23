//! Capture-side wiring (§6.4, §7.2, §7.4): the encoder profile is chosen at burst start
//! from this node's own links and a heard PATH_HINT, the encoder bandwidth follows the
//! capture route, and the mutes (built-in mic without headset, hard mute) keep capture
//! running while nothing is sent.

use mesh_api::{TalkMode, UiCommand};
use mesh_core::{AudioRouteInfo, AudioRouteKind, Platform, PlatformEvent, rt_flags};
use mesh_wire::mesh::{
    Bandwidth, EncoderProfile, FrameDur, MeshHeader, MeshPayload, PacketType, PathHint, TalkState,
    VoiceCfg, encode_packet,
};

use super::dsp::{concat, tone_amp, window};
use super::world::{OwnedPayload, Source, World};

fn pair(salt: u8) -> World {
    let mut w = World::new(2, Platform::WebSim, salt);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(500);
    w.go_live(0, TalkMode::Ptt);
    w.go_live(1, TalkMode::Ptt);
    w.nodes[0].source = Source::tone(500.0, 6_000.0, 0, 60_000);
    w
}

/// PTT burst on node 0 over `[from_ms, to_ms)`.
fn burst(w: &mut World, from_ms: u64, to_ms: u64) {
    w.run_until_ms(from_ms);
    w.set_flags(0, rt_flags::PTT_DOWN);
    w.run_until_ms(to_ms);
    w.set_flags(0, 0);
}

/// `(TALK START profile, first VOICE cfg)` of every burst node 0 sent after `from_us`.
fn bursts(w: &World, from_us: u64) -> Vec<(EncoderProfile, VoiceCfg)> {
    let p = w.sent_payloads(0);
    let mut v = Vec::new();
    for (i, (t, _, x)) in p.iter().enumerate() {
        let OwnedPayload::Talk(k) = x else { continue };
        if *t < from_us || k.state != TalkState::Start {
            continue;
        }
        let cfg = p[i..].iter().find_map(|(_, _, y)| match y {
            OwnedPayload::Voice { burst, cfg, .. } if *burst == k.burst => Some(VoiceCfg(*cfg)),
            _ => None,
        });
        v.push((k.profile, cfg.expect("voice after START")));
    }
    v
}

#[test]
fn profile_follows_path_hint_and_bandwidth_follows_the_capture_route() {
    let mut w = pair(71);
    // Burst 1: BLE link, no hint → BLE16, 40 ms, wideband.
    burst(&mut w, 1_000, 1_400);
    // A relay floods PATH_HINT{BLE12, target all}.
    let mut h = MeshHeader::new(PacketType::PathHint, 0x0B0B, 9, 1);
    h.ttl = 3;
    let hint = PathHint {
        profile: EncoderProfile::Ble12,
        at: 0x0B0B,
        target: 0,
    };
    let pkt = encode_packet(&h, &MeshPayload::PathHint(hint)).unwrap();
    w.nodes[1].inject.borrow_mut().push(pkt);
    // Burst 2: BLE12 for the 30 s hold.
    burst(&mut w, 1_800, 2_200);
    // The headset moves to CVSD (8 kHz capture): burst 3 encodes narrow band.
    w.event(
        0,
        PlatformEvent::AudioRoute(AudioRouteInfo {
            kind: AudioRouteKind::HfpNarrowband,
            input_rate: 8_000,
            output: AudioRouteKind::HfpNarrowband,
            headset_present: true,
        }),
    );
    burst(&mut w, 2_600, 3_200);
    w.run_until_ms(3_400);
    let b = bursts(&w, 0);
    assert_eq!(b.len(), 3, "{b:?}");
    assert_eq!(b[0].0, EncoderProfile::Ble16);
    assert_eq!(b[0].1.dur(), FrameDur::Ms40);
    assert_eq!(b[0].1.bw(), Bandwidth::Wide);
    assert!(!b[0].1.lbrr());
    assert_eq!(b[1].0, EncoderProfile::Ble12);
    assert_eq!(b[1].1.bw(), Bandwidth::Wide);
    assert_eq!(b[2].0, EncoderProfile::Ble12, "the hint holds 30 s");
    assert_eq!(b[2].1.bw(), Bandwidth::Narrow);
    // The narrow-band burst is still heard.
    let out = concat(&window(&w.nodes[1].out, 2_750, 3_150));
    assert!(tone_amp(&out, 500.0) > 3_000.0);
}

#[test]
fn builtin_mic_mute_and_hard_mute_send_nothing_until_lifted() {
    let mut w = World::new(2, Platform::WebSim, 72);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(500);
    w.go_live(0, TalkMode::Vox);
    w.go_live(1, TalkMode::Vox);
    // Speech at 1.0-1.6 s, 2.0-2.6 s and 3.0-3.6 s.
    w.nodes[0].source = Source::Sum(vec![
        Source::tone(400.0, 5_000.0, 1_000, 1_600),
        Source::tone(400.0, 5_000.0, 2_000, 2_600),
        Source::tone(400.0, 5_000.0, 3_000, 3_600),
    ]);
    // Motorcycle on the phone mic: bit2 set, no override.
    w.run_until_ms(800);
    w.set_flags(0, rt_flags::BUILTIN_MIC_MUTE);
    w.run_until_ms(1_900);
    assert!(w.sent_payloads(0).is_empty(), "muted: nothing sent");
    let s = w.snapshot(0).expect("snapshot");
    assert!(s.audio.tx_muted_no_headset);
    // The rider overrides it: speech goes out.
    assert!(
        w.command(0, UiCommand::SetBuiltinMicTx { allowed: true })
            .is_ok()
    );
    w.run_until_ms(2_900);
    let voice = |w: &World, a: u64, b: u64| {
        w.sent_payloads(0)
            .iter()
            .filter(|(t, _, p)| *t >= a && *t < b && matches!(p, OwnedPayload::Voice { .. }))
            .count()
    };
    assert!(voice(&w, 2_000_000, 2_900_000) > 10);
    assert!(!w.snapshot(0).unwrap().audio.tx_muted_no_headset);
    // Hard mute (bit1) silences it again; capture keeps running.
    w.set_flags(0, rt_flags::BUILTIN_MIC_MUTE | rt_flags::HARD_MUTE);
    w.run_until_ms(3_800);
    assert_eq!(voice(&w, 2_950_000, 3_800_000), 0);
    // B heard the overridden burst only.
    let heard = concat(&window(&w.nodes[1].out, 2_150, 2_650));
    assert!(tone_amp(&heard, 400.0) > 2_500.0);
    let quiet = concat(&window(&w.nodes[1].out, 1_100, 1_600));
    assert!(tone_amp(&quiet, 400.0) < 100.0);
}
