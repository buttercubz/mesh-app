//! mesh-ffi: the native runtime around the sans-IO engine (DESIGN §3.4-§3.6).
//!
//! - [`runtime`]: the engine thread (QoS / priority, 5 ms tick), the bounded crossbeam
//!   MPSC inbound channel (2048), one-shot replies for UI commands, `catch_unwind` →
//!   `engine: crashed`, and audio checks without a session.
//! - [`api`]: the UniFFI control plane (contract C4): `MeshEngine`, `PlatformHooks`,
//!   `ble_constants()`, `protocol_version()`, `api_version()`.
//! - [`rt`]: the real-time C ABI (contract C5, `include/mesh_rt.h`): process-global
//!   `rtrb` SPSC rings, RT flags and stats.
//!
//! Not a sans-IO crate: threads and clocks are allowed here (the workspace allows the
//! determinism lints and this crate does not deny them, DESIGN §3.1).
//!
//! Owned by WP-12. `start(Demo)` attaches mesh-sim's Demo ride world (bots on virtual
//! links inside the engine thread, no radio; DESIGN §3.4).

pub mod api;
pub mod rt;
pub mod runtime;

pub use api::MeshEngine;
pub use api::hooks::PlatformHooks;

uniffi::setup_scaffolding!("mesh_ffi");

#[cfg(test)]
mod tests;
