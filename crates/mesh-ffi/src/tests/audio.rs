//! Audio check without a session and the RT pump of the engine thread.

use super::*;
use crate::api::types::{FfiPlatformSample, FfiRadioAvail, FfiRuntimeStatus, FfiSessionStart};
use crate::rt::{FLAG_HARD_MUTE, FLAG_LATCH_TOGGLE, FLAG_PTT_DOWN};

fn sample() -> FfiPlatformSample {
    FfiPlatformSample {
        wall_ms: Some(1_760_000_000_000),
        gps: None,
        battery: Some(0.8),
        charging: Some(false),
        thermal: None,
        background: false,
        locked: Some(false),
        on_call: false,
        anchor_session: Some(true),
        anchor_location: None,
        anchor_live_activity: None,
        ble: FfiRadioAvail::On,
        aware: FfiRadioAvail::Unsupported,
    }
}

#[test]
fn audio_check_runs_the_engine_thread_without_a_session() {
    let r = Rig::new(20);
    let start = r.cmd(r#"{"type":"audioCheck","op":"start","mode":"echo"}"#);
    assert_eq!(start["ok"], true, "{start}");
    assert_eq!(r.engine.status(), FfiRuntimeStatus::AudioCheck);
    assert!(r.rt.is_active(), "RT rings live during the check");
    assert!(
        r.hooks
            .calls()
            .contains(&Call::Audio(FfiAudioCommand::StartCapture))
    );

    // Capture: three 10 ms blocks reach the engine in order.
    let block = [77i16; 160];
    for _ in 0..3 {
        assert_eq!(r.rt.capture_rt(&block, 0), 160);
    }
    wait_until("capture blocks", || {
        r.probe.capture_blocks.load(SeqCst) >= 3
    });
    assert_eq!(r.probe.first_sample.load(SeqCst), 77);

    // Render: the engine keeps the ring filled with the module's playout (the first
    // queued audio of a session is faded in over RESUME_FADE_SAMPLES).
    let mut out = [0i16; 160];
    wait_until("render fill", || {
        r.rt.render_rt(&mut out, 0) == 160 && out.iter().all(|&s| s == PLAYOUT_VALUE)
    });

    // RT flags: levels follow, the latch edge is delivered once.
    r.rt.set_flags(FLAG_PTT_DOWN | FLAG_LATCH_TOGGLE);
    wait_until("flags with latch", || {
        r.probe
            .flags
            .lock()
            .unwrap()
            .contains(&(FLAG_PTT_DOWN | FLAG_LATCH_TOGGLE))
    });
    wait_until("latch consumed", || {
        r.probe.flags.lock().unwrap().last() == Some(&FLAG_PTT_DOWN)
    });
    r.rt.set_flags(FLAG_HARD_MUTE);
    wait_until("mute", || {
        r.probe.flags.lock().unwrap().last() == Some(&FLAG_HARD_MUTE)
    });

    // Platform samples: the radio state is forwarded once, counters every time.
    r.engine.on_platform_sample(sample());
    r.engine.on_platform_sample(sample());
    wait_until("samples", || {
        let ev = r.probe.events.lock().unwrap();
        ev.iter().filter(|e| e.starts_with("sample:")).count() == 2
    });
    let ev = r.probe.events.lock().unwrap().clone();
    assert_eq!(ev.iter().filter(|e| e.starts_with("radio:")).count(), 1);
    assert_eq!(ev.first().map(String::as_str), Some("radio:On"));

    let stop = r.cmd(r#"{"type":"audioCheck","op":"stop"}"#);
    assert_eq!(stop["ok"], true);
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
    assert!(!r.rt.is_active() && !r.rt.is_claimed());
    assert!(
        r.hooks
            .calls()
            .contains(&Call::Audio(FfiAudioCommand::CloseSession))
    );
}

#[test]
fn starting_a_session_during_a_check_keeps_the_audio_plane() {
    let r = Rig::new(21);
    assert_eq!(
        r.cmd(r#"{"type":"audioCheck","op":"start","mode":"tone"}"#)["ok"],
        true
    );
    let generation = r.rt.stats().generation;
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 4321 })
        .unwrap();
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Running);
    assert!(r.rt.is_active());
    assert_eq!(
        r.rt.stats().generation,
        generation + 1,
        "rings reset at start"
    );
    r.engine.stop();
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
    assert!(!r.rt.is_claimed());
}

#[test]
fn a_second_runtime_runs_without_audio_while_the_plane_is_claimed() {
    let r = Rig::new(22);
    let other = r.rt.claim().unwrap();
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 1 })
        .unwrap();
    assert!(!r.rt.is_active());
    assert!(
        r.hooks
            .count(|c| matches!(c, Call::Log(FfiLogLevel::Warn, m) if m.contains("RT plane")))
            == 1
    );
    r.engine.stop();
    drop(other);
}
