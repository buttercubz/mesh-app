//! The in-crate labrun/1 checker (`mesh_sim::schema`) agrees with the reference JSON
//! Schema 2020-12 validator (`boon`) on every line the simulated engines write and on
//! systematic mutations of those lines (removed, retyped, out-of-range and extra
//! fields at every depth), so S10's "files validate against the schema" means what
//! the C7 contract means.

use boon::{Compiler, SchemaIndex, Schemas};
use mesh_api::{LabAnchor, LabAudio, LabBench, LabPattern};
use mesh_sim::scenario::lab;
use mesh_sim::schema::SchemaValidator;
use mesh_sim::{World, WorldConfig};
use serde_json::{Value, json};

const SCHEMA_URL: &str = "https://convlink.voice/schema/labrun-1.schema.json";

fn boon_schema() -> (Schemas, SchemaIndex) {
    let mut schemas = Schemas::new();
    let mut c = Compiler::new();
    let v: Value = serde_json::from_str(mesh_lab::SCHEMA_JSON).expect("schema JSON");
    c.add_resource(SCHEMA_URL, v).expect("resource");
    let idx = c.compile(SCHEMA_URL, &mut schemas).expect("compiles");
    (schemas, idx)
}

/// Lines of a few real runs (controller and follower files, every audio path).
fn simulated_lines() -> Vec<Value> {
    let mut w = World::new(WorldConfig::iphones_on_line(77, 2, 3.0));
    w.start_lab_all(1_234);
    assert!(lab::wait_links(&mut w, 0, 1, 15_000));
    let runs = [
        (LabAudio::Off, LabAnchor::None, LabPattern::Ping),
        (
            LabAudio::RenderOnly,
            LabAnchor::Audio,
            LabPattern::VoiceLike,
        ),
        (
            LabAudio::Full,
            LabAnchor::AudioLocation,
            LabPattern::PingUnderLoad,
        ),
    ];
    let mut out = Vec::new();
    for (audio, anchor, pattern) in runs {
        let mut p = lab::params(audio, anchor, 3);
        p.pattern = Some(pattern);
        let id = lab::start(&mut w, 0, LabBench::T0, p).expect("labStart");
        assert!(lab::wait_finished(&mut w, 1, &id, 15_000));
        assert!(lab::wait_finished(&mut w, 0, &id, 15_000));
        for n in 0..2 {
            for l in w.nodes[n].lab_lines(&id) {
                out.push(serde_json::from_str(l).expect("JSON line"));
            }
        }
        w.run_ms(1_000);
    }
    out
}

/// Every single-point mutation of `v`, depth-first, at most `cap` of them.
fn mutations(v: &Value, cap: usize) -> Vec<Value> {
    let mut out = Vec::new();
    mutate_at(v, &mut Vec::new(), v, &mut out, cap);
    out
}

fn replace(root: &Value, path: &[PathStep], new: Option<Value>) -> Value {
    let mut r = root.clone();
    let mut cur = &mut r;
    for step in &path[..path.len() - 1] {
        cur = match step {
            PathStep::Key(k) => &mut cur[k.as_str()],
            PathStep::Idx(i) => &mut cur[*i],
        };
    }
    match (path.last().expect("non-empty path"), new) {
        (PathStep::Key(k), Some(n)) => cur[k.as_str()] = n,
        (PathStep::Key(k), None) => {
            cur.as_object_mut().expect("object").remove(k);
        }
        (PathStep::Idx(i), Some(n)) => cur[*i] = n,
        (PathStep::Idx(i), None) => {
            cur.as_array_mut().expect("array").remove(*i);
        }
    }
    r
}

#[derive(Clone)]
enum PathStep {
    Key(String),
    Idx(usize),
}

fn mutate_at(root: &Value, path: &mut Vec<PathStep>, v: &Value, out: &mut Vec<Value>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    if !path.is_empty() {
        out.push(replace(root, path, None));
        for alt in [
            json!(null),
            json!("x"),
            json!(-1),
            json!(1e12),
            json!(true),
            json!([]),
            json!({}),
        ] {
            if &alt != v {
                out.push(replace(root, path, Some(alt)));
            }
        }
    }
    match v {
        Value::Object(m) => {
            if !path.is_empty() || m.contains_key("t") {
                let mut extra = v.clone();
                extra["zzUnknown"] = json!(1);
                out.push(if path.is_empty() {
                    extra
                } else {
                    replace(root, path, Some(extra))
                });
            }
            for (k, child) in m {
                path.push(PathStep::Key(k.clone()));
                mutate_at(root, path, child, out, cap);
                path.pop();
            }
        }
        Value::Array(a) => {
            for (i, child) in a.iter().enumerate().take(2) {
                path.push(PathStep::Idx(i));
                mutate_at(root, path, child, out, cap);
                path.pop();
            }
        }
        _ => {}
    }
}

#[test]
fn the_built_in_checker_agrees_with_boon() {
    let (schemas, idx) = boon_schema();
    let ours = SchemaValidator::labrun();
    let lines = simulated_lines();
    assert!(lines.len() > 20, "only {} lines", lines.len());
    let mut kinds = std::collections::BTreeSet::new();
    let (mut checked, mut rejected) = (0usize, 0usize);
    for line in &lines {
        kinds.insert(line["t"].as_str().unwrap_or("?").to_string());
        assert!(
            schemas.validate(line, idx).is_ok(),
            "boon rejects an emitted line: {line}"
        );
        assert!(
            ours.validate(line).is_ok(),
            "ours rejects an emitted line: {line}"
        );
        for m in mutations(line, 400) {
            let reference = schemas.validate(&m, idx).is_ok();
            let mine = ours.validate(&m).is_ok();
            assert_eq!(
                mine, reference,
                "disagreement (boon ok = {reference}) on {m}"
            );
            checked += 1;
            rejected += usize::from(!reference);
        }
    }
    for k in ["run", "s", "summary"] {
        assert!(kinds.contains(k), "no {k} line among {kinds:?}");
    }
    assert!(
        checked > 1_000 && rejected > checked / 2,
        "{checked} checked, {rejected} rejected"
    );
}
