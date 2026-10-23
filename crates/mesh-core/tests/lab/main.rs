//! Lab mode (WP-10A): bench runs between in-memory engines over simulated links, the
//! audio path per `LabParams.audio`, the MESH_PROBE flood, lab_node_id collisions,
//! T5a loopback through push_capture/pull_playout, and determinism.

#[path = "../links/harness.rs"]
mod harness;

mod audio_paths;
mod runs;

use boon::{Compiler, SchemaIndex, Schemas};
use harness::World;
use mesh_api::{CommandResult, LabAnchor, LabAudio, LabBench, LabParams, LabPattern, UiCommand};
use mesh_core::{AnchorState, Instant, PlatformEvent, PlatformSample};
use serde_json::Value;

const SCHEMA_URL: &str = "https://convlink.voice/schema/labrun-1.schema.json";

/// The labrun/1 schema (contract C7).
pub fn schema() -> (Schemas, SchemaIndex) {
    let mut schemas = Schemas::new();
    let mut c = Compiler::new();
    let v: Value = serde_json::from_str(mesh_lab::SCHEMA_JSON).expect("schema is JSON");
    c.add_resource(SCHEMA_URL, v).expect("resource");
    let idx = c.compile(SCHEMA_URL, &mut schemas).expect("compiles");
    (schemas, idx)
}

/// Validates every line; returns them parsed.
pub fn validate(lines: &[String]) -> Vec<Value> {
    let (s, idx) = schema();
    lines
        .iter()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}"));
            if let Err(e) = s.validate(&v, idx) {
                panic!("{e:#}\nline: {l}");
            }
            v
        })
        .collect()
}

/// A platform sample with the audio session anchor up, in the foreground.
pub fn sample(w: &World, session: bool) -> PlatformSample {
    PlatformSample {
        wall_ms: Some(1_792_746_723_120 + w.now.0 / 1_000),
        battery: Some(0.8),
        charging: Some(false),
        anchor: AnchorState {
            session: Some(session),
            location: None,
            live_activity: None,
        },
        ..PlatformSample::default()
    }
}

/// Feeds a platform sample to every node every second for `secs` seconds.
pub fn run_with_samples(w: &mut World, secs: u64, session: bool) {
    for _ in 0..secs {
        for i in 0..w.nodes.len() {
            let s = sample(w, session);
            w.event(i, PlatformEvent::Platform(s));
        }
        w.run_ms(1_000);
    }
}

/// Parameters for a short run.
pub fn params(audio: LabAudio, anchor: LabAnchor, secs: u32) -> LabParams {
    let mut p = LabParams::new(audio, anchor, false);
    p.duration_s = Some(secs);
    p
}

/// Starts a run on node `i` and returns its id.
pub fn start(w: &mut World, i: usize, bench: LabBench, p: LabParams) -> String {
    match w.command(i, UiCommand::LabStart { bench, params: p }) {
        CommandResult::LabRun(r) => r.run_id,
        other => panic!("labStart refused: {other:?}"),
    }
}

/// A voice-like pattern with `streams` streams.
pub fn voice(mut p: LabParams, streams: u8) -> LabParams {
    p.pattern = Some(LabPattern::VoiceLike);
    p.streams = Some(streams);
    p
}

/// The current simulated time.
pub fn now(w: &World) -> Instant {
    w.now
}
