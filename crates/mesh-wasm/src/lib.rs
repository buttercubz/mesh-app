//! `mesh-wasm`: the browser simulator core (DESIGN §2.3, §11.2).
//!
//! A cdylib for `wasm32-unknown-unknown` exporting the raw `extern "C"` JSON ABI of
//! §11.2 ([`abi`]) over a [`SimConvoy`]: a mesh-sim `World` of real mesh-core engines
//! (SimCodec, no opus) in which node 0 is the page's phone and the others are bots that
//! ride, go live and talk. `@meshapp/engine-web` loads it (browser and Node) and drives
//! it every 20 ms.
//!
//! Not a sans-IO crate: it owns the process-wide world table. It still reads no clock
//! and no OS randomness; the page passes elapsed time and the wall clock.

pub mod abi;
mod bots;
pub mod config;
pub mod control;
pub mod convoy;
mod roam;

pub use abi::{Registry, frame_into, with_registry};
pub use config::{ConfigError, ConvoyConfig};
pub use convoy::{LOCAL, SimConvoy};
