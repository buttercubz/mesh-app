//! Shared test helpers around the library's two-device simulation.
#![allow(dead_code, unused_imports)]

use mesh_lab::{Bench, LabParams, RunConfig, RunParams, RunRole};

pub use mesh_lab::sim::{LINK, PairSim as Pair, jsonl, platform};

pub fn config(bench: Bench, params: &LabParams, role: RunRole, lab_node: u16) -> RunConfig {
    mesh_lab::sim::config(
        &RunParams::resolve(bench, params).expect("valid params"),
        role,
        lab_node,
    )
}

pub fn pair(bench: Bench, params: &LabParams, offset_us: u64) -> Pair {
    Pair::new(
        &RunParams::resolve(bench, params).expect("valid params"),
        offset_us,
    )
}
