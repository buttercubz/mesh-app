//! Ride session (WP-10B, DESIGN §3.2, §5.8, §7.8): capture gating (property test over
//! random command/event sequences across session and Lab modes), the L0-L3 ladder in
//! PRESENCE and snapshot, Bluetooth radio state (notice, cue, PRESENCE bit, relink),
//! the audio-check lifecycle, signed RIDE_CTL, member reachability notices and
//! POSITION-based distances.

#[path = "../links/harness.rs"]
mod harness;
#[path = "../secure/support.rs"]
mod support;

mod gating;
mod probe;
mod seam;

use mesh_api::{
    AudioCheckMode, AudioCheckOp, ErrorCode, LadderLevel, MemberFlag, MemberStatus, NoticeKey,
    RideOp, RidePhase, UiCommand,
};
use mesh_core::effects::{AudioCommand, AudioSessionKind};
use mesh_core::media::Cue;
use mesh_core::{
    AudioIoState, GpsFix, InterruptReason, PlatformEvent, PlatformSample, PromptKey, RadioAvail,
};
use mesh_voice::Earcon;
use support::*;

fn io(w: &mut harness::World, i: usize, s: AudioIoState) {
    w.event(i, PlatformEvent::AudioIo(s));
}

fn member(w: &harness::World, i: usize, node: u16) -> Option<mesh_api::Member> {
    snapshot(w, i)?
        .members
        .into_iter()
        .find(|m| m.node_id == node)
}

#[test]
fn ladder_transitions_show_in_presence_and_snapshot() {
    let (mut w, g) = pair(21);
    let log = probe::attach(&mut w, 0);
    let b = node_id(&w, 1, &g);
    assert!(w.command(1, UiCommand::GoLive {}).is_ok());
    let steps = [
        (AudioIoState::Running, LadderLevel::L0, 0u8),
        (AudioIoState::Restarting, LadderLevel::L1, 1),
        (AudioIoState::CaptureLost, LadderLevel::L2, 2),
        (AudioIoState::Running, LadderLevel::L0, 0),
    ];
    for (state, level, bits) in steps {
        io(&mut w, 1, state);
        w.run_ms(2_200);
        let s = snapshot(&w, 1).expect("snapshot").session.expect("session");
        assert_eq!(s.ladder, level, "{state:?}");
        let last = log
            .borrow()
            .iter()
            .rev()
            .find(|(o, _)| *o == b)
            .map(|(_, p)| p.audio.ladder());
        assert_eq!(last, Some(bits), "PRESENCE ladder after {state:?}");
        let m = member(&w, 0, b).expect("member");
        assert_eq!(m.flags.contains(&MemberFlag::MicLost), bits >= 2);
    }
    let a = audio(&w, 1);
    assert!(a.contains(&AudioCommand::OutputOnlyFallback(true)));
    assert!(a.contains(&AudioCommand::OutputOnlyFallback(false)));
    assert!(notices(&w, 1).contains(&NoticeKey::CaptureLost));
    assert!(notices(&w, 1).contains(&NoticeKey::CaptureRestored));
    // A call interrupts: ON_CALL flag at the peer.
    io(
        &mut w,
        1,
        AudioIoState::Interrupted {
            reason: InterruptReason::Call,
        },
    );
    w.run_ms(2_200);
    assert!(
        member(&w, 0, b)
            .unwrap()
            .flags
            .contains(&MemberFlag::OnCall)
    );
    assert!(notices(&w, 1).contains(&NoticeKey::OnCall));
}

#[test]
fn radio_off_gives_notice_cue_presence_bit_and_relinks() {
    let (mut w, g) = pair(22);
    w.in_range = vec![(0, 1)];
    let b = node_id(&w, 1, &g);
    w.event(
        1,
        PlatformEvent::RadioState {
            ble: RadioAvail::Off,
            aware: RadioAvail::Unsupported,
        },
    );
    assert!(notices(&w, 1).contains(&NoticeKey::BluetoothOff));
    let radio_off = Cue::with_prompt(Some(Earcon::BluetoothOff), PromptKey::BluetoothOff);
    assert!(cues(&w, 1).contains(&radio_off));
    assert!(w.nodes[1].effects.iter().any(|e| matches!(
        e,
        mesh_core::effects::Effect::Notify(n) if n.key == NoticeKey::BluetoothOff
    )));
    assert!(up_peers(&w, 1).is_empty(), "BLE links are down");
    // Still off a minute later: the cue repeats.
    w.run_ms(61_000);
    assert_eq!(cues(&w, 1).iter().filter(|c| **c == radio_off).count(), 2);
    w.event(
        1,
        PlatformEvent::RadioState {
            ble: RadioAvail::On,
            aware: RadioAvail::Unsupported,
        },
    );
    for _ in 0..20 {
        w.advertise();
        w.run_ms(500);
        if !up_peers(&w, 1).is_empty() {
            break;
        }
    }
    assert_eq!(
        up_peers(&w, 1),
        vec![node_id(&w, 0, &g)],
        "recovery relinks"
    );
    w.run_ms(2_500);
    let m = member(&w, 0, b).expect("member");
    assert!(
        m.flags.contains(&MemberFlag::RadioOff),
        "radio_off_recently seen"
    );
    // The bit clears 60 s after the radio came back.
    w.run_ms(62_000);
    assert!(
        !member(&w, 0, b)
            .unwrap()
            .flags
            .contains(&MemberFlag::RadioOff)
    );
}

#[test]
fn audio_check_lifecycle() {
    let mut w = harness::World::new(1, mesh_core::Platform::Ios, 23);
    let start = UiCommand::AudioCheck(AudioCheckOp::Start {
        mode: AudioCheckMode::Tone,
    });
    // Without a session (Idle).
    assert!(w.command(0, start.clone()).is_ok());
    let a = audio(&w, 0);
    assert!(
        matches!(a[0], AudioCommand::OpenSession { profile } if profile.kind == AudioSessionKind::Check && profile.capture)
    );
    assert_eq!(a[1], AudioCommand::StartCapture);
    assert!(
        w.command(0, UiCommand::AudioCheck(AudioCheckOp::Stop {}))
            .is_ok()
    );
    assert_eq!(
        &audio(&w, 0)[2..],
        &[AudioCommand::StopCapture, AudioCommand::CloseSession]
    );
    // In the lobby; Go live takes the audio over; refused while live.
    let g = create(&mut w, 0, "Ana");
    start_session(&mut w, &g);
    assert!(w.command(0, start.clone()).is_ok());
    assert!(w.command(0, UiCommand::GoLive {}).is_ok());
    let a = audio(&w, 0);
    let n = a.len();
    // The ride session replaces the temporary check session.
    assert!(
        matches!(a[n - 2], AudioCommand::OpenSession { profile } if profile.kind == AudioSessionKind::Ride)
    );
    assert_eq!(a[n - 1], AudioCommand::StartCapture);
    // Refused while live, without any audio command.
    assert_eq!(w.command(0, start).error_code(), Some(ErrorCode::WrongMode));
    assert_eq!(audio(&w, 0).len(), n);
    assert_eq!(
        w.command(0, UiCommand::GoLive {}).error_code(),
        Some(ErrorCode::AlreadyLive)
    );
}

fn start_session(w: &mut harness::World, g: &str) {
    start(w, 0, g);
}

#[test]
fn ride_control_is_signed_and_scoped() {
    let (mut w, _g) = pair(24);
    // A member's Start is local; End is the leader's.
    assert_eq!(
        w.command(1, UiCommand::RideControl { op: RideOp::End })
            .error_code(),
        Some(ErrorCode::NotLeader)
    );
    assert!(
        w.command(0, UiCommand::RideControl { op: RideOp::Start })
            .is_ok()
    );
    w.run_ms(500);
    let s = snapshot(&w, 1).unwrap().session.unwrap();
    assert_eq!(s.ride, RidePhase::Riding);
    assert!(notices(&w, 1).contains(&NoticeKey::RideStarted));
    assert!(cues(&w, 1).contains(&Cue::with_prompt(
        Some(Earcon::RideStarted),
        PromptKey::RideStarted
    )));
    assert_eq!(
        s.mode,
        mesh_api::TalkMode::Vox,
        "moto default after the ride starts"
    );
    assert!(
        w.command(0, UiCommand::RideControl { op: RideOp::End })
            .is_ok()
    );
    w.run_ms(500);
    assert_eq!(
        snapshot(&w, 1).unwrap().session.unwrap().ride,
        RidePhase::Lobby
    );
    assert!(notices(&w, 1).contains(&NoticeKey::RideEnded));
}

#[test]
fn reachability_lost_back_and_distance() {
    let (mut w, g) = pair(25);
    let b = node_id(&w, 1, &g);
    for i in 0..2 {
        assert!(
            w.command(i, UiCommand::SetShareDistance { enabled: true })
                .is_ok()
        );
    }
    for k in 0..6u64 {
        for (i, lat) in [(0usize, 40.0), (1, 40.001)] {
            let s = PlatformSample {
                wall_ms: Some(1_792_746_723_120 + w.now.0 / 1_000),
                gps: Some(GpsFix {
                    lat,
                    lon: -3.0,
                    speed_mps: Some(10.0),
                    acc_m: Some(5.0),
                    heading_deg: Some(0.0),
                }),
                battery: Some(0.5),
                ..PlatformSample::default()
            };
            w.event(i, PlatformEvent::Platform(s));
        }
        w.run_ms(1_000 + k);
    }
    let m = member(&w, 0, b).expect("member");
    assert_eq!(m.status, MemberStatus::Direct);
    let d = m.distance_m.expect("distance from POSITION");
    assert!((100.0..=125.0).contains(&d), "{d}");
    assert_eq!(m.battery, Some(0.5));
    // Out of range: lost (6 s, aggregated 5 s), then back.
    w.drop_link(0, mesh_core::LinkDownReason::Timeout);
    w.run_ms(13_000);
    assert!(notices(&w, 0).contains(&NoticeKey::MemberLost));
    assert_eq!(member(&w, 0, b).unwrap().status, MemberStatus::Lost);
    w.connect(1, 0);
    w.run_ms(8_000);
    assert!(notices(&w, 0).contains(&NoticeKey::MemberBack));
    assert_eq!(member(&w, 0, b).unwrap().status, MemberStatus::Direct);
}
