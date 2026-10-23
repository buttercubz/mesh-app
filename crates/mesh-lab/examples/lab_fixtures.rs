//! Writes the lab-analyze fixtures (`tools/lab-analyze/fixtures/*.jsonl`) from two
//! simulated devices, so the fixtures are real `BenchRunner` output:
//!
//! - T0 (both devices): 2 voice-like streams + pings with the headset path, 20 ms one way;
//! - T1 (both devices, 15 → 70 m apart): 40 s good, 30 s where both phones lost their
//!   keep-alive anchor and the link went silent, 40 s good again.
//!
//! Run: `cargo run -p mesh-lab --example lab_fixtures [-- <out dir>]`.

use std::path::PathBuf;

use mesh_lab::sim::{PairSim, jsonl};
use mesh_lab::{Anchor, Audio, Bench, LabParams, Pattern, RunParams};

fn main() -> std::io::Result<()> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/lab-analyze/fixtures")
        });
    std::fs::create_dir_all(&out)?;

    let mut t0 = LabParams::new(Audio::Full, Anchor::Audio, false);
    t0.pattern = Some(Pattern::PingUnderLoad);
    t0.streams = Some(2);
    t0.ping_hz = Some(5);
    t0.duration_s = Some(12);
    let p = RunParams::resolve(Bench::T0, &t0).expect("T0 params");
    let mut pair = PairSim::new(&p, 2_500_000);
    pair.jitter_us = 3_000;
    pair.run(13.0);
    pair.stop();
    std::fs::write(out.join("t0-device-a.jsonl"), jsonl(&pair.lines_a))?;
    std::fs::write(out.join("t0-device-b.jsonl"), jsonl(&pair.lines_b))?;

    let mut t1 = LabParams::defaults_for(Bench::T1, true);
    t1.duration_s = Some(110);
    t1.ping_hz = Some(2);
    let p = RunParams::resolve(Bench::T1, &t1).expect("T1 params");
    let mut pair = PairSim::new(&p, 900_000);
    pair.jitter_us = 2_000;
    // The second vehicle pulls away from 15 m to ~70 m.
    pair.separation_m = 15.0;
    pair.separation_m_per_s = 0.5;
    pair.run(40.0);
    pair.a.mark(mesh_lab::Instant(pair.now), "pocket");
    pair.anchor_ok = false;
    pair.blackout = true;
    pair.run(30.0);
    pair.anchor_ok = true;
    pair.blackout = false;
    pair.run(41.0);
    pair.stop();
    std::fs::write(out.join("t1-device-a.jsonl"), jsonl(&pair.lines_a))?;
    std::fs::write(out.join("t1-device-b.jsonl"), jsonl(&pair.lines_b))?;
    println!("fixtures written to {}", out.display());
    Ok(())
}
