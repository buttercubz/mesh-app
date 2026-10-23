//! Lab mode against simulated peers, the Demo ride, determinism, the handle table and
//! output framing of the ABI, and memory staying flat over a long session.

mod common;

use common::Page;
use mesh_wasm::Registry;
use serde_json::{Value, json};

#[test]
fn lab_bench_runs_against_simulated_peers() {
    let mut p = Page::new(r#"{"preset":"pair","nodes":2,"botsTalk":false,"seed":5}"#);
    p.ok(json!({"type": "sim.startEngine", "lab": {"code": "4321"}}));
    // The bot follows with the same code.
    assert_eq!(p.ok(json!({"type": "sim.info"}))["botsLab"], true);
    // Wait for the peer.
    assert!(
        p.run_until(20_000, |s| s["mesh"]["links"].as_u64().unwrap_or(0) >= 1),
        "lab peer never appeared: {:?}",
        p.snapshot().map(|s| &s["mesh"])
    );
    let r = p.ok(json!({"type": "labStart", "bench": "T0",
        "params": {"audio": "off", "anchor": "audio", "gps": false, "durationS": 5}}));
    let run_id = r["runId"].as_str().expect("runId").to_string();
    p.run_ms(8_000);
    assert!(
        !p.of_type("labFinished").is_empty(),
        "the run never finished"
    );
    let lines = p.c.poll_lab();
    assert!(lines.len() > 3, "too few labrun lines: {}", lines.len());
    let first: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(first["t"], "run");
    assert_eq!(first["runId"], run_id.as_str());
    assert_eq!(first["bench"], "T0");
    for l in &lines {
        let v: Value = serde_json::from_str(l).expect("labrun line is JSON");
        assert!(v["t"].is_string());
    }
    assert!(lines.iter().any(|l| l.contains("\"t\":\"summary\"")));
    let csv = p.ok(json!({"type": "sim.labCsv", "runId": run_id}));
    let csv = csv["csv"].as_str().unwrap();
    assert_eq!(csv.lines().count(), 2, "{csv}");
    assert!(csv.lines().nth(1).unwrap().contains(&run_id), "{csv}");
    assert_eq!(
        p.cmd(json!({"type": "sim.labCsv", "runId": "r-nope"}))["code"],
        "unknownLabRun"
    );
    // Incremental: nothing new until the next run.
    assert!(p.c.poll_lab().is_empty());

    // Stopping returns the bot to its ride.
    p.ok(json!({"type": "sim.stopEngine"}));
    assert_eq!(p.ok(json!({"type": "sim.info"}))["botsLab"], false);
    assert_eq!(
        p.cmd(json!({"type": "sim.startEngine", "lab": {"code": "12345"}}))["code"],
        "invalidParams"
    );
}

#[test]
fn demo_ride_joins_the_bots() {
    let mut p = Page::new(r#"{"preset":"cluster15","nodes":4,"botsTalk":false,"seed":9}"#);
    p.ok(json!({"type": "sim.startEngine", "demo": true}));
    assert!(p.run_until(30_000, |s| s["mesh"]["reachable"] == 4));
    assert_eq!(
        p.cmd(json!({"type": "sim.startEngine", "demo": true}))["code"],
        "wrongMode"
    );
}

#[test]
fn refusals_use_contract_error_codes() {
    let mut p = Page::new(r#"{"preset":"pair","botsTalk":false}"#);
    for (cmd, code) in [
        (json!({"type": "noSuchCommand"}), "invalidCommand"),
        (json!({"type": "sim.nope"}), "invalidCommand"),
        (json!({"type": "goLive", "extra": 1}), "invalidCommand"),
        (json!({"type": "sim.ptt"}), "invalidParams"),
        (json!({"type": "sim.startEngine"}), "invalidParams"),
        (
            json!({"type": "sim.startEngine", "groupId": "00"}),
            "unknownGroup",
        ),
        (
            json!({"type": "sim.botTalk", "bot": 9, "ms": 100}),
            "unknownMember",
        ),
        (
            json!({"type": "sim.botTalk", "bot": 1, "ms": 0}),
            "invalidParams",
        ),
    ] {
        let r = p.cmd(cmd.clone());
        assert_eq!(r["ok"], false, "{cmd}");
        assert_eq!(r["code"], code, "{cmd}");
    }
    assert_eq!(
        p.c.command_json("not json"),
        r#"{"code":"invalidCommand","ok":false}"#
    );
    // A UI command the idle engine refuses keeps its own code.
    assert_eq!(p.cmd(json!({"type": "goLive"}))["ok"], false);
}

fn transcript(cfg: &str) -> Vec<String> {
    let mut p = Page::new(cfg);
    p.join_bots_ride();
    p.run_ms(5_000);
    p.go_live_ptt();
    p.ok(json!({"type": "sim.ptt", "down": true}));
    p.run_ms(1_000);
    p.ok(json!({"type": "sim.ptt", "down": false}));
    p.run_ms(10_000);
    p.events.iter().map(Value::to_string).collect()
}

#[test]
fn same_scenario_same_events() {
    let cfg = r#"{"preset":"cluster15","nodes":4,"seed":21,"botTalkMeanS":2}"#;
    let a = transcript(cfg);
    let b = transcript(cfg);
    assert!(a.len() > 20);
    assert_eq!(a, b);
    let c = transcript(r#"{"preset":"cluster15","nodes":4,"seed":22,"botTalkMeanS":2}"#);
    assert_ne!(a, c);
}

fn read_frame(b: &[u8]) -> String {
    let len = u32::from_le_bytes(b[..4].try_into().unwrap()) as usize;
    assert_eq!(b.len(), 4 + len);
    String::from_utf8(b[4..].to_vec()).unwrap()
}

#[test]
fn registry_handles_and_frames() {
    let mut r = Registry::default();
    assert_eq!(r.world_new(r#"{"preset":"ring9"}"#), 0);
    assert_eq!(r.world_new("{nope"), 0);
    let a = r.world_new(r#"{"preset":"pair","botsTalk":false}"#);
    let b = r.world_new(r#"{"preset":"pair","botsTalk":false,"seed":2}"#);
    assert_eq!((a, b), (1, 2));
    assert_eq!(r.live(), 2);

    let res = read_frame(r.command(a, r#"{"type":"sim.info"}"#).unwrap());
    let v: Value = serde_json::from_str(&res).unwrap();
    assert_eq!(v["preset"], "pair");
    r.step(a, 100_000);
    let evs: Value = serde_json::from_str(&read_frame(r.poll_events(a).unwrap())).unwrap();
    assert!(evs.is_array());
    assert_eq!(read_frame(r.poll_events(a).unwrap()), "[]");
    assert_eq!(read_frame(r.poll_lab(a).unwrap()), "");

    // Unknown handles are no-ops.
    assert!(r.command(0, "{}").is_none());
    assert!(r.command(99, "{}").is_none());
    assert!(r.poll_events(99).is_none());
    r.step(99, 1);
    r.world_free(99);

    r.world_free(a);
    assert!(r.command(a, "{}").is_none());
    assert_eq!(r.live(), 1);
    // A freed slot is reused.
    assert_eq!(r.world_new(r#"{"preset":"pair"}"#), a);
    let mut out = Vec::new();
    mesh_wasm::frame_into(&mut out, b"hi");
    assert_eq!(out, [2, 0, 0, 0, b'h', b'i']);
}

#[test]
fn default_world_and_memory_stay_flat() {
    let mut p = Page::new(r#"{"botTalkMeanS":2}"#);
    let info = p.ok(json!({"type": "sim.info"}));
    assert_eq!(info["preset"], "convoy15");
    assert_eq!(info["nodes"], 7);
    assert_eq!(info["bots"].as_array().unwrap().len(), 6);
    p.join_bots_ride();
    p.go_live_ptt();
    p.run_ms(60_000);
    let w = p.c.world();
    for n in &w.nodes {
        assert!(n.effects.is_empty(), "effect log not drained");
        assert!(n.commands.is_empty());
        assert!(n.talk_hist.len() <= 256 && n.notices.len() <= 256);
        assert!(n.audio.talk.as_ref().is_none_or(|s| s.len() <= 16));
        assert!(n.tap.borrow().frames.len() < 20_000);
    }
    assert!(p.c.dropped_events() == 0);
}
