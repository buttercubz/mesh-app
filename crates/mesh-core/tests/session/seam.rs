//! The session → media seam (DESIGN §3.3, §1.3 F3/F8, §6.5, §7.7): the group identity
//! and member names reach the media module on the inter-module bus; `goLive` passes
//! through to the media module once (no request on the bus for it); the session posts
//! the gated talk mode (muted in the lobby and at L2/L3); cues carry their earcon,
//! prompt and aggregated names; Leave floods a signed LEAVE before the links close.

use mesh_api::{
    LadderLevel, NoticeKey, ParamValue, PromptKey, RideOp, TalkMode, TalkState, UiCommand, UiEvent,
};
use mesh_core::AudioIoState;
use mesh_core::effects::{AudioCommand, RadioCommand};
use mesh_core::media::{Cue, MediaIdentity, MediaRequest};
use mesh_core::{Platform, PlatformEvent};
use mesh_voice::Earcon;
use mesh_wire::frame::RejectReason;

use crate::harness::World;
use crate::support::*;

/// Requests of node `i` posted after index `from`.
fn since(w: &World, i: usize, from: usize) -> Vec<MediaRequest> {
    media_requests(w, i)[from..].to_vec()
}

fn talk_modes(reqs: &[MediaRequest]) -> Vec<TalkMode> {
    reqs.iter()
        .filter_map(|r| match r {
            MediaRequest::SetTalkMode(m) => Some(*m),
            _ => None,
        })
        .collect()
}

fn last_names(w: &World, i: usize) -> Vec<(u16, String)> {
    media_requests(w, i)
        .into_iter()
        .rev()
        .find_map(|r| match r {
            MediaRequest::MemberNames(v) => Some(v),
            _ => None,
        })
        .expect("MemberNames posted")
}

#[test]
fn identity_names_and_gated_talk_mode_reach_the_media_module() {
    let (mut w, g) = pair(31);
    let (a, b) = (node_id(&w, 0, &g), node_id(&w, 1, &g));
    // The identity is posted when the session opens, before any Go live.
    for (i, node, leader) in [(0, a, true), (1, b, false)] {
        let ids: Vec<MediaIdentity> = media_requests(&w, i)
            .into_iter()
            .filter_map(|r| match r {
                MediaRequest::Identity(x) => Some(x),
                _ => None,
            })
            .collect();
        let want = MediaIdentity {
            node,
            leader,
            epoch: 0,
            leader_priority: true,
        };
        assert_eq!(ids.first(), Some(&want), "node {i}");
        let mut names = vec![(a, "Ana".to_owned()), (b, "Ben".to_owned())];
        names.sort();
        assert_eq!(
            last_names(&w, i),
            names,
            "names after the roster (node {i})"
        );
    }
    // Go live passes through: one OpenSession + StartCapture, no GoLive on the bus, and
    // the session hands the media module the lobby's muted mode.
    let (fx0, bus0) = (w.nodes[1].effects.len(), media_requests(&w, 1).len());
    assert!(w.command(1, UiCommand::GoLive {}).is_ok());
    let starts = w.nodes[1].effects[fx0..]
        .iter()
        .filter(|e| {
            matches!(
                e,
                mesh_core::effects::Effect::Audio(AudioCommand::StartCapture)
            )
        })
        .count();
    assert_eq!(starts, 1);
    let posted = since(&w, 1, bus0);
    assert!(
        !posted
            .iter()
            .any(|r| matches!(r, MediaRequest::GoLive { .. } | MediaRequest::ResumeMic)),
        "goLive is not also posted: {posted:?}"
    );
    assert_eq!(talk_modes(&posted), [TalkMode::Muted]);
    // The user's choice is answered by the session; the lobby keeps the media muted.
    let bus1 = media_requests(&w, 1).len();
    assert!(
        w.command(
            1,
            UiCommand::SetTalkMode {
                mode: TalkMode::Ptt
            }
        )
        .is_ok()
    );
    assert!(talk_modes(&since(&w, 1, bus1)).is_empty());
    w.run_ms(200);
    let s = snapshot(&w, 1).unwrap().session.unwrap();
    assert_eq!((s.mode, s.talk), (TalkMode::Ptt, TalkState::Muted));
    // Start ride: the chosen mode goes to the media module, with the ride cue.
    assert!(
        w.command(0, UiCommand::RideControl { op: RideOp::Start })
            .is_ok()
    );
    w.run_ms(500);
    let posted = since(&w, 1, bus1);
    assert_eq!(talk_modes(&posted), [TalkMode::Ptt]);
    let ride = Cue::with_prompt(Some(Earcon::RideStarted), PromptKey::RideStarted);
    assert!(posted.contains(&MediaRequest::Cue(ride)));
    let s = snapshot(&w, 1).unwrap().session.unwrap();
    assert_eq!((s.mode, s.talk), (TalkMode::Ptt, TalkState::Idle));
    // Capture lost (L2): muted with the mic-lost cue; back (L0): PTT again.
    let bus2 = media_requests(&w, 1).len();
    w.event(1, PlatformEvent::AudioIo(AudioIoState::CaptureLost));
    w.event(1, PlatformEvent::AudioIo(AudioIoState::Running));
    let posted = since(&w, 1, bus2);
    assert_eq!(talk_modes(&posted), [TalkMode::Muted, TalkMode::Ptt]);
    let lost = Cue::with_prompt(Some(Earcon::MicLost), PromptKey::MicPaused);
    let back = Cue::with_prompt(None, PromptKey::MicBack);
    assert!(posted.contains(&MediaRequest::Cue(lost)));
    assert!(posted.contains(&MediaRequest::Cue(back)));
    w.run_ms(200);
    assert_eq!(
        snapshot(&w, 1).unwrap().session.unwrap().ladder,
        LadderLevel::L0
    );
}

#[test]
fn headset_and_battery_cues_come_from_the_session() {
    let (mut w, _g) = pair(32);
    assert!(w.command(1, UiCommand::GoLive {}).is_ok());
    let route = |headset| {
        let mut r = mesh_core::AudioRouteInfo::NONE;
        r.headset_present = headset;
        PlatformEvent::AudioRoute(r)
    };
    w.event(1, route(true));
    w.event(1, route(false));
    let sample = mesh_core::PlatformSample {
        battery: Some(0.1),
        charging: Some(false),
        ..mesh_core::PlatformSample::default()
    };
    w.event(1, PlatformEvent::Platform(sample.clone()));
    w.run_ms(1_000);
    w.event(1, PlatformEvent::Platform(sample));
    let keys = cue_keys(&w, 1);
    assert_eq!(
        keys.iter()
            .filter(|k| **k == PromptKey::HeadsetLost)
            .count(),
        1
    );
    assert_eq!(
        keys.iter().filter(|k| **k == PromptKey::BatteryLow).count(),
        1,
        "once per 10 min"
    );
}

#[test]
fn joined_members_are_announced_with_their_aggregated_names() {
    let mut w = World::new(3, Platform::Ios, 33);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Ben");
    join(&mut w, 2, &url, "Cid");
    for i in 0..3 {
        start(&mut w, i, &g);
    }
    w.connect(1, 0);
    w.connect(2, 0);
    w.run_ms(7_000);
    let joined: Vec<Cue> = cues(&w, 0)
        .into_iter()
        .filter(|c| c.prompt == Some(PromptKey::MemberJoined))
        .collect();
    assert_eq!(joined.len(), 1, "one cue for both (5 s aggregation)");
    assert_eq!(joined[0].earcon, Some(Earcon::MemberJoined));
    let names = joined[0].param.clone().unwrap();
    assert!(names == "Ben, Cid" || names == "Cid, Ben", "{names}");
}

#[test]
fn leave_floods_a_signed_leave_before_the_links_close() {
    let (mut w, g) = pair(34);
    let b = node_id(&w, 1, &g);
    w.run_ms(2_000);
    stop(&mut w, 1);
    // My side closes with CLOSE{leaving} once the LEAVE left; the peer reads both.
    assert!(w.radio_cmds(1).iter().any(|c| matches!(
        c,
        RadioCommand::Close {
            reason: Some(RejectReason::Leaving),
            ..
        }
    )));
    w.run_ms(100);
    assert!(
        up_peers(&w, 0).is_empty(),
        "closed at once, not by link death"
    );
    w.run_ms(6_000);
    let left = w.ui_events(0).into_iter().find_map(|e| match e {
        UiEvent::Notice {
            key: NoticeKey::MemberLeft,
            params,
        } => Some(params.clone()),
        _ => None,
    });
    let left = left.expect("memberLeft notice from the verified LEAVE");
    assert_eq!(left.get("names"), Some(&ParamValue::from("Ben")));
    let spoken = Cue::with_prompt(Some(Earcon::MemberLost), PromptKey::MemberLeft).param("Ben");
    assert!(cues(&w, 0).contains(&spoken));
    assert!(
        !notices(&w, 0).contains(&NoticeKey::MemberLost),
        "a rider who left is not reported lost"
    );
    assert!(!up_peers(&w, 0).contains(&b));
}
