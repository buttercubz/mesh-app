//! Module registration (DESIGN §3.3): the fixed list of mesh-core modules. Each
//! sub-WP plugs in by editing only its own `wire/<module>.rs` (and its module
//! directory), never `engine.rs` or `Cargo.toml`.

pub mod group;
pub mod media;
pub mod mesh;
pub mod secure;
pub mod session;

use crate::engine::ModuleRegistry;

/// Registers every module, in call order: secure, group, session, media, mesh.
pub fn register_all(reg: &mut ModuleRegistry) {
    secure::register(reg);
    group::register(reg);
    session::register(reg);
    media::register(reg);
    mesh::register(reg);
}
