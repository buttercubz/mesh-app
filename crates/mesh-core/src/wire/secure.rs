//! Registration of the `secure` module (DESIGN §3.3, WP-10B). The secure link
//! implementation (`crate::secure`) needs the per-group identity, the retained epoch
//! keyring and the roster at every link, so it is driven by the group module registered
//! in `wire/group.rs`; nothing is registered here.

use crate::engine::ModuleRegistry;

/// No separate registration: see `wire/group.rs`.
pub fn register(_reg: &mut ModuleRegistry) {}
