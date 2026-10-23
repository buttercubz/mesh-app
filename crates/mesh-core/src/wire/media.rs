//! Registration of the `media` module (DESIGN §3.3; stub created by WP-10A, owned by
//! WP-10C): capture and receive chains, earcons, prompts, audio check, latency
//! estimator.

use crate::engine::ModuleRegistry;
use crate::media::MediaModule;

/// Registers the `media` module.
pub fn register(reg: &mut ModuleRegistry) {
    reg.register(Box::new(MediaModule::new()));
}
