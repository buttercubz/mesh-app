//! Bench runs between engines: T0 over a stream link, T0b MESH_PROBE relay, id
//! collisions, determinism, T5a loopback.

use mesh_api::{LabAnchor, LabAudio, LabBench, LabVerdict, UiCommand, UiEvent};
use mesh_core::effects::{AudioCommand, Effect};
use mesh_core::{BLOCK_SAMPLES, Platform};
use serde_json::Value;

use super::harness::World;
use super::{params, run_with_samples, start, validate, voice};

fn audio_cmds(w: &World, i: usize) -> Vec<AudioCommand> {
    w.nodes[i]
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Audio(a) => Some(*a),
            _ => None,
        })
        .collect()
}

fn finished(w: &World, i: usize, run_id: &str) -> Option<mesh_api::LabSummary> {
    w.ui_events(i).into_iter().find_map(|e| match e {
        UiEvent::LabFinished {
            run_id: r, summary, ..
        } if r == run_id => Some(summary.clone()),
        _ => None,
    })
}

#[test]
fn t0_voice_like_run_between_two_engines_writes_valid_labrun_files() {
    let mut w = World::new(2, Platform::Ios, 20);
    w.start_lab(1234);
    w.connect(0, 1);
    w.run_ms(1_000);
    let mut p = voice(params(LabAudio::Full, LabAnchor::Audio, 6), 2);
    p.pattern = Some(mesh_api::LabPattern::PingUnderLoad);
    p.ping_hz = Some(10);
    let run_id = start(&mut w, 0, LabBench::T0, p);
    assert!(run_id.starts_with("r-20261023-0912-"), "{run_id}");
    run_with_samples(&mut w, 9, true);
    for (i, role) in [(0, "controller"), (1, "follower")] {
        let lines = w.lab_lines(i, &run_id);
        let v = validate(&lines);
        assert!(v.len() > 8, "node {i}: {} lines", v.len());
        let run = &v[0];
        assert_eq!(run["t"], "run");
        assert_eq!(run["runId"], run_id.as_str());
        assert_eq!(run["bench"], "T0");
        assert_eq!(run["audio"], "full");
        assert_eq!(run["anchor"], "audio");
        assert_eq!(run["role"], role);
        assert_eq!(run["startedAt"].as_str().unwrap().len(), 24);
        let samples: Vec<&Value> = v.iter().filter(|l| l["t"] == "s").collect();
        assert!(samples.len() >= 5);
        assert!(samples.iter().all(|s| s["anchor"].is_object()));
        let last = v.last().unwrap();
        assert_eq!(last["t"], "summary", "node {i}");
        let s = finished(&w, i, &run_id).expect("labFinished");
        assert!(s.loss < 0.05, "loss {}", s.loss);
        assert!(s.duration_s >= 5.9);
    }
    // Two voice-like streams of 118 B every 40 ms ≈ 47 kbps + 10 Hz pings of 64 B.
    let s = finished(&w, 0, &run_id).unwrap();
    assert!((40.0..70.0).contains(&s.tx_kbps.mean), "{}", s.tx_kbps.mean);
    assert_eq!(s.verdict, LabVerdict::Go, "{s:?}");
    assert!(s.owd_ms.is_some() && s.rtt_ms.is_some());
    // Controller: OpenSession + StartCapture at start, StopCapture + CloseSession at end.
    let a = audio_cmds(&w, 0);
    assert!(matches!(a[0], AudioCommand::OpenSession { profile } if profile.capture));
    assert_eq!(a[1], AudioCommand::StartCapture);
    assert_eq!(
        &a[2..],
        &[AudioCommand::StopCapture, AudioCommand::CloseSession]
    );
    // The follower never touches audio (capture needs a local gesture).
    assert!(audio_cmds(&w, 1).is_empty());
    // Both phones list the run with its verdict.
    let runs = list(&mut w, 1);
    assert_eq!(runs[0].run_id, run_id);
    assert!(runs[0].verdict.is_some());
}

fn list(w: &mut World, i: usize) -> Vec<mesh_api::LabRunInfo> {
    match w.command(i, UiCommand::LabListRuns {}) {
        mesh_api::CommandResult::LabRuns(r) => r.runs,
        other => panic!("{other:?}"),
    }
}

#[test]
fn three_engines_relay_mesh_probe_with_trace() {
    let mut w = World::new(3, Platform::Ios, 21);
    w.start_lab(4321);
    w.connect(0, 1);
    w.connect(1, 2);
    w.run_ms(1_000);
    let ids: Vec<u16> = (0..3)
        .map(|i| w.nodes[i].engine.lab_node_id().unwrap())
        .collect();
    let run_id = start(
        &mut w,
        0,
        LabBench::T0b,
        params(LabAudio::Off, LabAnchor::Audio, 4),
    );
    run_with_samples(&mut w, 6, false);
    let c = validate(&w.lab_lines(2, &run_id));
    assert_eq!(c[0]["role"], "follower", "the start reached C through B");
    let probes: Vec<&Value> = c
        .iter()
        .filter(|l| l["t"] == "raw")
        .filter_map(|l| l.get("meshProbes"))
        .filter_map(Value::as_array)
        .flatten()
        .collect();
    assert!(probes.len() >= 10, "{} probes at C", probes.len());
    for p in &probes {
        assert_eq!(p["origin"], ids[0]);
        assert_eq!(p["hops"], 2);
        assert_eq!(p["trace"], serde_json::json!([ids[1]]));
    }
    let b = validate(&w.lab_lines(1, &run_id));
    let at_b = b
        .iter()
        .filter_map(|l| l.get("meshProbes"))
        .filter_map(Value::as_array)
        .flatten()
        .count();
    assert!(at_b >= 10);
    // A never delivers its own probes back to itself.
    let a = validate(&w.lab_lines(0, &run_id));
    assert!(a.iter().all(|l| l.get("meshProbes").is_none()));
}

#[test]
fn lab_node_id_collision_is_resolved_by_redraw() {
    let mut w = World::new(2, Platform::Ios, 22);
    w.start_lab(1000);
    w.connect(0, 1);
    w.run_ms(500);
    let b = w.nodes[1].engine.lab_node_id().unwrap();
    w.nodes[0].engine.force_lab_node_id(b);
    w.collect(0);
    w.run_ms(500);
    let (a2, b2) = (
        w.nodes[0].engine.lab_node_id().unwrap(),
        w.nodes[1].engine.lab_node_id().unwrap(),
    );
    assert_ne!(a2, b2);
    let peer_of = |i: usize| {
        w.nodes[i]
            .engine
            .ctx()
            .links
            .iter()
            .next()
            .and_then(|l| l.peer_node)
            .map(|n| n.0)
    };
    assert_eq!(peer_of(0), Some(b2));
    assert_eq!(peer_of(1), Some(a2));
}

#[test]
fn two_hop_collision_is_resolved_through_the_middle_node() {
    let mut w = World::new(3, Platform::Ios, 23);
    w.start_lab(1000);
    w.connect(0, 1);
    w.connect(1, 2);
    w.run_ms(500);
    let a = w.nodes[0].engine.lab_node_id().unwrap();
    w.nodes[2].engine.force_lab_node_id(a);
    w.collect(2);
    w.run_ms(500);
    let ids: Vec<u16> = (0..3)
        .map(|i| w.nodes[i].engine.lab_node_id().unwrap())
        .collect();
    assert_ne!(ids[0], ids[2], "{ids:?}");
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
}

#[test]
fn same_seed_same_run_bytes() {
    let go = |salt: u8| {
        let mut w = World::new(2, Platform::Android, salt);
        w.start_lab(2468);
        w.connect(0, 1);
        w.run_ms(800);
        let id = start(
            &mut w,
            0,
            LabBench::T0,
            voice(params(LabAudio::Off, LabAnchor::Audio, 3), 1),
        );
        run_with_samples(&mut w, 5, false);
        (
            id.clone(),
            w.lab_lines(0, &id),
            w.lab_lines(1, &id),
            w.nodes[0].tx_log.clone(),
        )
    };
    let x = go(40);
    let y = go(40);
    assert_eq!(x, y);
    let z = go(41);
    assert_ne!(x.0, z.0, "another seed draws another run id");
}

#[test]
fn t5a_click_train_loops_back_through_capture() {
    let mut w = World::new(1, Platform::Ios, 24);
    w.start_lab(1);
    let mut p = params(LabAudio::Full, LabAnchor::Audio, 6);
    p.interval_ms = Some(1_000);
    let run_id = start(&mut w, 0, LabBench::T5a, p);
    // 30 ms acoustic loop: what is rendered now is captured 3 blocks later.
    let mut delay: std::collections::VecDeque<[i16; BLOCK_SAMPLES]> =
        std::collections::VecDeque::from(vec![[0i16; BLOCK_SAMPLES]; 3]);
    let mut out = [0i16; BLOCK_SAMPLES];
    for step in 0..700u64 {
        let now = mesh_core::Instant(w.now.0 + step * 10_000);
        w.nodes[0].engine.pull_playout(now, &mut out);
        delay.push_back(out);
        let cap = delay.pop_front().unwrap();
        w.nodes[0].engine.push_capture(now, &cap);
        if step % 100 == 0 {
            let s = super::sample(&w, true);
            w.nodes[0]
                .engine
                .handle_event(now, mesh_core::PlatformEvent::Platform(s));
        }
        w.nodes[0].engine.handle_timeout(now);
        w.collect(0);
    }
    w.now = mesh_core::Instant(w.now.0 + 700 * 10_000);
    let v = validate(&w.lab_lines(0, &run_id));
    let loops: Vec<f64> = v
        .iter()
        .filter(|l| l["t"] == "e" && l["ev"] == "loopback")
        .filter_map(|l| l["delayMs"].as_f64())
        .collect();
    assert!(loops.len() >= 3, "{v:?}");
    for d in loops {
        assert!((d - 30.0).abs() < 2.0, "loopback {d} ms");
    }
}
