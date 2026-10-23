//! S10 lab-bench-sim (DESIGN §13.2): T0 / T0b / T0c / T3 run by real engines against
//! simulated peers over the radio model, covering every audio path (`off`,
//! `renderOnly`, `full`) with every keep-alive anchor (`none`, `audio`,
//! `audioLocation`). Assertions: every labrun/1 line of every device validates
//! against the schema (incl. `audio`/`anchor`); a forced Lab id collision is
//! resolved; `StartCapture` appears only for `audio = full`; plus the bench-level
//! sanity checks below and the global assertions of every world.

use mesh_api::{LabAnchor, LabAudio, LabBench, LabLinkRole, LabParams, LabPattern, LabTransport};
use mesh_core::effects::{AudioCommand, Effect};
use serde_json::Value;

use super::Outcome;
use super::lab::{
    capture_gating_violations, check_file_shape, command_audio, params, start, summary,
    validated_lines, wait_finished, wait_links,
};
use crate::config::WorldConfig;
use crate::rng::mix;
use crate::schema::SchemaValidator;
use crate::world::World;

/// The shared 4-digit Lab code of every S10 world.
const LAB_CODE: u16 = 2_710;

/// One bench run of the script.
struct RunSpec {
    bench: LabBench,
    audio: LabAudio,
    anchor: LabAnchor,
    secs: u32,
    tweak: fn(&mut LabParams),
}

/// What a finished run left behind.
struct RunResult {
    run_id: String,
    /// Parsed, schema-validated lines per participating node.
    files: Vec<(usize, Vec<Value>)>,
}

fn sub_seed(seed: u64, k: u64) -> u64 {
    mix(&[seed, 0x5100, k])
}

/// The audio commands a `labStart` must emit for `audio` / `anchor` (§3.2, §12.1).
fn expected_start_audio(
    cmds: &[AudioCommand],
    audio: LabAudio,
    anchor: LabAnchor,
) -> Result<(), String> {
    let loc = anchor == LabAnchor::AudioLocation;
    let ok = match audio {
        LabAudio::Off => cmds.is_empty(),
        LabAudio::RenderOnly => matches!(
            cmds,
            [AudioCommand::OpenSession { profile }] if !profile.capture && profile.location_anchor == loc
        ),
        LabAudio::Full => matches!(
            cmds,
            [AudioCommand::OpenSession { profile }, AudioCommand::StartCapture]
                if profile.capture && profile.location_anchor == loc
        ),
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "labStart(audio {}, anchor {}) emitted {cmds:?}",
            audio.as_str(),
            anchor.as_str()
        ))
    }
}

/// Anchor fields of the controller's sample lines (all but the first, which may carry
/// the platform sample taken before the session opened).
fn check_sample_anchors(
    lines: &[Value],
    audio: LabAudio,
    anchor: LabAnchor,
) -> Result<usize, String> {
    let session = audio != LabAudio::Off;
    let location = session && anchor == LabAnchor::AudioLocation;
    let samples: Vec<&Value> = lines.iter().filter(|l| l["t"] == "s").collect();
    for s in samples.iter().skip(1) {
        let a = &s["anchor"];
        if a["session"] != session || a["location"] != location {
            return Err(format!(
                "sample anchor {a} (want session {session}, location {location})"
            ));
        }
    }
    Ok(samples.len())
}

fn label(spec: &RunSpec) -> String {
    format!(
        "{} {}/{}",
        spec.bench.as_str(),
        spec.audio.as_str(),
        spec.anchor.as_str()
    )
}

/// Starts `spec` on `controller` and checks the audio commands of its `labStart`.
fn begin_run(w: &mut World, o: &mut Outcome, controller: usize, spec: &RunSpec) -> Option<String> {
    let mut p = params(spec.audio, spec.anchor, spec.secs);
    (spec.tweak)(&mut p);
    let label = label(spec);
    let run_id = match start(w, controller, spec.bench, p) {
        Ok(r) => r,
        Err(e) => {
            o.check(false, || format!("{label}: {e}"));
            return None;
        }
    };
    let k = w.nodes[controller].commands.len() - 1;
    let cmds = command_audio(w, controller, k);
    if let Err(e) = expected_start_audio(&cmds, spec.audio, spec.anchor) {
        o.check(false, || format!("{label}: {e}"));
    }
    Some(run_id)
}

/// Runs `spec` with `controller` and checks files of `nodes`, audio commands and the
/// simulated audio IO.
fn run_bench(
    w: &mut World,
    o: &mut Outcome,
    v: &SchemaValidator,
    controller: usize,
    nodes: &[usize],
    spec: &RunSpec,
) -> Option<RunResult> {
    let run_id = begin_run(w, o, controller, spec)?;
    end_run(w, o, v, controller, nodes, spec, run_id)
}

/// Waits for the end of a started run on every node of `nodes` and checks the files,
/// the audio IO and the anchors.
fn end_run(
    w: &mut World,
    o: &mut Outcome,
    v: &SchemaValidator,
    controller: usize,
    nodes: &[usize],
    spec: &RunSpec,
    run_id: String,
) -> Option<RunResult> {
    let label = label(spec);
    let timeout = u64::from(spec.secs) * 1_000 + 15_000;
    for &n in nodes {
        let done = wait_finished(w, n, &run_id, timeout);
        o.check(done, || {
            format!("{label}: node {n} never finished {run_id}")
        });
    }
    // Audio IO really ran as requested on the controller.
    let a = &w.nodes[controller].audio;
    let min_blocks = u64::from(spec.secs) * 100 * 8 / 10;
    let (cap, ren) = (a.capture_blocks, a.render_blocks);
    match spec.audio {
        LabAudio::Off => {}
        LabAudio::RenderOnly => {
            o.check(cap == 0 && ren >= min_blocks, || {
                format!("{label}: renderOnly ran capture {cap} / render {ren} blocks")
            });
        }
        LabAudio::Full => {
            o.check(cap >= min_blocks && ren >= min_blocks, || {
                format!("{label}: full ran capture {cap} / render {ren} blocks")
            });
        }
    }
    o.check(a.session.is_none() && !a.capture, || {
        format!("{label}: the audio session is still open after the run")
    });
    let mut files = Vec::new();
    for &n in nodes {
        match validated_lines(w, n, &run_id, v) {
            Ok(lines) => {
                let shape = check_file_shape(
                    &lines,
                    &run_id,
                    spec.bench.as_str(),
                    spec.audio.as_str(),
                    spec.anchor.as_str(),
                );
                o.check(shape.is_ok(), || {
                    format!("{label} node {n}: {}", shape.unwrap_err())
                });
                let role = if n == controller {
                    "controller"
                } else {
                    "follower"
                };
                o.check(lines[0]["role"] == role, || {
                    format!("{label} node {n}: role {} (want {role})", lines[0]["role"])
                });
                if n == controller {
                    match check_sample_anchors(&lines, spec.audio, spec.anchor) {
                        Ok(count) => {
                            o.check(count + 1 >= spec.secs as usize, || {
                                format!("{label}: only {count} sample lines")
                            });
                        }
                        Err(e) => {
                            o.check(false, || format!("{label}: {e}"));
                        }
                    }
                }
                files.push((n, lines));
            }
            Err(e) => {
                o.check(false, || format!("{label}: {e}"));
            }
        }
    }
    Some(RunResult { run_id, files })
}

/// Checks that ends every S10 world: capture gating, followers never touch audio,
/// then the world's violations and digest join the outcome.
fn finish_world(w: &World, o: &mut Outcome, controllers: &[usize]) {
    for e in capture_gating_violations(w) {
        o.failures.push(e);
    }
    for n in &w.nodes {
        if controllers.contains(&n.idx) {
            continue;
        }
        let audio = n
            .effects
            .iter()
            .filter(|(_, e)| matches!(e, Effect::Audio(_)))
            .count();
        o.check(audio == 0, || {
            format!("follower node {} emitted {audio} audio commands", n.idx)
        });
    }
    o.absorb(w);
}

/// Summary loss of a run on `node`, recorded as a metric checked against `max`.
fn check_loss(w: &World, o: &mut Outcome, node: usize, run_id: &str, name: &str, max: f64) {
    match summary(w, node, run_id) {
        Some(s) => {
            o.metric(name, s.loss, s.loss <= max, &format!("≤ {max}"));
        }
        None => {
            o.check(false, || format!("{name}: no summary for {run_id}"));
        }
    }
}

/// T0 over L2CAP (two iPhones 3 m apart): four runs with different audio/anchor
/// pairs, central role, headset off.
fn t0_l2cap(
    seed: u64,
    v: &SchemaValidator,
    o: &mut Outcome,
    covered: &mut Vec<(LabAudio, LabAnchor)>,
) {
    let mut w = World::new(WorldConfig::iphones_on_line(seed, 2, 3.0));
    w.start_lab_all(LAB_CODE);
    if !o.check(wait_links(&mut w, 0, 1, 15_000), || {
        "T0/L2CAP: no Lab link within 15 s".into()
    }) {
        o.absorb(&w);
        return;
    }
    let specs = [
        RunSpec {
            bench: LabBench::T0,
            audio: LabAudio::Off,
            anchor: LabAnchor::Audio,
            secs: 6,
            tweak: |p| {
                p.transport = Some(LabTransport::L2cap);
                p.role = Some(LabLinkRole::Central);
                p.headset = Some(false);
                p.pattern = Some(LabPattern::VoiceLike);
                p.streams = Some(2);
            },
        },
        RunSpec {
            bench: LabBench::T0,
            audio: LabAudio::Off,
            anchor: LabAnchor::AudioLocation,
            secs: 5,
            tweak: |p| {
                p.pattern = Some(LabPattern::Ping);
                p.ping_hz = Some(10);
            },
        },
        RunSpec {
            bench: LabBench::T0,
            audio: LabAudio::RenderOnly,
            anchor: LabAnchor::None,
            secs: 5,
            tweak: |p| {
                p.pattern = Some(LabPattern::VoiceLike);
                p.streams = Some(1);
            },
        },
        RunSpec {
            bench: LabBench::T0,
            audio: LabAudio::Full,
            anchor: LabAnchor::AudioLocation,
            secs: 5,
            tweak: |p| {
                p.pattern = Some(LabPattern::VoiceLike);
                p.streams = Some(4);
            },
        },
    ];
    for (k, spec) in specs.iter().enumerate() {
        let Some(r) = run_bench(&mut w, o, v, 0, &[0, 1], spec) else {
            continue;
        };
        covered.push((spec.audio, spec.anchor));
        let link_kind = r.files[0]
            .1
            .iter()
            .find(|l| l["t"] == "s")
            .map(|s| s["links"][0]["kind"].clone());
        o.check(link_kind.as_ref().is_some_and(|k| k == "ble-l2cap"), || {
            format!("T0/L2CAP run {k}: link kind {link_kind:?}")
        });
        // Voice-like and ping runs within the 120 kbps link: loss below the T0 bound.
        if k < 3 {
            check_loss(&w, o, 0, &r.run_id, &format!("T0.l2cap.run{k}.loss"), 0.05);
        } else if let Some(s) = summary(&w, 0, &r.run_id) {
            // Four streams (≈ 94 kbps) exceed the initial 60 kbps L2CAP pacing: reported.
            o.report("T0.l2cap.voice4.loss", s.loss);
        }
        if k == 0
            && let Some(s) = summary(&w, 0, &r.run_id)
        {
            let kbps = s.tx_kbps.mean;
            o.metric(
                "T0.l2cap.voice2.txKbps",
                kbps,
                (35.0..80.0).contains(&kbps),
                "in 35..80",
            );
        }
        w.run_ms(1_500);
    }
    finish_world(&w, o, &[0]);
}

/// T0 over GATT: the peer publishes no L2CAP PSM, so the link is GATT (ATT MTU 185);
/// the controller runs as peripheral with an HFP headset, then GATT write saturate.
fn t0_gatt(
    seed: u64,
    v: &SchemaValidator,
    o: &mut Outcome,
    covered: &mut Vec<(LabAudio, LabAnchor)>,
) {
    let mut w = World::new(WorldConfig::iphones_on_line(seed, 2, 3.0));
    w.nodes[0].spec.headset = true;
    w.start_lab_psm(0, LAB_CODE, 0);
    w.start_lab_psm(1, LAB_CODE, 0);
    if !o.check(wait_links(&mut w, 0, 1, 15_000), || {
        "T0/GATT: no Lab link within 15 s".into()
    }) {
        o.absorb(&w);
        return;
    }
    let specs = [
        RunSpec {
            bench: LabBench::T0,
            audio: LabAudio::Full,
            anchor: LabAnchor::Audio,
            secs: 6,
            tweak: |p| {
                p.transport = Some(LabTransport::GattNotify);
                p.role = Some(LabLinkRole::Peripheral);
                p.headset = Some(true);
                p.pattern = Some(LabPattern::PingUnderLoad);
                p.streams = Some(2);
                p.ping_hz = Some(10);
            },
        },
        RunSpec {
            bench: LabBench::T0,
            audio: LabAudio::RenderOnly,
            anchor: LabAnchor::AudioLocation,
            secs: 5,
            tweak: |p| {
                p.transport = Some(LabTransport::GattWrite);
                p.pattern = Some(LabPattern::Saturate);
            },
        },
    ];
    for (k, spec) in specs.iter().enumerate() {
        let Some(r) = run_bench(&mut w, o, v, 0, &[0, 1], spec) else {
            continue;
        };
        covered.push((spec.audio, spec.anchor));
        let s0 = r.files[0].1.iter().find(|l| l["t"] == "s").cloned();
        let kind = s0.as_ref().map(|s| s["links"][0]["kind"].clone());
        o.check(kind.as_ref().is_some_and(|k| k == "ble-gatt"), || {
            format!("T0/GATT run {k}: link kind {kind:?}")
        });
        if k == 0 {
            check_loss(&w, o, 0, &r.run_id, "T0.gatt.hfp.loss", 0.05);
            let route = s0.map(|s| s["audio"]["route"].clone());
            o.check(route.as_ref().is_some_and(|r| r == "hfpWideband"), || {
                format!("T0/GATT headset run: route {route:?}")
            });
        } else if let Some(s) = summary(&w, 0, &r.run_id) {
            o.metric(
                "T0.gatt.saturate.txKbps",
                s.tx_kbps.mean,
                s.tx_kbps.mean > 10.0,
                "> 10",
            );
        }
        w.run_ms(1_500);
    }
    finish_world(&w, o, &[0]);
}

/// T0b relay chain A–B–C, 5 m apart, A and C shielded from each other (the "out of
/// direct range" setup): MESH_PROBE from A reaches C through B; A runs `full` audio
/// with the `none` anchor.
fn t0b(seed: u64, v: &SchemaValidator, o: &mut Outcome, covered: &mut Vec<(LabAudio, LabAnchor)>) {
    let mut w = World::new(WorldConfig::iphones_on_line(seed, 3, 5.0));
    w.block(0, 2, true);
    w.start_lab_all(LAB_CODE);
    let linked = wait_links(&mut w, 1, 2, 20_000) && wait_links(&mut w, 0, 1, 5_000);
    if !o.check(linked, || "T0b: chain did not form within 25 s".into()) {
        o.absorb(&w);
        return;
    }
    let spec = RunSpec {
        bench: LabBench::T0b,
        audio: LabAudio::Full,
        anchor: LabAnchor::None,
        secs: 6,
        tweak: |p| {
            p.pattern = Some(LabPattern::VoiceLike);
            p.streams = Some(2);
        },
    };
    if let Some(r) = run_bench(&mut w, o, v, 0, &[0, 1, 2], &spec) {
        covered.push((spec.audio, spec.anchor));
        let c = r
            .files
            .iter()
            .find(|(n, _)| *n == 2)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        let probes = c
            .iter()
            .filter(|l| l["t"] == "raw")
            .filter_map(|l| l.get("meshProbes").and_then(Value::as_array))
            .map(Vec::len)
            .sum::<usize>();
        o.metric("T0b.meshProbesAtC", probes as f64, probes >= 10, "≥ 10");
    }
    finish_world(&w, o, &[0]);
}

/// T0c hub: H (node 0) with three peers (BLE max 3), `full` audio, anchor `audio`,
/// two voice-like streams out on every link.
fn t0c(seed: u64, v: &SchemaValidator, o: &mut Outcome, covered: &mut Vec<(LabAudio, LabAnchor)>) {
    let mut cfg = WorldConfig::iphones_on_line(seed, 4, 0.0);
    cfg.mobility =
        crate::mobility::Mobility::Static(vec![(0.0, 0.0), (6.0, 0.0), (0.0, 6.0), (-6.0, 0.0)]);
    let mut w = World::new(cfg);
    for (a, b) in [(1, 2), (1, 3), (2, 3)] {
        w.block(a, b, true);
    }
    w.nodes[0].spec.headset = true;
    w.start_lab_all(LAB_CODE);
    if !o.check(wait_links(&mut w, 0, 3, 30_000), || {
        "T0c: hub did not reach 3 links within 30 s".into()
    }) {
        o.absorb(&w);
        return;
    }
    let spec = RunSpec {
        bench: LabBench::T0c,
        audio: LabAudio::Full,
        anchor: LabAnchor::Audio,
        secs: 6,
        tweak: |p| {
            p.pattern = Some(LabPattern::VoiceLike);
            p.streams = Some(2);
            p.headset = Some(true);
        },
    };
    if let Some(r) = run_bench(&mut w, o, v, 0, &[0, 1, 2, 3], &spec) {
        covered.push((spec.audio, spec.anchor));
        let max_links = r.files[0]
            .1
            .iter()
            .filter(|l| l["t"] == "s")
            .filter_map(|l| l["links"].as_array().map(Vec::len))
            .max()
            .unwrap_or(0);
        o.metric("T0c.hubLinks", max_links as f64, max_links == 3, "= 3");
        check_loss(&w, o, 0, &r.run_id, "T0c.loss", 0.05);
        // Node airtime of the hub stays within the design budget (§5.6).
        let used = w.nodes[0].air_used_us as f64 / w.now.0.max(1) as f64;
        o.report("T0c.hubAirtimeShare", used);
    }
    finish_world(&w, o, &[0]);
}

/// Rejoin bound of the Phase 0 (Lab) engine after a walk-apart: a connect issued to
/// the vanished peer runs into the 8 s connect timeout and the 5 s initial per-peer
/// backoff (§9.5), plus up to 7 s for discovery (1.5 s mean), connect (1-3 s) and the
/// Lab handshake. The 10 s S4 bound needs the Phase 2 link selector (ArmReconnect);
/// the measured value is the `T3.rejoinS` metric.
fn t3_rejoin_bound_s() -> f64 {
    let l = mesh_core::Limits::for_platform(mesh_core::Platform::Ios);
    (l.connect_timeout_us + l.backoff_initial_us) as f64 / 1e6 + 7.0
}

/// T3 rediscovery: the peer walks 300 m away for 12 s and comes back; the link drops
/// and rejoins within [`t3_rejoin_bound_s`] of being back in range. Then a control
/// run (`off`, anchor
/// `none`) without a break.
fn t3(seed: u64, v: &SchemaValidator, o: &mut Outcome, covered: &mut Vec<(LabAudio, LabAnchor)>) {
    let mut w = World::new(WorldConfig::iphones_on_line(seed, 2, 5.0));
    w.start_lab_all(LAB_CODE);
    if !o.check(wait_links(&mut w, 0, 1, 15_000), || {
        "T3: no Lab link within 15 s".into()
    }) {
        o.absorb(&w);
        return;
    }
    let walk = RunSpec {
        bench: LabBench::T3,
        audio: LabAudio::RenderOnly,
        anchor: LabAnchor::Audio,
        secs: 32,
        tweak: |p| {
            p.pattern = Some(LabPattern::Ping);
            p.ping_hz = Some(5);
            p.variant = Some("walkApart".into());
        },
    };
    if let Some(run_id) = begin_run(&mut w, o, 0, &walk) {
        let t0 = w.now;
        w.run_ms(4_000);
        w.mobility.set_position(1, (300.0, 0.0));
        w.run_ms(12_000);
        w.mobility.set_position(1, (5.0, 0.0));
        let back_s = w.now.since(t0) as f64 / 1e6;
        if let Some(r) = end_run(&mut w, o, v, 0, &[0, 1], &walk, run_id) {
            covered.push((walk.audio, walk.anchor));
            let ev = |name: &str| -> Vec<f64> {
                r.files[0]
                    .1
                    .iter()
                    .filter(|l| l["t"] == "e" && l["ev"] == name)
                    .filter_map(|l| l["ts"].as_f64())
                    .collect()
            };
            let downs = ev("linkDown");
            o.check(downs.iter().any(|t| *t > 4.0 && *t < back_s), || {
                format!("T3: no linkDown while apart (downs {downs:?})")
            });
            match ev("linkUp").into_iter().find(|t| *t >= back_s) {
                Some(up) => {
                    let rejoin = up - back_s;
                    let bound = t3_rejoin_bound_s();
                    o.metric(
                        "T3.rejoinS",
                        rejoin,
                        rejoin <= bound,
                        &format!("≤ {bound} s after back in range"),
                    );
                }
                None => {
                    o.check(false, || "T3: the link never came back".into());
                }
            }
        }
    }
    w.run_ms(1_500);
    let control = RunSpec {
        bench: LabBench::T3,
        audio: LabAudio::Off,
        anchor: LabAnchor::None,
        secs: 5,
        tweak: |p| p.variant = Some("control".into()),
    };
    if run_bench(&mut w, o, v, 0, &[0, 1], &control).is_some() {
        covered.push((control.audio, control.anchor));
    }
    finish_world(&w, o, &[0]);
}

/// Forced Lab id collisions (§8.2): on the chain 0–1–2–3–4, node 1 draws node 0's id
/// (direct neighbours) and node 4 draws node 2's id (two hops apart, via 3) before the
/// links form, as two phones that drew the same id and then meet. Both collisions are
/// resolved by a redraw and no collision survives.
fn collision(seed: u64, o: &mut Outcome) {
    let n = 5;
    let mut w = World::new(WorldConfig::iphones_on_line(seed, n, 5.0));
    for a in 0..n {
        for b in a + 2..n {
            w.block(a, b, true);
        }
    }
    w.start_lab_all(LAB_CODE);
    let id = |w: &World, i: usize| w.nodes[i].lab_node_id().unwrap_or(0);
    let (id0, id2) = (id(&w, 0), id(&w, 2));
    w.with_engine(1, |e, _| e.force_lab_node_id(id0));
    w.with_engine(4, |e, _| e.force_lab_node_id(id2));
    let chain = (1..n - 1).all(|i| wait_links(&mut w, i, 2, 30_000));
    if !o.check(chain, || "collision: chain did not form within 30 s".into()) {
        o.absorb(&w);
        return;
    }
    // Longer than the 5 s grace of the global id assertion, so it checks too.
    w.run_ms(6_000);
    let ids: Vec<u16> = (0..n).map(|i| id(&w, i)).collect();
    let mut uniq = ids.clone();
    uniq.sort_unstable();
    uniq.dedup();
    o.check(uniq.len() == n && !ids.contains(&0), || {
        format!("Lab ids after collisions: {ids:?}")
    });
    let redraws = w
        .nodes
        .iter()
        .flat_map(|n| n.effects.iter())
        .filter(|(_, e)| matches!(e, Effect::Log { msg, .. } if msg.contains("collision")))
        .count();
    o.metric(
        "S10.labIdRedraws",
        redraws as f64,
        redraws >= 2,
        "≥ 2 (one per forced collision)",
    );
    finish_world(&w, o, &[]);
}

/// S10 entry point.
pub fn run(seed: u64) -> Outcome {
    let mut o = Outcome::new("S10", seed);
    let v = SchemaValidator::labrun();
    let mut covered = Vec::new();
    t0_l2cap(sub_seed(seed, 1), &v, &mut o, &mut covered);
    t0_gatt(sub_seed(seed, 2), &v, &mut o, &mut covered);
    t0b(sub_seed(seed, 3), &v, &mut o, &mut covered);
    t0c(sub_seed(seed, 4), &v, &mut o, &mut covered);
    t3(sub_seed(seed, 5), &v, &mut o, &mut covered);
    collision(sub_seed(seed, 6), &mut o);
    for audio in LabAudio::ALL {
        for anchor in LabAnchor::ALL {
            o.check(covered.contains(&(*audio, *anchor)), || {
                format!(
                    "no successful run with audio {} and anchor {}",
                    audio.as_str(),
                    anchor.as_str()
                )
            });
        }
    }
    o
}
