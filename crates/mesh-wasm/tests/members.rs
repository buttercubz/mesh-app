//! Members dropping out and coming back in the browser simulator: a bot riding out of
//! range (`sim.botAway`), a bot's app restarting (`sim.botRestart`) and the `roam`
//! scenario option, all observed through the local phone's snapshots.

mod common;

use common::Page;
use serde_json::{Value, json};

const CLUSTER4: &str = r#"{"preset":"cluster15","nodes":4,"botsTalk":false,"seed":5}"#;

/// The local snapshot's member entry for `node_id`.
fn member_status(p: &Page, node_id: u64) -> Option<String> {
    p.snapshot()?["members"]
        .as_array()?
        .iter()
        .find(|m| m["nodeId"].as_u64() == Some(node_id))
        .and_then(|m| m["status"].as_str())
        .map(str::to_string)
}

fn reachable(s: &Value) -> u64 {
    s["mesh"]["reachable"].as_u64().unwrap_or(0)
}

fn formed(p: &mut Page) {
    p.join_bots_ride();
    assert!(
        p.run_until(30_000, |s| reachable(s) == 4),
        "ride did not form: {:?}",
        p.snapshot().map(|s| &s["mesh"])
    );
    p.go_live_ptt();
    p.run_ms(500);
}

#[test]
fn a_bot_drops_out_of_range_and_rejoins() {
    let mut p = Page::new(CLUSTER4);
    formed(&mut p);
    let ids = p.bot_ids();
    let away_id = ids[2];

    p.ok(json!({"type": "sim.botAway", "bot": 3, "away": true}));
    let info = p.ok(json!({"type": "sim.info"}));
    assert_eq!(info["bots"][2]["away"], true);
    assert!(
        p.run_until(30_000, |s| reachable(s) == 3),
        "the away bot is still reachable: {:?}",
        p.snapshot().map(|s| &s["mesh"])
    );
    let st = member_status(&p, away_id);
    assert!(
        matches!(st.as_deref(), Some("lost" | "unconfirmed")),
        "away member status {st:?}"
    );

    p.ok(json!({"type": "sim.botAway", "bot": 3, "away": false}));
    assert!(
        p.run_until(40_000, |s| reachable(s) == 4 && p_member_ok(s, away_id)),
        "the bot did not rejoin: {:?} status {:?}",
        p.snapshot().map(|s| &s["mesh"]),
        member_status(&p, away_id)
    );
    // It is back in the ride: it goes live again and the page hears it talk.
    let mut talked = false;
    for _ in 0..20 {
        if p.cmd(json!({"type": "sim.botTalk", "bot": 3, "ms": 3000}))["ok"] == true {
            talked = true;
            break;
        }
        p.run_ms(500);
    }
    assert!(talked, "the rejoined bot never went live again");
    let mut heard = false;
    for _ in 0..100 {
        p.run_ms(20);
        heard |= p.talkers().contains(&away_id);
    }
    assert!(heard, "the page does not hear the rejoined bot");
}

#[test]
fn a_bot_restart_comes_back_live() {
    let mut p = Page::new(CLUSTER4);
    formed(&mut p);
    let ids = p.bot_ids();
    p.ok(json!({"type": "sim.botRestart", "bot": 2}));
    let info = p.ok(json!({"type": "sim.info"}));
    assert_eq!(info["bots"][1]["live"], false);
    assert!(
        p.run_until(40_000, |s| reachable(s) == 4 && p_member_ok(s, ids[1])),
        "the restarted bot did not come back: {:?}",
        p.snapshot().map(|s| &s["mesh"])
    );
    let mut live = false;
    for _ in 0..30 {
        p.run_ms(200);
        let info = p.ok(json!({"type": "sim.info"}));
        if info["bots"][1]["live"] == true {
            live = true;
            break;
        }
    }
    assert!(live, "the restarted bot did not go live again");
    assert_eq!(
        p.cmd(json!({"type": "sim.botRestart", "bot": 9}))["code"],
        "unknownMember"
    );
    assert_eq!(
        p.cmd(json!({"type": "sim.botAway", "bot": 1}))["code"],
        "invalidParams"
    );
}

fn p_member_ok(s: &Value, node_id: u64) -> bool {
    s["members"].as_array().is_some_and(|a| {
        a.iter().any(|m| {
            m["nodeId"].as_u64() == Some(node_id)
                && matches!(m["status"].as_str(), Some("direct" | "relayed"))
        })
    })
}

#[test]
fn roaming_bots_drop_out_and_come_back() {
    let mut p = Page::new(
        r#"{"preset":"cluster15","nodes":4,"botsTalk":false,"seed":9,"roam":true,"roamMeanS":10}"#,
    );
    formed(&mut p);
    let (mut saw_away, mut saw_back) = (false, false);
    for _ in 0..240 {
        p.run_ms(500);
        let info = p.ok(json!({"type": "sim.info"}));
        let away = info["bots"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["away"] == true)
            .count();
        assert!(away <= 1, "more than one bot away at once");
        if away == 1 {
            saw_away = true;
        } else if saw_away {
            saw_back = true;
        }
        if saw_back {
            break;
        }
    }
    assert!(saw_away && saw_back, "away {saw_away}, back {saw_back}");
    p.ok(json!({"type": "sim.roam", "on": false}));
    assert_eq!(p.ok(json!({"type": "sim.info"}))["roam"], false);
}
