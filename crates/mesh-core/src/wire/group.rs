//! Registration of the `group` module (DESIGN §3.3, WP-10B). The group module also
//! drives the `secure` and `session` sub-modules, which share its identities, keys and
//! roster state (see `crate::group::core`).

use crate::engine::ModuleRegistry;

/// Registers the group module (secure links, groups, ride session).
pub fn register(reg: &mut ModuleRegistry) {
    reg.register(Box::new(crate::group::GroupCore::new()));
}
