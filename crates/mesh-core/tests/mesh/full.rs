//! Full stack: real groups, Noise XXpsk1 links and IDPROOF (the WP-10B modules) with the
//! mesh glue, over the link-layer harness of `tests/links`. A chain leader – member –
//! member where the ends have no direct link: the far member's JOIN_REQ and PRESENCE
//! reach the leader through the relay, and PTT voice crosses it and plays out.

use mesh_api::{MemberStatus, RideOp, TalkMode, UiCommand};
use mesh_core::effects::DiscoveryMode;
use mesh_core::links::{CandidateClass, PeerKey};
use mesh_core::{BLOCK_SAMPLES, LinkDownReason, Platform, rt_flags};

use crate::harness::World;
use crate::support::*;

fn tone(t_us: u64) -> [i16; BLOCK_SAMPLES] {
    let mut b = [0i16; BLOCK_SAMPLES];
    for (k, s) in b.iter_mut().enumerate() {
        let t = (t_us as f64 + k as f64 * 62.5) / 1e6;
        *s = (8_000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i16;
    }
    b
}

fn status_of(w: &World, i: usize, node: u16) -> Option<(MemberStatus, u8)> {
    snapshot(w, i)?
        .members
        .into_iter()
        .find(|m| m.node_id == node)
        .map(|m| (m.status, m.hops))
}

#[test]
fn a_chain_of_real_secure_links_relays_control_and_voice() {
    let mut w = World::new(3, Platform::Ios, 71);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Ben");
    join(&mut w, 2, &url, "Cid");
    for i in 0..3 {
        start(&mut w, i, &g);
    }
    w.connect(0, 1);
    w.connect(1, 2);
    w.run_ms(8_000);
    let (a, c) = (node_id(&w, 0, &g), node_id(&w, 2, &g));
    assert!(
        roster(&w, 0, &g).contains(&c),
        "Cid's JOIN_REQ reached the leader via Ben"
    );
    assert_eq!(up_peers(&w, 0).len(), 1, "no direct link between the ends");
    // PRESENCE floods both ways through the relay.
    let far = status_of(&w, 0, c).expect("leader lists Cid");
    assert_eq!(far.0, MemberStatus::Relayed, "leader sees Cid: {far:?}");
    let back = status_of(&w, 2, a).expect("Cid lists the leader");
    assert_eq!(
        back.0,
        MemberStatus::Relayed,
        "Cid sees the leader: {back:?}"
    );

    // Ride started by the leader; everyone goes live in PTT.
    assert!(
        w.command(0, UiCommand::RideControl { op: RideOp::Start })
            .is_ok()
    );
    w.run_ms(500);
    for i in 0..3 {
        assert!(w.command(i, UiCommand::GoLive {}).is_ok(), "goLive {i}");
        let r = w.command(
            i,
            UiCommand::SetTalkMode {
                mode: TalkMode::Ptt,
            },
        );
        assert!(r.is_ok(), "setTalkMode {i}: {r:?}");
    }
    w.run_ms(200);
    w.nodes[0].engine.set_rt_flags(rt_flags::PTT_DOWN);
    w.collect(0);
    let mut loud = 0;
    for _ in 0..200 {
        let now = w.now;
        let block = tone(now.0);
        w.nodes[0].engine.push_capture(now, &block);
        w.collect(0);
        for i in 1..3 {
            let mut out = [0i16; BLOCK_SAMPLES];
            w.nodes[i].engine.push_capture(now, &[0; BLOCK_SAMPLES]);
            w.nodes[i].engine.pull_playout(now, &mut out);
            w.collect(i);
            if i == 2 && out.iter().any(|s| s.unsigned_abs() > 1_000) {
                loud += 1;
            }
        }
        w.run_ms(10);
    }
    assert!(
        loud > 50,
        "Cid played {loud} loud blocks of the leader's voice two hops away"
    );
}

/// The §5.7 class the leader's radio policy gives node `i`'s current advertising token.
fn class_at_leader(w: &World, i: usize) -> CandidateClass {
    let token = w.nodes[i].engine.ctx().radio.token();
    w.nodes[0]
        .engine
        .ctx()
        .radio
        .class_of(PeerKey::Token(token))
}

#[test]
fn an_unreachable_member_keeps_discovery_forming_and_its_token_connects_first() {
    // Leader Ana links Ben and Cid (the BLE target of 2); Dee hangs off Ben.
    let mut w = World::new(4, Platform::Ios, 73);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    for (i, name) in [(1, "Ben"), (2, "Cid"), (3, "Dee")] {
        join(&mut w, i, &url, name);
    }
    for i in 0..4 {
        start(&mut w, i, &g);
    }
    w.connect(0, 1);
    w.connect(0, 2);
    let l13 = w.connect(1, 3);
    w.run_ms(10_000);
    let dee = node_id(&w, 3, &g);
    assert_eq!(roster(&w, 0, &g).len(), 4, "everyone joined");
    assert_eq!(up_peers(&w, 0).len(), 2);
    // Everyone is reachable and the leader is at its link target: Stable, and Dee's
    // token ranks with the reachable members (an extra path only).
    assert_eq!(w.nodes[0].discovery.0, DiscoveryMode::Stable);
    assert_eq!(class_at_leader(&w, 3), CandidateClass::ReachableMember);
    assert_eq!(class_at_leader(&w, 1), CandidateClass::ReachableMember);
    // Dee's only link goes: once she is unreachable (6 s), the leader is Forming again
    // although it still has its 2 links, and her token connects first.
    w.drop_link(l13, LinkDownReason::Timeout);
    w.run_ms(9_000);
    assert_eq!(up_peers(&w, 0).len(), 2);
    assert!(status_of(&w, 0, dee).is_none_or(|(s, _)| s != MemberStatus::Relayed));
    assert_eq!(
        w.nodes[0].discovery.0,
        DiscoveryMode::Forming,
        "a member is missing, even at the link target"
    );
    assert_eq!(class_at_leader(&w, 3), CandidateClass::UnreachableMember);
    assert_eq!(class_at_leader(&w, 2), CandidateClass::ReachableMember);
    assert_eq!(
        w.nodes[0]
            .engine
            .ctx()
            .radio
            .class_of(PeerKey::Token([0xAB; 4])),
        CandidateClass::Unknown,
        "a token no member shows"
    );
    // She is back through Ben: Stable again.
    w.connect(1, 3);
    w.run_ms(8_000);
    assert_eq!(w.nodes[0].discovery.0, DiscoveryMode::Stable);
    assert_eq!(class_at_leader(&w, 3), CandidateClass::ReachableMember);
}
