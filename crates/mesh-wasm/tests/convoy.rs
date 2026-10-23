//! The browser simulator end to end through its JSON command surface (the same calls
//! the `mw_*` ABI forwards): joining the bots' ride, talking, the floor cap, the page's
//! own group, Lab mode, determinism and long-run memory trimming.

mod common;

use common::Page;
use serde_json::json;

/// A line local(0) - bot1 - bot2, 10 m apart, scripted bots: every link of the line
/// carries both bots' voice, so the floor cap (2 per BLE link, §6.1) is what decides.
const LINE3: &str = r#"{"preset":"pair","nodes":3,"shadow":[[0,2]],"botsTalk":false,"seed":7}"#;

#[test]
fn join_talk_and_busy_on_third_talker() {
    let mut p = Page::new(LINE3);
    let gid = p.join_bots_ride();
    // The ride forms: both bots reachable (bot2 through bot1).
    assert!(
        p.run_until(20_000, |s| s["mesh"]["reachable"] == 3
            && s["session"]["groupId"] == gid.as_str()),
        "ride did not form: {:?}",
        p.snapshot()
    );
    p.go_live_ptt();
    p.run_ms(500);
    let bots = p.bot_ids();
    assert_eq!(bots.len(), 2);

    // The local phone talks alone and is heard by both bots.
    p.ok(json!({"type": "sim.ptt", "down": true}));
    assert!(p.run_until(1_000, |s| s["session"]["talk"] == "talking"));
    let me = p.snapshot().unwrap()["session"]["myNodeId"]
        .as_u64()
        .unwrap();
    assert!(p.talkers().contains(&me));
    p.run_ms(1_000);
    for b in 1..=2 {
        let snap = p.c.world().nodes[b].snapshot.as_ref().unwrap();
        assert!(
            snap.talkers.iter().any(|t| u64::from(t.node_id) == me),
            "bot {b} does not hear the local phone"
        );
    }
    p.ok(json!({"type": "sim.ptt", "down": false}));
    assert!(p.run_until(1_500, |s| s["session"]["talk"] == "idle"
        && s["talkers"].as_array().is_some_and(|a| a.is_empty())));

    // Two bots take the floor.
    p.ok(json!({"type": "sim.botTalk", "bot": 1, "ms": 6000}));
    p.run_ms(400);
    p.ok(json!({"type": "sim.botTalk", "bot": 2, "ms": 6000}));
    assert!(
        p.run_until(1_500, |s| s["talkers"]
            .as_array()
            .is_some_and(|a| a.len() == 2)),
        "the local phone does not hear both bots: {:?}",
        p.snapshot().map(|s| &s["talkers"])
    );
    let mut both = bots.clone();
    both.sort_unstable();
    assert_eq!(p.talkers(), both);

    // The third talker is busy: BLOCKED and a talkDenied{cap}; nothing is sent.
    let denied_before = p.of_type("talkDenied").len();
    p.ok(json!({"type": "sim.ptt", "down": true}));
    assert!(
        p.run_until(500, |s| s["session"]["talk"] == "blocked"),
        "not blocked: {:?}",
        p.talk_state()
    );
    let denied = p.of_type("talkDenied");
    assert!(denied.len() > denied_before, "no talkDenied event");
    assert_eq!(denied.last().unwrap()["reason"], "cap");
    assert_eq!(p.talkers(), both, "the busy talker must not appear");
    for b in 1..=2 {
        let snap = p.c.world().nodes[b].snapshot.as_ref().unwrap();
        assert!(
            snap.talkers.iter().all(|t| u64::from(t.node_id) != me),
            "bot {b} hears the blocked local phone"
        );
    }

    // When a slot frees while PTT is still held, the local phone gets the floor.
    assert!(
        p.run_until(8_000, |s| s["session"]["talk"] == "talking"),
        "never got the floor back: {:?}",
        p.talk_state()
    );
    p.ok(json!({"type": "sim.ptt", "down": false}));
    assert!(p.run_until(1_500, |s| s["session"]["talk"] == "idle"));
}

#[test]
fn the_ui_ptt_command_also_drives_the_microphone() {
    let mut p = Page::new(LINE3);
    p.join_bots_ride();
    assert!(p.run_until(20_000, |s| s["mesh"]["reachable"] == 3));
    p.go_live_ptt();
    p.run_ms(300);
    p.ok(json!({"type": "ptt", "down": true}));
    assert!(p.run_until(1_000, |s| s["session"]["talk"] == "talking"));
    let from = p.events.len();
    p.run_ms(1_500);
    let peak = p.events[from..]
        .iter()
        .filter(|e| e["type"] == "snapshot")
        .filter_map(|e| e["s"]["audio"]["inputLevel"].as_f64())
        .fold(0.0, f64::max);
    assert!(peak > 0.0, "synthetic speech must reach the capture chain");
    let me = p.snapshot().unwrap()["session"]["myNodeId"]
        .as_u64()
        .unwrap();
    let bot = p.c.world().nodes[1].snapshot.as_ref().unwrap();
    assert!(bot.talkers.iter().any(|t| u64::from(t.node_id) == me));
    p.ok(json!({"type": "ptt", "down": false}));
    assert!(p.run_until(1_500, |s| s["session"]["talk"] == "idle"));
}

#[test]
fn bots_follow_the_pages_new_group_and_wait_for_its_ride() {
    let mut p = Page::new(r#"{"preset":"cluster15","nodes":4,"botsTalk":false,"seed":3}"#);
    p.ok(json!({"type": "setDisplayName", "name": "Me"}));
    let r = p.ok(json!({"type": "createGroup", "name": "Web ride", "vehicle": "moto"}));
    let gid = r["groupId"].as_str().unwrap().to_string();
    let info = p.ok(json!({"type": "sim.info"}));
    assert_eq!(info["botsGroupId"], gid.as_str());
    p.ok(json!({"type": "sim.startEngine", "groupId": gid}));
    assert!(
        p.run_until(30_000, |s| s["mesh"]["reachable"] == 4),
        "bots did not reach the page's group: {:?}",
        p.snapshot().map(|s| &s["mesh"])
    );
    let snap = p.snapshot().unwrap();
    assert_eq!(snap["session"]["role"], "leader");
    assert_eq!(snap["members"].as_array().unwrap().len(), 4);
    // Bots stay in the lobby until the page's leader starts the ride.
    p.run_ms(2_000);
    for b in 1..4 {
        let s = p.c.world().nodes[b].snapshot.as_ref().unwrap();
        assert_eq!(s.session.as_ref().unwrap().ride, mesh_api::RidePhase::Lobby);
    }
    p.ok(json!({"type": "goLive"}));
    p.ok(json!({"type": "rideControl", "op": "start"}));
    p.run_ms(3_000);
    for b in 1..4 {
        let s = p.c.world().nodes[b].snapshot.as_ref().unwrap();
        let sess = s.session.as_ref().unwrap();
        assert_eq!(sess.ride, mesh_api::RidePhase::Riding, "bot {b}");
        assert!(sess.live, "bot {b} not live");
    }
    // The sim invite is now the page's own group.
    let url = p.ok(json!({"type": "sim.invite"}))["url"]
        .as_str()
        .unwrap()
        .to_string();
    let r = p.cmd(json!({"type": "joinGroup", "url": url, "displayName": "Me"}));
    assert_eq!(r["code"], "alreadyMember");
}

#[test]
fn bots_talk_on_their_own_schedule() {
    let mut p = Page::new(r#"{"preset":"cluster15","nodes":4,"seed":11,"botTalkMeanS":3}"#);
    p.join_bots_ride();
    p.go_live_ptt();
    let mut heard = std::collections::BTreeSet::new();
    for _ in 0..60 {
        p.run_ms(500);
        heard.extend(p.talkers());
    }
    assert!(heard.len() >= 2, "heard only {heard:?} in 30 s");
}
