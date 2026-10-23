//! Registration of the `session` module (DESIGN §3.3, WP-10B). The ride session
//! (`crate::session`) is driven by the group module registered in `wire/group.rs`,
//! which owns the links and the flood it originates on; nothing is registered here.

use crate::engine::ModuleRegistry;

/// No separate registration: see `wire/group.rs`.
pub fn register(_reg: &mut ModuleRegistry) {}
