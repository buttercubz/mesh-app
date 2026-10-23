//! Field bench logic (DESIGN §12, contracts C7 and C8): bench catalog, traffic
//! generators, per-link statistics, the NTP-style clock-offset estimator, the loopback
//! correlator and the `labrun/1` record emitter.
//!
//! Sans-IO: time is a caller-supplied [`Instant`] (µs), there is no I/O, no clock, no
//! threads and no randomness. mesh-core drives a [`BenchRunner`] from its Lab mode
//! (links, LAB frames, platform samples) and writes the produced [`LabRecord`] lines to
//! the per-device JSONL file. The JSON Schema of the lines is
//! `crates/mesh-lab/schema/labrun-1.schema.json` ([`SCHEMA_JSON`]).
//!
//! Owned by WP-05 (stub created by WP-01, DESIGN §16.2). Sans-IO crate: the determinism
//! lints below are denied here (clippy.toml holds the list; the workspace allows them for
//! every other crate).
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]

pub mod catalog;
pub mod clock;
pub mod correlator;
pub mod geo;
pub mod json;
pub mod kinds;
pub mod linkstats;
pub mod params;
pub mod record;
mod record_json;
pub mod ridelog;
pub mod runner;
pub mod sim;
pub mod stats;
pub mod time;
pub mod traffic;
pub mod verdict;

pub use catalog::{BenchSpec, CATALOG, spec};
pub use clock::{ClockEstimate, ClockEstimator, Exchange};
pub use correlator::{ClickTrain, LoopbackCorrelator, LoopbackResult, LoopbackSeries};
pub use kinds::{
    Anchor, Audio, Bench, BtState, LinkKind, LinkRole, Pattern, RunRole, Transport, Verdict,
};
pub use linkstats::{Stats, WindowStats};
pub use params::{LabParams, ParamError, RunParams};
pub use record::LabRecord;
pub use record_json::{SUMMARY_CSV_HEADER, summary_csv};
pub use runner::{BenchRunner, LabTx, LiveSample, PlatformSample, RunConfig};
pub use time::Instant;

pub use ridelog::{DiagEvent, DiagRecord, RIDELOG_SCHEMA_JSON, RideRecorder};

/// The `labrun/1` JSON Schema (contract C7), embedded for runtimes and tests.
pub const SCHEMA_JSON: &str = include_str!("../schema/labrun-1.schema.json");
