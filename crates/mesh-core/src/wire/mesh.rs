//! Registration of the `mesh` module (DESIGN §3.3, owned by WP-10D): the multi-hop
//! glue between the engine and mesh-routing (relay, pacing, NBR, link selection).

use crate::engine::ModuleRegistry;
use crate::mesh::MeshModule;

/// Registers the `mesh` module (after `media`, so it sees every MESH record the group
/// and media modules delivered without claiming).
pub fn register(reg: &mut ModuleRegistry) {
    reg.register(Box::new(MeshModule::new()));
}
