//! Monotonic acceptance of signed control state (DESIGN §4.7, normative):
//!
//! - ROSTER: the signature verifies against the current leader key and `roster_version`
//!   is **strictly greater** than the stored one;
//! - REKEY: `new_epoch > epoch` and it contains a sealed entry for this node that opens;
//! - GROUP_CTL: `ctl_version` strictly greater and `epoch ≥` the current epoch;
//! - JOIN_REQ: idempotent, keyed by `id_hash` ([`crate::join::JoinBook`]).
//!
//! Anti-entropy (§8.9) uses exactly these rules, so it can never roll state back.
//! [`ControlState`] stores the latest accepted bodies verbatim and applies the effects.

mod state;

pub use state::{ControlEffect, ControlState, SyncKind};

use mesh_wire::mesh::{GroupCtl, Rekey, Roster};

use crate::error::{CryptoError, Result};
use crate::identity::GroupIdentity;
use crate::prim::Secret32;
use crate::{group_ctl as gc, rekey as rk, roster as ro};

/// ROSTER acceptance: signature by `leader_ed_pk`, same group, version strictly greater
/// than `stored_version` (`None` before the first roster).
pub fn roster(
    candidate: &Roster,
    group_id: &[u8; 16],
    leader_ed_pk: &[u8; 32],
    stored_version: Option<u32>,
) -> Result<()> {
    ro::verify(candidate, group_id, leader_ed_pk)?;
    if stored_version.is_some_and(|v| candidate.roster_version <= v) {
        return Err(CryptoError::Stale("roster_version not greater than stored"));
    }
    Ok(())
}

/// REKEY acceptance: signature by `leader_ed_pk`, `new_epoch > current_epoch`, and an
/// entry for `me` that opens. Returns the new epoch secret.
pub fn rekey(
    candidate: &Rekey,
    group_id: &[u8; 16],
    leader_ed_pk: &[u8; 32],
    current_epoch: u32,
    me: &GroupIdentity,
) -> Result<Secret32> {
    rk::verify(candidate, group_id, leader_ed_pk)?;
    if candidate.new_epoch <= current_epoch {
        return Err(CryptoError::Stale(
            "new_epoch not greater than current epoch",
        ));
    }
    rk::open_entry(candidate, me, me.node_id())
}

/// GROUP_CTL acceptance: signature by `leader_ed_pk`, `ctl_version` strictly greater than
/// `stored_ctl_version` (`None` before the first), `epoch ≥ current_epoch`.
pub fn group_ctl(
    candidate: &GroupCtl,
    group_id: &[u8; 16],
    leader_ed_pk: &[u8; 32],
    stored_ctl_version: Option<u32>,
    current_epoch: u32,
) -> Result<()> {
    gc::verify(candidate, group_id, leader_ed_pk)?;
    if stored_ctl_version.is_some_and(|v| candidate.ctl_version <= v) {
        return Err(CryptoError::Stale("ctl_version not greater than stored"));
    }
    if candidate.epoch < current_epoch {
        return Err(CryptoError::Stale("GROUP_CTL for an older epoch"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
