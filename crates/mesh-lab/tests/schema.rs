//! Contract C7: every emitted `labrun/1` line validates against
//! `schema/labrun-1.schema.json` (JSON Schema 2020-12, boon validator); a run line
//! without `audio` or `anchor`, and a sample without `anchor` or `bt`, are rejected.

mod common;

use boon::{Compiler, SchemaIndex, Schemas};
use common::jsonl;
use mesh_lab::{
    Anchor, Audio, Bench, BenchRunner, CATALOG, Instant, LabParams, LabRecord, RunRole, SCHEMA_JSON,
};
use serde_json::{Value, json};

const SCHEMA_URL: &str = "https://convlink.voice/schema/labrun-1.schema.json";

fn schema() -> (Schemas, SchemaIndex) {
    let mut schemas = Schemas::new();
    let mut c = Compiler::new();
    let v: Value = serde_json::from_str(SCHEMA_JSON).expect("schema is JSON");
    c.add_resource(SCHEMA_URL, v).expect("resource");
    let idx = c
        .compile(SCHEMA_URL, &mut schemas)
        .expect("schema compiles");
    (schemas, idx)
}

fn validate_line(s: &Schemas, idx: SchemaIndex, line: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}: {line}"))?;
    s.validate(&v, idx)
        .map_err(|e| format!("{e:#}\nline: {line}"))
}

fn validate_all(s: &Schemas, idx: SchemaIndex, text: &str) -> usize {
    let mut n = 0;
    for line in text.lines() {
        if let Err(e) = validate_line(s, idx, line) {
            panic!("{e}");
        }
        n += 1;
    }
    n
}

#[test]
fn every_emitted_line_validates() {
    let (s, idx) = schema();
    let mut kinds = std::collections::BTreeSet::new();
    // A T0 run with pings, a mark and a link drop, on both devices.
    let mut p = LabParams::new(Audio::Full, Anchor::Audio, true);
    p.pattern = Some(mesh_lab::Pattern::PingUnderLoad);
    p.duration_s = Some(8);
    p.variant = Some("pocket \"quoted\", ñ".into());
    let mut pair = common::pair(Bench::T0, &p, 3_000_000);
    pair.run(3.0);
    pair.a.mark(Instant(pair.now), "stop at the gas station");
    pair.link_down();
    pair.run(1.5);
    pair.link_up();
    pair.a.on_event(
        Instant(pair.now),
        "routeChange",
        vec![("route".into(), "builtin".into())],
    );
    pair.run(5.0);
    for lines in [&pair.lines_a, &pair.lines_b] {
        for l in lines.iter() {
            kinds.insert(match l {
                LabRecord::Run(_) => "run",
                LabRecord::Sample(_) => "s",
                LabRecord::Event { .. } => "e",
                LabRecord::Mark { .. } => "mark",
                LabRecord::Raw(_) => "raw",
                LabRecord::Summary(_) => "summary",
            });
        }
        assert!(validate_all(&s, idx, &jsonl(lines)) > 10);
    }
    // A run stopped early, with no platform samples at all.
    let mut r = BenchRunner::new(
        common::config(
            Bench::T2,
            &LabParams::defaults_for(Bench::T2, false),
            RunRole::Controller,
            1,
        ),
        Instant(0),
    );
    r.tick(Instant(2_500_000));
    r.stop(Instant(2_600_000));
    validate_all(&s, idx, &jsonl(&r.drain()));
    // Every bench's default run line validates.
    for spec in CATALOG {
        let params = LabParams::defaults_for(spec.bench, true);
        let mut r = BenchRunner::new(
            common::config(spec.bench, &params, RunRole::Follower, 2),
            Instant(0),
        );
        r.tick(Instant(1_000_000));
        r.stop(Instant(1_000_001));
        validate_all(&s, idx, &jsonl(&r.drain()));
    }
    assert_eq!(kinds.len(), 6, "line kinds covered: {kinds:?}");
}

#[test]
fn t5a_loopback_and_t0b_mesh_lines_validate() {
    let (s, idx) = schema();
    let p = LabParams::defaults_for(Bench::T5a, false);
    let mut r = BenchRunner::new(
        common::config(Bench::T5a, &p, RunRole::Controller, 7),
        Instant(0),
    );
    let mut idx_c = 0u64;
    let mut rendered = Vec::new();
    for step in 0..400u64 {
        let now = Instant(step * 10_000);
        if let Some(train) = r.poll_click(now, idx_c) {
            rendered.resize(idx_c as usize, 0);
            rendered.extend(train);
        }
        let block: Vec<i16> = (idx_c..idx_c + 160)
            .map(|i| {
                i.checked_sub(500)
                    .and_then(|j| rendered.get(j as usize))
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        r.on_capture(now, idx_c, &block);
        idx_c += 160;
        r.tick(now);
    }
    r.stop(Instant(4_000_000));
    let lines = r.drain();
    assert!(
        lines
            .iter()
            .any(|l| matches!(l, LabRecord::Raw(raw) if !raw.loopback.is_empty()))
    );
    validate_all(&s, idx, &jsonl(&lines));

    let mut sink = BenchRunner::new(
        common::config(
            Bench::T0b,
            &LabParams::defaults_for(Bench::T0b, false),
            RunRole::Follower,
            3,
        ),
        Instant(0),
    );
    let mut probe = mesh_wire::mesh::MeshProbe::new(9, 123_456_789);
    probe.append(42).unwrap();
    sink.on_mesh_probe(Instant(500_000), 7, &probe, 2, 61_000);
    sink.stop(Instant(1_200_000));
    let lines = sink.drain();
    assert!(
        lines
            .iter()
            .any(|l| matches!(l, LabRecord::Raw(raw) if !raw.mesh_probes.is_empty()))
    );
    validate_all(&s, idx, &jsonl(&lines));
}

/// The DESIGN §12.3 example lines, with the mandatory sample fields added.
fn design_example() -> Vec<Value> {
    vec![
        json!({"t":"run","v":1,"runId":"r-20261023-0912-ab12","bench":"T0","variant":{"transport":"l2cap","role":"central","headset":"hfp-wb","pattern":"voice4"},"audio":"full","anchor":"audio","startedAt":"2026-10-23T09:12:03.120Z","device":{"model":"iPhone15,2","os":"iOS 27.0","app":"0.1.0 (12)","platform":"ios"},"labNode":4711,"role":"controller","peers":[{"labNode":918,"model":"iPhone13,1"}],"params":{"durationS":60,"streams":4,"payloadB":118,"intervalMs":40}}),
        json!({"t":"s","ts":1.000,"links":[{"id":3,"kind":"ble-l2cap","role":"central","rssi":-61,"txKbps":93.4,"rxKbps":93.1,"loss":0.004,"rttP50":38.0,"rttP95":61.5,"owdP95":31.2,"stalls":0,"maxWrite":null}],"gps":{"lat":18.4861,"lon":-69.9312,"spd":0.0,"acc":4.8},"peerGps":[{"labNode":918,"lat":18.4862,"lon":-69.9311}],"distM":12.4,"audio":{"capFps":100,"renFps":100,"running":true,"route":"hfpWideband","ladder":"L0","xruns":0},"dev":{"bat":0.81,"chg":false,"therm":"nominal","app":"background","locked":true},"anchor":{"ok":true,"session":true,"location":false,"liveActivity":true},"bt":"on"}),
        json!({"t":"e","ts":13.104,"ev":"linkDown","link":3,"reason":"timeout"}),
        json!({"t":"mark","ts":20.0,"label":"pocket"}),
        json!({"t":"raw","ts":21.0,"pings":[[1203,1.000000,1.038112,0.000210]]}),
        json!({"t":"summary","durationS":60.0,"txKbps":{"mean":92.8},"loss":0.006,"owdMs":{"p50":19.1,"p95":31.2,"p99":44.0},"stalls":0,"verdict":"go"}),
    ]
}

#[test]
fn design_example_validates_and_mandatory_fields_are_enforced() {
    let (s, idx) = schema();
    let ex = design_example();
    for v in &ex {
        s.validate(v, idx).unwrap_or_else(|e| panic!("{e:#}\n{v}"));
    }
    let reject = |v: &Value, why: &str| assert!(s.validate(v, idx).is_err(), "accepted {why}: {v}");
    for field in ["audio", "anchor"] {
        let mut run = ex[0].clone();
        run.as_object_mut().unwrap().remove(field);
        reject(&run, &format!("a run line without {field}"));
    }
    let mut bad_audio = ex[0].clone();
    bad_audio["audio"] = json!("loud");
    reject(&bad_audio, "an unknown audio path");
    let mut bad_anchor = ex[0].clone();
    bad_anchor["anchor"] = json!(null);
    reject(&bad_anchor, "a null anchor");
    for field in ["anchor", "bt"] {
        let mut sample = ex[1].clone();
        sample.as_object_mut().unwrap().remove(field);
        reject(&sample, &format!("a sample without {field}"));
    }
    let mut bad_run_id = ex[0].clone();
    bad_run_id["runId"] = json!("run-1");
    reject(&bad_run_id, "a malformed runId");
    reject(&json!({"t":"nope","ts":1.0}), "an unknown line type");
    reject(&json!({"t":"raw","ts":1.0}), "an empty raw line");
}

#[test]
fn lab_analyze_fixtures_validate() {
    let (s, idx) = schema();
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/lab-analyze/fixtures");
    let mut files = 0;
    for entry in std::fs::read_dir(&dir).expect("fixtures directory") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let text = std::fs::read_to_string(&path).expect("read fixture");
            validate_all(&s, idx, &text);
            files += 1;
        }
    }
    assert!(
        files >= 2,
        "expected at least two fixture files in {}",
        dir.display()
    );
}
