//! Capture gating (DESIGN §3.2, normative): over random command/event sequences across
//! Idle, group-session and Lab modes, `AudioCommand::StartCapture` is emitted only while
//! handling `goLive`, `resumeMic`, `labStart{audio: full}` or `audioCheck{start}`, and
//! in Idle only for `audioCheck{start}`.

use mesh_api::{
    AudioCheckMode, AudioCheckOp, LabAnchor, LabAudio, LabBench, LabParams, RideOp, TalkMode,
    UiCommand,
};
use mesh_core::effects::{AudioCommand, Effect};
use mesh_core::{
    AudioIoState, EngineMode, InterruptReason, Platform, PlatformEvent, PlatformSample, RadioAvail,
    SessionStart,
};
use proptest::prelude::*;

use crate::harness::World;
use crate::support::{create, id_bytes, invite_url, join};

#[derive(Debug, Clone)]
enum Op {
    Cmd(u8, u8),
    Io(u8),
    Radio(bool),
    Sample,
    StartGroup,
    StartLab,
    Stop,
    Advance(u16),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (0u8..12, 0u8..3).prop_map(|(c, a)| Op::Cmd(c, a)),
        3 => (0u8..7).prop_map(Op::Io),
        1 => any::<bool>().prop_map(Op::Radio),
        1 => Just(Op::Sample),
        2 => Just(Op::StartGroup),
        1 => Just(Op::StartLab),
        1 => Just(Op::Stop),
        3 => (10u16..3_000).prop_map(Op::Advance),
    ]
}

fn command(c: u8, a: u8) -> UiCommand {
    match c {
        0 => UiCommand::GoLive {},
        1 => UiCommand::ResumeMic {},
        2 => UiCommand::AudioCheck(AudioCheckOp::Start {
            mode: AudioCheckMode::ALL[usize::from(a)],
        }),
        3 => UiCommand::AudioCheck(AudioCheckOp::Stop {}),
        4 => UiCommand::SetTalkMode {
            mode: TalkMode::ALL[usize::from(a)],
        },
        5 => UiCommand::RideControl {
            op: if a == 0 { RideOp::End } else { RideOp::Start },
        },
        6 => {
            let mut p = LabParams::new(LabAudio::ALL[usize::from(a)], LabAnchor::Audio, false);
            p.duration_s = Some(2);
            UiCommand::LabStart {
                bench: LabBench::T0,
                params: p,
            }
        }
        7 => UiCommand::LabStop {},
        8 => UiCommand::SetShareDistance { enabled: a == 1 },
        9 => UiCommand::LockGroup {},
        10 => UiCommand::RequestSnapshot {},
        _ => UiCommand::SetRoutePreference {
            pref: mesh_api::RoutePreference::ALL[usize::from(a)],
        },
    }
}

fn io_state(k: u8) -> AudioIoState {
    match k {
        0 => AudioIoState::Running,
        1 => AudioIoState::Interrupted {
            reason: InterruptReason::Call,
        },
        2 => AudioIoState::Restarting,
        3 => AudioIoState::CaptureLost,
        4 => AudioIoState::RenderLost,
        5 => AudioIoState::Restored,
        _ => AudioIoState::Stopped,
    }
}

fn may_start(cmd: &UiCommand, mode: EngineMode) -> bool {
    match cmd {
        UiCommand::AudioCheck(AudioCheckOp::Start { .. }) => true,
        _ if mode == EngineMode::Idle => false,
        UiCommand::GoLive {} | UiCommand::ResumeMic {} => true,
        UiCommand::LabStart { params, .. } => params.audio == LabAudio::Full,
        _ => false,
    }
}

fn starts(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Audio(AudioCommand::StartCapture)))
        .count()
}

fn run(ops: &[Op]) {
    let mut w = World::new(2, Platform::Ios, 40);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Ben");
    let gid = id_bytes(&g);
    for op in ops {
        let mode = w.nodes[0].engine.mode();
        let from = w.nodes[0].effects.len();
        let mut allowed = false;
        match op {
            Op::Cmd(c, a) => {
                let cmd = command(*c, *a);
                allowed = may_start(&cmd, mode);
                w.command(0, cmd);
            }
            Op::Io(k) => w.event(0, PlatformEvent::AudioIo(io_state(*k))),
            Op::Radio(on) => w.event(
                0,
                PlatformEvent::RadioState {
                    ble: if *on { RadioAvail::On } else { RadioAvail::Off },
                    aware: RadioAvail::Unsupported,
                },
            ),
            Op::Sample => w.event(0, PlatformEvent::Platform(PlatformSample::default())),
            Op::StartGroup => {
                let now = w.now;
                let _ = w.nodes[0]
                    .engine
                    .start(now, SessionStart::Group { group_id: gid });
                w.collect(0);
                if w.links.is_empty() {
                    let _ = w.nodes[1]
                        .engine
                        .start(now, SessionStart::Group { group_id: gid });
                    w.collect(1);
                    w.connect(1, 0);
                }
            }
            Op::StartLab => {
                let now = w.now;
                let _ = w.nodes[0]
                    .engine
                    .start(now, SessionStart::Lab { lab_code: 4321 });
                w.collect(0);
            }
            Op::Stop => {
                let now = w.now;
                w.nodes[0].engine.stop(now);
                w.collect(0);
            }
            Op::Advance(ms) => w.run_ms(u64::from(*ms)),
        }
        let n = starts(&w.nodes[0].effects[from..]);
        assert!(
            n == 0 || allowed,
            "StartCapture while handling {op:?} in {mode:?}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn start_capture_only_from_the_four_gesture_commands(ops in proptest::collection::vec(op(), 1..40)) {
        run(&ops);
    }
}

#[test]
fn go_live_and_resume_mic_emit_capture_in_a_session() {
    run(&[
        Op::StartGroup,
        Op::Cmd(0, 0),
        Op::Io(3),
        Op::Cmd(1, 0),
        Op::Io(0),
        Op::Advance(2_000),
        Op::Stop,
        Op::Cmd(0, 0),
        Op::Cmd(2, 1),
    ]);
}
