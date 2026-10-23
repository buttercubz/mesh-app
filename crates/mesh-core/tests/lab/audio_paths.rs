//! Lab audio paths (DESIGN §3.2, §12.1), property-tested over random Lab command
//! sequences: `audio: off` emits no AudioCommand, `renderOnly` emits `OpenSession`
//! without `StartCapture`, `full` emits `OpenSession` + `StartCapture`; capture starts
//! only while handling such a `labStart`; followers never touch audio.

use mesh_api::{CommandResult, LabAnchor, LabAudio, LabBench, UiCommand};
use mesh_core::effects::{AudioCommand, Effect};
use mesh_core::{Platform, PlatformEvent, RadioAvail};
use proptest::prelude::*;

use super::harness::World;
use super::{params, sample};

#[derive(Debug, Clone)]
enum Op {
    Start {
        bench: u8,
        audio: u8,
        anchor: u8,
        secs: u8,
    },
    Stop,
    Mark,
    Advance(u16),
    Snapshot,
    Platform,
    Radio(bool),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0u8..6, 0u8..3, 0u8..3, 1u8..5).prop_map(|(bench, audio, anchor, secs)| Op::Start {
            bench,
            audio,
            anchor,
            secs
        }),
        2 => Just(Op::Stop),
        1 => Just(Op::Mark),
        4 => (50u16..2_500).prop_map(Op::Advance),
        1 => Just(Op::Snapshot),
        2 => Just(Op::Platform),
        1 => any::<bool>().prop_map(Op::Radio),
    ]
}

const BENCHES: [LabBench; 6] = [
    LabBench::T0,
    LabBench::T1,
    LabBench::T2,
    LabBench::T3,
    LabBench::T6,
    LabBench::T5a,
];

fn audio_of(e: &Effect) -> Option<AudioCommand> {
    match e {
        Effect::Audio(a) => Some(*a),
        _ => None,
    }
}

fn check(ops: &[Op]) -> usize {
    let mut w = World::new(2, Platform::Ios, 30);
    w.start_lab(5150);
    w.connect(0, 1);
    w.run_ms(300);
    let mut expected_capture_starts = 0;
    let mut started = 0;
    for op in ops {
        let start_len = w.nodes[0].effects.len();
        match op {
            Op::Start {
                bench,
                audio,
                anchor,
                secs,
            } => {
                let audio = LabAudio::ALL[usize::from(*audio)];
                let p = params(
                    audio,
                    LabAnchor::ALL[usize::from(*anchor)],
                    u32::from(*secs),
                );
                let r = w.command(
                    0,
                    UiCommand::LabStart {
                        bench: BENCHES[usize::from(*bench)],
                        params: p,
                    },
                );
                let cmds: Vec<AudioCommand> = w.nodes[0].effects[start_len..]
                    .iter()
                    .filter_map(audio_of)
                    .collect();
                match (&r, audio) {
                    (CommandResult::LabRun(_), LabAudio::Off) => assert!(cmds.is_empty()),
                    (CommandResult::LabRun(_), LabAudio::RenderOnly) => {
                        assert_eq!(cmds.len(), 1, "{cmds:?}");
                        assert!(
                            matches!(cmds[0], AudioCommand::OpenSession { profile } if !profile.capture)
                        );
                    }
                    (CommandResult::LabRun(_), LabAudio::Full) => {
                        assert_eq!(cmds.len(), 2, "{cmds:?}");
                        assert!(
                            matches!(cmds[0], AudioCommand::OpenSession { profile } if profile.capture)
                        );
                        assert_eq!(cmds[1], AudioCommand::StartCapture);
                        expected_capture_starts += 1;
                    }
                    _ => assert!(cmds.is_empty(), "refused start emitted {cmds:?}"),
                }
                if r.is_ok() {
                    started += 1;
                }
            }
            Op::Stop => {
                w.command(0, UiCommand::LabStop {});
                let cmds: Vec<AudioCommand> = w.nodes[0].effects[start_len..]
                    .iter()
                    .filter_map(audio_of)
                    .collect();
                assert!(
                    cmds.iter().all(|c| matches!(
                        c,
                        AudioCommand::StopCapture | AudioCommand::CloseSession
                    ))
                );
            }
            Op::Mark => {
                w.command(
                    0,
                    UiCommand::LabMark {
                        label: "stop".into(),
                    },
                );
            }
            Op::Advance(ms) => w.run_ms(u64::from(*ms)),
            Op::Snapshot => {
                w.command(0, UiCommand::RequestSnapshot {});
            }
            Op::Platform => {
                let s = sample(&w, true);
                w.event(0, PlatformEvent::Platform(s));
            }
            Op::Radio(on) => w.event(
                0,
                PlatformEvent::RadioState {
                    ble: if *on { RadioAvail::On } else { RadioAvail::Off },
                    aware: RadioAvail::Unsupported,
                },
            ),
        }
        // Nothing but labStart may ever start capture or open a session.
        if !matches!(op, Op::Start { .. }) {
            assert!(w.nodes[0].effects[start_len..].iter().all(|e| !matches!(
                e,
                Effect::Audio(AudioCommand::StartCapture | AudioCommand::OpenSession { .. })
            )));
        }
    }
    let now = w.now;
    w.nodes[0].engine.stop(now);
    w.collect(0);
    let all: Vec<AudioCommand> = w.nodes[0].effects.iter().filter_map(audio_of).collect();
    let count = |f: fn(&AudioCommand) -> bool| all.iter().filter(|c| f(c)).count();
    assert_eq!(
        count(|c| *c == AudioCommand::StartCapture),
        expected_capture_starts
    );
    assert_eq!(
        count(|c| matches!(c, AudioCommand::OpenSession { .. })),
        count(|c| *c == AudioCommand::CloseSession),
        "every session is closed: {all:?}"
    );
    assert_eq!(
        count(|c| *c == AudioCommand::StartCapture),
        count(|c| *c == AudioCommand::StopCapture)
    );
    assert!(
        w.nodes[1].effects.iter().all(|e| audio_of(e).is_none()),
        "a follower never emits audio commands"
    );
    started
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        // Regressions are kept as explicit tests below, not as files outside tests/lab/.
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn lab_audio_commands_follow_the_audio_param(ops in prop::collection::vec(op(), 1..14)) {
        check(&ops);
    }
}

/// Regression found by the property test: a T5a run without audio must not spin on
/// its click-train deadline (no playout drives it).
#[test]
fn t5a_without_audio_does_not_spin() {
    check(&[
        Op::Start {
            bench: 5,
            audio: 0,
            anchor: 0,
            secs: 1,
        },
        Op::Advance(2_000),
    ]);
}

#[test]
fn each_audio_path_once() {
    for audio in 0..3 {
        let started = check(&[
            Op::Start {
                bench: 1,
                audio,
                anchor: 1,
                secs: 2,
            },
            Op::Advance(2_500),
        ]);
        assert_eq!(started, 1, "audio path {audio}");
    }
}
