//! Talk FSM (§6.4) and the single-hangover property through the real capture chain.

mod common;

use common::{FakeEnv, Noise, blocks, sine};
use mesh_voice::fsm::{
    MAX_BURST_PTT_US, MAX_BURST_VOX_US, PTT_BUSY_REPEAT_US, RejectReason, StopReason,
    VOX_HOLDOFF_US,
};
use mesh_voice::{
    BLOCK_US, CaptureChain, Earcon, EnergyVad, TalkFsm, TalkInput, TalkMode, TalkOutput,
    TalkOutputs, TalkState, VadEvent, VadSensitivity,
};
use proptest::prelude::*;

fn has_start(o: &TalkOutputs) -> bool {
    o.contains(|x| matches!(x, TalkOutput::StartBurst { .. }))
}

fn stop_of(o: &TalkOutputs) -> Option<(StopReason, bool, bool)> {
    o.iter().find_map(|x| match *x {
        TalkOutput::StopBurst {
            reason,
            flush,
            stop_twice,
            ..
        } => Some((reason, flush, stop_twice)),
        _ => None,
    })
}

fn has_earcon(o: &TalkOutputs, e: Earcon) -> bool {
    o.contains(|x| *x == TalkOutput::Earcon(e))
}

#[test]
fn vox_release_is_exactly_one_300ms_hangover_after_speech() {
    // 1 s of noise, 1 s of "speech" (a tone well above the floor), 1 s of noise.
    let mut pcm = Noise::new(1).pcm(16_000, -60.0);
    pcm.extend(sine(250.0, -18.0, 16_000));
    pcm.extend(Noise::new(2).pcm(16_000, -60.0));
    let speech_end_us = 2_000_000u64;

    let mut chain = CaptureChain::new(Box::new(EnergyVad::new()), VadSensitivity::Medium);
    let mut fsm = TalkFsm::new(TalkMode::Vox, 10);
    let mut env = FakeEnv::admitting();
    let (mut started, mut stopped, mut vad_off_at) = (None, None, None);
    // End of the last block the VAD still scored as speech (p ≥ θ_off).
    let mut last_speech_end = 0u64;
    for (i, mut b) in blocks(&pcm).into_iter().enumerate() {
        // `now` = end of this block (the tick that has just captured it).
        let now = (i as u64 + 1) * BLOCK_US;
        let f = chain.process(&mut b);
        if f.vad_p >= VadSensitivity::Medium.theta_off() {
            last_speech_end = now;
        }
        let mut outs = Vec::new();
        match f.event {
            Some(VadEvent::On) => outs.push(fsm.handle(now, TalkInput::VadOn, &mut env)),
            Some(VadEvent::Off) => {
                vad_off_at = Some(now);
                outs.push(fsm.handle(now, TalkInput::VadOff, &mut env));
            }
            None => {}
        }
        outs.push(fsm.handle(now, TalkInput::Block { vad_p: f.vad_p }, &mut env));
        for o in &outs {
            if has_start(o) {
                started = Some(now);
            }
            if let Some((r, flush, twice)) = stop_of(o) {
                assert_eq!((r, flush, twice), (StopReason::VoxRelease, true, false));
                stopped = Some(now);
            }
        }
    }
    let started = started.expect("burst started");
    assert!(
        started > 1_000_000 && started <= 1_030_000,
        "onset within 2-3 blocks: {started}"
    );
    let stopped = stopped.expect("burst released");
    assert_eq!(
        vad_off_at,
        Some(stopped),
        "the FSM adds no hangover of its own"
    );
    // The HPF tail of the abrupt tone cut keeps the first block after it "speech".
    assert!(last_speech_end >= speech_end_us && last_speech_end <= speech_end_us + BLOCK_US);
    assert_eq!(
        stopped - last_speech_end,
        300_000,
        "exactly one 300 ms hangover"
    );
    assert_eq!(fsm.state(), TalkState::Idle);
}

#[test]
fn ptt_press_release_and_stop_twice() {
    let mut f = TalkFsm::new(TalkMode::Ptt, 0);
    let mut env = FakeEnv::admitting();
    let o = f.handle(0, TalkInput::PttDown, &mut env);
    assert_eq!(
        o.iter().next(),
        Some(&TalkOutput::StartBurst {
            burst: 0,
            mode: TalkMode::Ptt
        })
    );
    assert_eq!(f.state(), TalkState::Talking);
    // VAD edges do nothing in PTT mode.
    assert!(f.handle(10, TalkInput::VadOff, &mut env).is_empty());
    let o = f.handle(500_000, TalkInput::PttUp, &mut env);
    assert_eq!(stop_of(&o), Some((StopReason::PttUp, true, true)));
    assert_eq!(f.state(), TalkState::Idle);
    let o = f.handle(600_000, TalkInput::PttDown, &mut env);
    assert_eq!(
        o.iter().next(),
        Some(&TalkOutput::StartBurst {
            burst: 1,
            mode: TalkMode::Ptt
        })
    );
}

#[test]
fn latch_toggle_acts_like_a_latch_tap() {
    let mut f = TalkFsm::new(TalkMode::Ptt, 0);
    let mut env = FakeEnv::admitting();
    assert!(has_start(&f.handle(0, TalkInput::LatchToggle, &mut env)));
    assert!(f.latched());
    // A hold press/release while latched keeps talking.
    assert!(f.handle(1, TalkInput::PttDown, &mut env).is_empty());
    assert!(f.handle(2, TalkInput::PttUp, &mut env).is_empty());
    assert_eq!(f.state(), TalkState::Talking);
    let o = f.handle(3, TalkInput::LatchToggle, &mut env);
    assert_eq!(stop_of(&o), Some((StopReason::PttUp, true, true)));
    assert!(!f.latched());
}

#[test]
fn ptt_blocked_busy_repeats_and_go_ahead() {
    let mut f = TalkFsm::new(TalkMode::Ptt, 0);
    let mut env = FakeEnv::refusing();
    let o = f.handle(0, TalkInput::PttDown, &mut env);
    assert!(has_earcon(&o, Earcon::Busy) && o.contains(|x| *x == TalkOutput::TalkDenied));
    assert_eq!(f.state(), TalkState::Blocked);
    let mut busy_at = Vec::new();
    for i in 1..=500u64 {
        let now = i * BLOCK_US;
        if has_earcon(
            &f.handle(now, TalkInput::Block { vad_p: 0.0 }, &mut env),
            Earcon::Busy,
        ) {
            busy_at.push(now);
        }
    }
    assert_eq!(
        busy_at,
        vec![PTT_BUSY_REPEAT_US, 2 * PTT_BUSY_REPEAT_US],
        "every 2 s while held"
    );
    let o = f.handle(5_000_000, TalkInput::SlotFreed, &mut env);
    assert!(has_earcon(&o, Earcon::GoAhead) && has_start(&o));
    assert_eq!(f.state(), TalkState::Talking);
    // Blocked + release → Idle.
    let mut f = TalkFsm::new(TalkMode::Ptt, 0);
    f.handle(0, TalkInput::PttDown, &mut env);
    assert!(f.handle(10, TalkInput::PttUp, &mut env).is_empty());
    assert_eq!(f.state(), TalkState::Idle);
    assert!(f.handle(20, TalkInput::SlotFreed, &mut env).is_empty());
}

#[test]
fn vox_blocked_busy_after_600ms_then_holdoff() {
    let mut f = TalkFsm::new(TalkMode::Vox, 0);
    let mut env = FakeEnv::refusing();
    let o = f.handle(0, TalkInput::VadOn, &mut env);
    assert!(o.contains(|x| *x == TalkOutput::TalkDenied));
    assert!(!has_earcon(&o, Earcon::Busy), "VOX: no busy tone at once");
    let mut busy_at = Vec::new();
    for i in 1..=1200u64 {
        let now = i * BLOCK_US;
        if has_earcon(
            &f.handle(now, TalkInput::Block { vad_p: 0.9 }, &mut env),
            Earcon::Busy,
        ) {
            busy_at.push(now);
        }
    }
    assert_eq!(
        busy_at,
        vec![600_000, 10_600_000],
        "after 600 ms, then once per 10 s"
    );
    // Speech ends: Idle with holdoff 1.5 s + jitter (250 ms here).
    let t = 12_000_000;
    f.handle(t, TalkInput::VadOff, &mut env);
    assert_eq!(f.state(), TalkState::Idle);
    env.admit = true;
    let asked = env.admits_asked;
    assert!(
        !has_start(&f.handle(t + 100_000, TalkInput::VadOn, &mut env)),
        "holdoff"
    );
    assert_eq!(env.admits_asked, asked);
    let mut started = None;
    for i in 1..=300u64 {
        let now = t + 100_000 + i * BLOCK_US;
        if has_start(&f.handle(now, TalkInput::Block { vad_p: 0.9 }, &mut env)) {
            started = Some(now);
            break;
        }
    }
    assert_eq!(
        started,
        Some(t + VOX_HOLDOFF_US + 250_000),
        "re-evaluated when the holdoff ends"
    );
}

#[test]
fn floor_reject_stops_at_once() {
    let mut env = FakeEnv::admitting();
    for (reason, cue) in [
        (RejectReason::Preempted, Earcon::Preempted),
        (RejectReason::Cap, Earcon::Busy),
    ] {
        let mut f = TalkFsm::new(TalkMode::Ptt, 7);
        f.handle(0, TalkInput::PttDown, &mut env);
        assert!(
            f.handle(1, TalkInput::FloorReject { burst: 99, reason }, &mut env)
                .is_empty(),
            "other burst"
        );
        let o = f.handle(2, TalkInput::FloorReject { burst: 7, reason }, &mut env);
        assert_eq!(
            stop_of(&o),
            Some((StopReason::FloorReject, false, false)),
            "no flush"
        );
        assert!(has_earcon(&o, cue));
        assert_eq!(
            o.iter()
                .filter(|x| matches!(x, TalkOutput::Earcon(_)))
                .count(),
            1
        );
        assert_eq!(f.state(), TalkState::Blocked);
        // Slot frees while PTT is still held: new burst id.
        let o = f.handle(3, TalkInput::SlotFreed, &mut env);
        assert!(o.contains(|x| *x
            == TalkOutput::StartBurst {
                burst: 8,
                mode: TalkMode::Ptt
            }));
    }
}

#[test]
fn max_burst_ends_with_a_tone_and_needs_a_fresh_press() {
    let mut env = FakeEnv::admitting();
    let mut f = TalkFsm::new(TalkMode::Ptt, 0);
    f.handle(0, TalkInput::LatchToggle, &mut env);
    let o = f.handle(
        MAX_BURST_PTT_US - 1,
        TalkInput::Block { vad_p: 0.0 },
        &mut env,
    );
    assert!(o.is_empty());
    let o = f.handle(MAX_BURST_PTT_US, TalkInput::Block { vad_p: 0.0 }, &mut env);
    assert_eq!(stop_of(&o), Some((StopReason::MaxBurst, true, true)));
    assert!(has_earcon(&o, Earcon::Busy));
    assert!(!f.latched(), "forgotten latch cleared");
    assert!(
        f.handle(
            MAX_BURST_PTT_US + 10,
            TalkInput::Block { vad_p: 0.0 },
            &mut env
        )
        .is_empty()
    );

    let mut f = TalkFsm::new(TalkMode::Vox, 0);
    f.handle(0, TalkInput::VadOn, &mut env);
    let mut stop = None;
    for i in 1..=4600u64 {
        let o = f.handle(i * BLOCK_US, TalkInput::Block { vad_p: 0.95 }, &mut env);
        if stop_of(&o).is_some() {
            stop = Some((i * BLOCK_US, stop_of(&o).unwrap().0));
            break;
        }
    }
    assert_eq!(stop, Some((MAX_BURST_VOX_US, StopReason::MaxBurst)));
}

#[test]
fn wind_hog_releases_low_probability_bursts() {
    let mut env = FakeEnv::admitting();
    let mut f = TalkFsm::new(TalkMode::Vox, 0);
    f.handle(0, TalkInput::VadOn, &mut env);
    let mut got = None;
    for i in 1..=200u64 {
        let o = f.handle(i * BLOCK_US, TalkInput::Block { vad_p: 0.3 }, &mut env);
        if let Some((r, ..)) = stop_of(&o) {
            assert!(o.contains(|x| *x == TalkOutput::RaiseVoxThreshold));
            got = Some((i * BLOCK_US, r));
            break;
        }
    }
    assert_eq!(got, Some((1_500_000, StopReason::WindHog)));
    // A genuine burst (mean p ≥ 0.5) is kept.
    let mut f = TalkFsm::new(TalkMode::Vox, 0);
    f.handle(0, TalkInput::VadOn, &mut env);
    for i in 1..=300u64 {
        assert!(
            f.handle(i * BLOCK_US, TalkInput::Block { vad_p: 0.7 }, &mut env)
                .is_empty()
        );
    }

    // Denied 3 times within 30 s → raise.
    let mut env = FakeEnv::refusing();
    let mut f = TalkFsm::new(TalkMode::Vox, 0);
    let mut raised = Vec::new();
    for k in 0..3u64 {
        let t = k * 5_000_000;
        let o = f.handle(t, TalkInput::VadOn, &mut env);
        if o.contains(|x| *x == TalkOutput::RaiseVoxThreshold) {
            raised.push(k);
        }
        f.handle(t + 100_000, TalkInput::VadOff, &mut env);
    }
    assert_eq!(raised, vec![2]);
}

#[test]
fn mutes_and_mode_changes() {
    let mut env = FakeEnv::admitting();
    let mut f = TalkFsm::new(TalkMode::Ptt, 0);
    f.handle(0, TalkInput::PttDown, &mut env);
    let o = f.handle(1, TalkInput::HardMute(true), &mut env);
    assert_eq!(stop_of(&o).map(|s| s.0), Some(StopReason::Muted));
    assert_eq!(f.state(), TalkState::Muted);
    assert!(f.handle(2, TalkInput::PttUp, &mut env).is_empty());
    assert!(
        f.handle(3, TalkInput::PttDown, &mut env).is_empty(),
        "nothing sent while muted"
    );
    // Unmute while PTT is held: talks again.
    assert!(has_start(&f.handle(
        4,
        TalkInput::HardMute(false),
        &mut env
    )));
    // Built-in-mic mute (moto, no headset) also mutes.
    f.handle(5, TalkInput::BuiltinMicMute(true), &mut env);
    assert_eq!(f.state(), TalkState::Muted);
    f.handle(6, TalkInput::BuiltinMicMute(false), &mut env);
    // Mode change while talking stops the burst.
    let o = f.handle(7, TalkInput::SetMode(TalkMode::Vox), &mut env);
    assert_eq!(stop_of(&o).map(|s| s.0), Some(StopReason::ModeChange));
    f.handle(8, TalkInput::SetMode(TalkMode::Muted), &mut env);
    assert_eq!(f.state(), TalkState::Muted);
    assert_eq!(TalkFsm::new(TalkMode::Muted, 0).state(), TalkState::Muted);
}

#[test]
fn burst_ids_wrap() {
    let mut env = FakeEnv::admitting();
    let mut f = TalkFsm::new(TalkMode::Ptt, 255);
    let o = f.handle(0, TalkInput::PttDown, &mut env);
    assert!(o.contains(|x| *x
        == TalkOutput::StartBurst {
            burst: 255,
            mode: TalkMode::Ptt
        }));
    f.handle(1, TalkInput::PttUp, &mut env);
    let o = f.handle(2, TalkInput::PttDown, &mut env);
    assert!(o.contains(|x| *x
        == TalkOutput::StartBurst {
            burst: 0,
            mode: TalkMode::Ptt
        }));
}

fn arb_input() -> impl Strategy<Value = TalkInput> {
    prop_oneof![
        Just(TalkInput::VadOn),
        Just(TalkInput::VadOff),
        Just(TalkInput::PttDown),
        Just(TalkInput::PttUp),
        Just(TalkInput::LatchToggle),
        (0u8..4).prop_map(|b| TalkInput::FloorReject {
            burst: b,
            reason: RejectReason::Cap
        }),
        Just(TalkInput::SlotFreed),
        prop_oneof![
            Just(TalkMode::Vox),
            Just(TalkMode::Ptt),
            Just(TalkMode::Muted)
        ]
        .prop_map(TalkInput::SetMode),
        any::<bool>().prop_map(TalkInput::HardMute),
        any::<bool>().prop_map(TalkInput::BuiltinMicMute),
        (0f32..1.0).prop_map(|p| TalkInput::Block { vad_p: p }),
    ]
}

proptest! {
    /// Start/stop alternate, the burst is set exactly while talking, never talking muted,
    /// and the same inputs give the same outputs (pure).
    #[test]
    fn fsm_invariants(steps in proptest::collection::vec((arb_input(), 0u64..3_000_000, any::<bool>()), 1..200)) {
        let run = || {
            let mut f = TalkFsm::new(TalkMode::Vox, 0);
            let mut env = FakeEnv::admitting();
            let mut now = 0u64;
            let mut talking = false;
            let mut log = Vec::new();
            for (input, dt, admit) in &steps {
                now += dt;
                env.admit = *admit;
                let o = f.handle(now, *input, &mut env);
                for x in o.iter() {
                    match x {
                        TalkOutput::StartBurst { .. } => { assert!(!talking, "start while talking"); talking = true; }
                        TalkOutput::StopBurst { .. } => { assert!(talking, "stop while idle"); talking = false; }
                        _ => {}
                    }
                }
                assert_eq!(talking, f.state() == TalkState::Talking);
                assert_eq!(f.current_burst().is_some(), talking);
                log.push(o);
            }
            log
        };
        prop_assert_eq!(run(), run());
    }
}
