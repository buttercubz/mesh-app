//! Lab-mode scripting helpers shared by the scenarios: waiting for links, starting
//! runs, collecting and schema-checking labrun/1 files, and the capture-gating check.

use mesh_api::{CommandResult, LabAnchor, LabAudio, LabBench, LabParams, UiCommand, UiEvent};
use mesh_core::effects::{AudioCommand, Effect};
use serde_json::Value;

use crate::schema::SchemaValidator;
use crate::world::World;

/// Minimal parameters of a run of `secs` seconds.
pub fn params(audio: LabAudio, anchor: LabAnchor, secs: u32) -> LabParams {
    let mut p = LabParams::new(audio, anchor, true);
    p.duration_s = Some(secs);
    p
}

/// Runs until node `i` has at least `n` Lab links up; `false` after `timeout_ms`.
pub fn wait_links(w: &mut World, i: usize, n: usize, timeout_ms: u64) -> bool {
    let until = w.now.plus_ms(timeout_ms);
    while w.now < until {
        if lab_links(w, i) >= n {
            return true;
        }
        w.run_ms(100);
    }
    lab_links(w, i) >= n
}

/// Lab links of node `i` in the Up state.
pub fn lab_links(w: &World, i: usize) -> usize {
    w.nodes[i]
        .engine
        .ctx()
        .links
        .iter()
        .filter(|l| l.state == mesh_core::LinkState::Up && l.is_lab())
        .count()
}

/// Starts a run on node `i` (as the controller) and returns its id.
pub fn start(w: &mut World, i: usize, bench: LabBench, p: LabParams) -> Result<String, String> {
    match w.command(i, UiCommand::LabStart { bench, params: p }) {
        CommandResult::LabRun(r) => Ok(r.run_id),
        other => Err(format!("labStart refused on node {i}: {other:?}")),
    }
}

/// Runs until node `i` reports `labFinished` for `run_id` (or `timeout_ms` passes).
pub fn wait_finished(w: &mut World, i: usize, run_id: &str, timeout_ms: u64) -> bool {
    let until = w.now.plus_ms(timeout_ms);
    loop {
        let done = w.nodes[i]
            .ui_events()
            .any(|e| matches!(e, UiEvent::LabFinished { run_id: r, .. } if r == run_id));
        if done {
            return true;
        }
        if w.now >= until {
            return false;
        }
        w.run_ms(250);
    }
}

/// The summary of a finished run on node `i`.
pub fn summary(w: &World, i: usize, run_id: &str) -> Option<mesh_api::LabSummary> {
    w.nodes[i].ui_events().find_map(|e| match e {
        UiEvent::LabFinished {
            run_id: r, summary, ..
        } if r == run_id => Some(summary.clone()),
        _ => None,
    })
}

/// Validates every line of node `i`'s file for `run_id`; returns the parsed lines.
pub fn validated_lines(
    w: &World,
    i: usize,
    run_id: &str,
    v: &SchemaValidator,
) -> Result<Vec<Value>, String> {
    let lines = w.nodes[i].lab_lines(run_id);
    if lines.is_empty() {
        return Err(format!("node {i}: no labrun/1 file for {run_id}"));
    }
    lines
        .iter()
        .enumerate()
        .map(|(k, l)| {
            v.validate_line(l)
                .map_err(|e| format!("node {i} {run_id} line {k}: {e}\n  {l}"))
        })
        .collect()
}

/// Structural checks of one device file: `run` first with the run's bench, audio and
/// anchor, `summary` last.
pub fn check_file_shape(
    lines: &[Value],
    run_id: &str,
    bench: &str,
    audio: &str,
    anchor: &str,
) -> Result<(), String> {
    let first = lines.first().ok_or("empty file")?;
    let last = lines.last().ok_or("empty file")?;
    let want = [
        ("t", "run"),
        ("runId", run_id),
        ("bench", bench),
        ("audio", audio),
        ("anchor", anchor),
    ];
    for (k, v) in want {
        if first[k] != v {
            return Err(format!("run line {k} = {} (want {v})", first[k]));
        }
    }
    if last["t"] != "summary" {
        return Err(format!("last line is {} (want summary)", last["t"]));
    }
    Ok(())
}

/// Every `StartCapture` of every node was emitted while handling a `labStart` with
/// `audio = full` on that node (DESIGN §3.2 capture gating, S10). Returns the
/// offending descriptions.
pub fn capture_gating_violations(w: &World) -> Vec<String> {
    let mut bad = Vec::new();
    for n in &w.nodes {
        for (k, (t, e)) in n.effects.iter().enumerate() {
            if !matches!(e, Effect::Audio(AudioCommand::StartCapture)) {
                continue;
            }
            let ok = n.commands.iter().any(|c| {
                (c.start..c.end).contains(&k)
                    && matches!(&c.cmd, UiCommand::LabStart { params, .. } if params.audio == LabAudio::Full)
            });
            if !ok {
                bad.push(format!(
                    "node {} StartCapture at {:?} outside a labStart(full)",
                    n.idx, t
                ));
            }
        }
    }
    bad
}

/// Audio commands node `i` emitted while handling its command number `k`.
pub fn command_audio(w: &World, i: usize, k: usize) -> Vec<AudioCommand> {
    let n = &w.nodes[i];
    let Some(c) = n.commands.get(k) else {
        return Vec::new();
    };
    n.effects[c.start..c.end]
        .iter()
        .filter_map(|(_, e)| match e {
            Effect::Audio(a) => Some(*a),
            _ => None,
        })
        .collect()
}
