//! Deterministic discrete-event N-node simulator (DESIGN §13.2, research bench T9).
//!
//! A [`World`] runs N real `mesh_core::Engine`s on a µs clock with one seeded event
//! queue and the radio, discovery, role, mobility and audio models of §13.2. Global
//! assertions ([`audit`]) run in every scenario; [`scenario`] holds the S1-S13 suite
//! (S10 in Phase 0) and [`suite`] the fixed-seed CI / random-seed nightly runners;
//! [`presets`] are the web scenarios (pair, chain5, convoy15, cluster15).
//!
//! Sans-IO crate: the determinism lints below are denied here and only here (clippy.toml
//! holds the list; the workspace allows them for every other crate).
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]

pub mod audit;
pub mod config;
pub mod demo;
pub mod link;
pub mod mobility;
pub mod node;
pub mod onset;
pub mod presets;
pub mod radio;
pub mod ride;
pub mod rng;
pub mod scan;
pub mod synth;
pub mod talk;
pub mod tap;
pub mod world;

pub use audit::{Audit, Violation, ViolationKind};
pub use config::{AppState, DiscoveryParams, NodeSpec, WorldConfig};
pub use mobility::{Convoy, ConvoyParams, Mobility, Road};
pub use node::SimNode;
pub use radio::{Mount, RadioModel, RadioParams};
pub mod scenario;
pub mod schema;
pub mod suite;

pub use presets::{Preset, PresetOptions};
pub use scenario::{Outcome, Scenario};
pub use world::{World, WorldStats};
