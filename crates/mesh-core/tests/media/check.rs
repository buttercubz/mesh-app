//! Audio check (§1.3, §3.2, §7.3): the echo test plays the captured block back 1 s
//! later; the VOX meter streams `audioCheckSample{vadP, gate}` at 20 Hz; the tone check
//! plays its tone then records level and route; a check never runs while live.
//!
//! The session module (WP-10B) owns the check's lifecycle and audio commands; the media
//! module runs its audio from `MediaRequest::AudioCheckStart/Stop`, which these tests
//! send directly (see `rig.rs`).

use mesh_api::{AudioCheckMode, AudioCheckPhase, AudioRouteKind, GateState, TalkMode, UiEvent};
use mesh_core::media::MediaRequest;
use mesh_core::{AudioRouteInfo, CoreModule, EngineMode, Platform};

use super::dsp::tone_amp;
use super::rig::Rig;
use super::world::Source;

fn samples(r: &Rig, from_us: u64, to_us: u64) -> Vec<(u64, f64, GateState, AudioCheckPhase)> {
    r.ui_events()
        .into_iter()
        .filter(|(t, _)| *t >= from_us && *t < to_us)
        .filter_map(|(t, e)| match e {
            UiEvent::AudioCheckSample {
                vad_p, gate, phase, ..
            } => Some((t, vad_p, gate, phase)),
            _ => None,
        })
        .collect()
}

#[test]
fn echo_returns_the_captured_block_one_second_later() {
    let mut r = Rig::new(Platform::WebSim, 31);
    r.source = Source::Noise {
        amp: 3_000,
        seed: 0xBEEF,
    };
    r.run_ms(100);
    r.request(MediaRequest::AudioCheckStart(AudioCheckMode::Echo));
    let started = r.now();
    r.run_ms(2_500);
    let src = r.source.clone();
    let mut checked = 0;
    for (p, out) in &r.out {
        if *p < started + 1_010_000 {
            continue;
        }
        let input = src.block(p - 1_000_000);
        for (a, b) in out.iter().zip(input.iter()) {
            assert!(
                (i32::from(*a) - i32::from(*b)).abs() <= 2,
                "at {p}: {a} vs {b}"
            );
        }
        checked += 1;
    }
    assert!(checked > 100);
    // Before the line filled up the output was silent.
    assert!(
        r.out
            .iter()
            .filter(|(p, _)| *p > started && *p <= started + 1_000_000)
            .all(|(_, b)| b.iter().all(|&s| s == 0))
    );
    // Echo samples report the echo phase at 20 Hz.
    let s = samples(&r, started, started + 2_500_000);
    assert!((49..=51).contains(&s.len()), "20 Hz: {}", s.len());
    assert!(s.iter().all(|x| x.3 == AudioCheckPhase::Echo));
    // Stop: nothing plays afterwards.
    r.request(MediaRequest::AudioCheckStop);
    let stopped = r.now();
    r.run_ms(300);
    assert!(
        r.out
            .iter()
            .filter(|(p, _)| *p > stopped)
            .all(|(_, b)| b.iter().all(|&s| s == 0))
    );
    assert!(samples(&r, stopped + 1, u64::MAX).is_empty());
}

#[test]
fn vox_meter_samples_carry_vad_p_and_gate_at_20hz() {
    let mut r = Rig::new(Platform::WebSim, 32);
    r.source = Source::tone(400.0, 5_000.0, 1_000, 2_000);
    r.run_ms(200);
    r.request(MediaRequest::AudioCheckStart(AudioCheckMode::VoxMeter));
    r.run_ms(2_400);
    let quiet = samples(&r, 400_000, 1_000_000);
    let loud = samples(&r, 1_200_000, 2_000_000);
    assert!((11..=13).contains(&quiet.len()), "20 Hz: {}", quiet.len());
    assert!((15..=17).contains(&loud.len()), "20 Hz: {}", loud.len());
    assert!(
        quiet.iter().all(|s| s.2 == GateState::Closed && s.1 < 0.5),
        "{quiet:?}"
    );
    assert!(
        loud.iter().all(|s| s.2 == GateState::Open && s.1 > 0.65),
        "{loud:?}"
    );
    assert!(loud.iter().all(|s| s.3 == AudioCheckPhase::Meter));
    // The meter plays nothing.
    assert!(r.out.iter().all(|(_, b)| b.iter().all(|&s| s == 0)));
}

#[test]
fn tone_check_plays_then_records_level_and_route() {
    let mut r = Rig::new(Platform::WebSim, 33);
    // The base engine records the route in `ctx.device` before the modules see it.
    r.ctx.device.route = AudioRouteInfo {
        kind: AudioRouteKind::HfpNarrowband,
        input_rate: 8_000,
        output: AudioRouteKind::HfpNarrowband,
        headset_present: true,
    };
    r.source = Source::tone(300.0, 2_000.0, 1_500, 2_500);
    r.run_ms(100);
    r.request(MediaRequest::AudioCheckStart(AudioCheckMode::Tone));
    let t0 = r.now();
    r.run_ms(3_200);
    let tone: Vec<_> = r
        .out
        .iter()
        .filter(|(p, _)| *p > t0 + 50_000 && *p < t0 + 950_000)
        .collect();
    assert!(!tone.is_empty());
    assert!(tone.iter().all(|(_, b)| tone_amp(b, 660.0) > 4_000.0));
    // The record phase plays nothing.
    assert!(
        r.out
            .iter()
            .filter(|(p, _)| *p > t0 + 1_050_000)
            .all(|(_, b)| b.iter().all(|&s| s == 0))
    );
    let ev: Vec<_> = r
        .ui_events()
        .into_iter()
        .filter_map(|(t, e)| match e {
            UiEvent::AudioCheckSample {
                level,
                phase,
                route,
                input_rate,
                ..
            } => Some((t, level, phase, route, input_rate)),
            _ => None,
        })
        .collect();
    assert!(ev.iter().any(|e| e.2 == AudioCheckPhase::Tone));
    assert!(
        ev.iter()
            .all(|e| e.3 == AudioRouteKind::HfpNarrowband && e.4 == 8_000)
    );
    let rec: Vec<_> = ev
        .iter()
        .filter(|e| e.2 == AudioCheckPhase::Record)
        .collect();
    assert!(
        (38..=41).contains(&rec.len()),
        "2 s of 20 Hz record samples: {}",
        rec.len()
    );
    // The recorded level follows the microphone (speech at 1.5-2.5 s).
    let loud = rec.iter().filter(|e| e.0 > 1_600_000 && e.0 < 2_400_000);
    assert!(
        loud.clone().count() > 10 && loud.clone().all(|e| e.1 > 0.02),
        "{rec:?}"
    );
    assert!(rec.iter().filter(|e| e.0 > 2_700_000).all(|e| e.1 < 0.01));
    assert!(
        ev.len() <= 61,
        "no samples after the 3 s check: {}",
        ev.len()
    );
}

#[test]
fn no_check_audio_while_live() {
    let mut r = Rig::new(Platform::WebSim, 34);
    r.ctx.mode = EngineMode::Session { group_id: [7; 16] };
    let ctx = &mut r.ctx;
    r.media
        .on_mode(ctx, EngineMode::Session { group_id: [7; 16] });
    r.request(MediaRequest::GoLive { riders: 1 });
    r.request(MediaRequest::SetTalkMode(TalkMode::Vox));
    assert!(r.media.live());
    r.request(MediaRequest::AudioCheckStart(AudioCheckMode::Echo));
    r.source = Source::Noise {
        amp: 3_000,
        seed: 1,
    };
    r.run_ms(1_500);
    assert!(
        r.ui_events()
            .iter()
            .all(|(_, e)| !matches!(e, UiEvent::AudioCheckSample { .. }))
    );
    assert!(r.out.iter().all(|(_, b)| b.iter().all(|&s| s == 0)));
}
