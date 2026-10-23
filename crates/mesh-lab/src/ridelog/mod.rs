//! `ridelog/1`: the always-on diagnostics flight recorder (contract C7b, schema
//! `schema/ridelog-1.schema.json`, reference `docs/lab/LAB_PROTOCOL.md` §ridelog).
//!
//! Producers:
//! - **mesh-core** (stage 2) emits [`DiagRecord`]s through `Effect::Diag`: typed
//!   [`DiagEvent`]s with static codes and, on its 1 Hz control tick, a [`CoreSample`]
//!   with *cumulative* counters.
//! - **the native runtime** (mesh-ffi, mesh-wasm) owns a [`RideRecorder`], adds RT
//!   counters ([`RtCounters`]) and the platform's [`PlatDiag`], accepts platform/UI
//!   [`NoteEvent`]s, and forwards [`Batch`]es to the platform file store.
//!
//! Sans-IO like the rest of the crate: no clock, no I/O, no randomness.

pub mod event;
mod event_fields;
pub mod header;
pub mod recorder;
pub mod redact;
pub mod sample;
pub mod sampler;

use sha2::{Digest, Sha256};

pub use event::{
    DiagEvent, DiagSrc, FloorKind, FrameType, GlitchKind, MarkLabel, NoteError, NoteEvent, is_ident,
};
pub use event_fields::hex4;
pub use header::{DeviceInfo, EndReason, Header, iso8601, session_id, utc_date};
pub use recorder::{Batch, RecorderConfig, RideRecorder};
pub use sample::{
    CoreAudio, CoreJb, CoreLink, CoreSample, DirCounters, JbCounters, MeshCounters, PlatDiag,
    QueueDepth, RtCounters,
};
pub use sampler::{FxCounters, SampleInputs, Sampler};

/// Schema id written in every `hdr` line.
pub const RIDELOG_SCHEMA_ID: &str = "ridelog/1";
/// The `ridelog/1` JSON Schema (draft 2020-12).
pub const RIDELOG_SCHEMA_JSON: &str = include_str!("../../schema/ridelog-1.schema.json");

/// What mesh-core hands to the runtime in `Effect::Diag`.
#[derive(Debug, Clone, PartialEq)]
pub enum DiagRecord {
    Event(DiagEvent),
    /// Built on the 1 Hz control tick; counters are cumulative.
    Sample(Box<CoreSample>),
}

/// `rideKey`: the first 4 bytes of `SHA-256("ridelog" ‖ group_id ‖ u32 UTC date
/// YYYYMMDD, little-endian)`. Every member of a group computes the same key on the same
/// UTC day, so the analyzer can group files of one ride without any identity in them.
pub fn ride_key(group_id: &[u8], utc_yyyymmdd: u32) -> [u8; 4] {
    let mut h = Sha256::new();
    h.update(b"ridelog");
    h.update(group_id);
    h.update(utc_yyyymmdd.to_le_bytes());
    let d = h.finalize();
    [d[0], d[1], d[2], d[3]]
}

/// Ride key of a Lab session: `lab-` + 4 hex digits of the public Lab tag.
pub fn lab_ride_key(lab_tag: u16) -> String {
    format!("lab-{lab_tag:04x}")
}

/// Ride key of the Demo ride.
pub const DEMO_RIDE_KEY: &str = "demo";
