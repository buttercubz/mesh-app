//! ROSTER signing and verification (DESIGN §4.7, §8.5): `ver u8=1 · group_id[16] ·
//! epoch u32 · roster_version u32 · invite_sig[64] · n u8 · n × (node_id u16 ·
//! id_hash[16] · ed_pk[32] · role u8 · name_len u8 · name[≤ 24 B]) · sig[64]`, signed by
//! the leader's per-group key over `"cvic/v1/roster" ‖ preceding bytes`.
//!
//! Monotonic acceptance lives in [`crate::accept`].

pub use mesh_wire::mesh::{Roster, RosterEntry, RosterRole};

use crate::error::{CryptoError, Result};
use crate::identity::{GroupIdentity, PublicIdentity};
use crate::invite::{InvitePolicy, InviteSig, invite_signed_message};
use crate::join::VerifiedJoin;
use crate::prim::{self, Secret32};

/// Builds and signs a roster. Entries keep their order (the roster index is the position
/// among non-denied entries). Fails if two non-denied entries share a `node_id`: the
/// leader never assigns a roster entry to a colliding `node_id`.
pub fn build(
    leader: &GroupIdentity,
    group_id: [u8; 16],
    epoch: u32,
    roster_version: u32,
    invite_sig: InviteSig,
    entries: Vec<RosterEntry>,
) -> Result<Roster> {
    let mut seen: Vec<u16> = Vec::with_capacity(entries.len());
    for e in entries.iter().filter(|e| e.role != RosterRole::Denied) {
        if seen.contains(&e.node_id) {
            return Err(CryptoError::NodeIdCollision);
        }
        seen.push(e.node_id);
    }
    let mut r = Roster {
        group_id,
        epoch,
        roster_version,
        invite_sig: invite_sig.0,
        entries,
        sig: [0; 64],
    };
    sign(leader, &mut r)?;
    Ok(r)
}

/// (Re)signs `roster` with the leader key.
pub fn sign(leader: &GroupIdentity, roster: &mut Roster) -> Result<()> {
    roster.sig = leader.sign(&roster.signed_message()?);
    Ok(())
}

/// Verifies the leader signature and the group.
pub fn verify(roster: &Roster, group_id: &[u8; 16], leader_ed_pk: &[u8; 32]) -> Result<()> {
    if &roster.group_id != group_id {
        return Err(CryptoError::WrongGroup);
    }
    prim::ed25519_verify(leader_ed_pk, &roster.signed_message()?, &roster.sig)
}

/// The roster entry with role Leader.
pub fn leader_entry(roster: &Roster) -> Option<&RosterEntry> {
    roster.entries.iter().find(|e| e.role == RosterRole::Leader)
}

/// The leader key the roster claims for itself, if its signature verifies under it.
/// Used to detect `inviteLeaderMismatch` when the invite's key does not verify.
pub fn self_signed_leader(roster: &Roster) -> Option<[u8; 32]> {
    let l = leader_entry(roster)?;
    verify(roster, &roster.group_id, &l.ed_pk)
        .ok()
        .map(|_| l.ed_pk)
}

/// A roster entry for a verified joiner.
pub fn entry_for(join: &VerifiedJoin, role: RosterRole) -> RosterEntry {
    entry_for_identity(&join.identity, role, &join.name)
}

/// A roster entry for a public identity.
pub fn entry_for_identity(id: &PublicIdentity, role: RosterRole, name: &str) -> RosterEntry {
    RosterEntry {
        node_id: id.node_id(),
        id_hash: id.id_hash(),
        ed_pk: id.ed_pk,
        role,
        name: mesh_wire::name::truncate_graphemes(name, mesh_wire::name::DISPLAY_NAME_MAX)
            .to_owned(),
    }
}

/// Marks `id_hash` as denied (Remove member). Returns `false` if it is not listed.
pub fn deny(entries: &mut [RosterEntry], id_hash: &[u8; 16]) -> bool {
    match entries.iter_mut().find(|e| &e.id_hash == id_hash) {
        Some(e) => {
            e.role = RosterRole::Denied;
            true
        }
        None => false,
    }
}

/// The non-denied entry of `id_hash`.
pub fn member<'a>(roster: &'a Roster, id_hash: &[u8; 16]) -> Option<&'a RosterEntry> {
    roster
        .entries
        .iter()
        .find(|e| &e.id_hash == id_hash && e.role != RosterRole::Denied)
}

/// The Ed25519 key of the non-denied member with `node_id` (verifies LEAVE/RIDE_CTL).
pub fn ed_pk_of(roster: &Roster, node_id: u16) -> Option<[u8; 32]> {
    roster
        .entries
        .iter()
        .find(|e| e.node_id == node_id && e.role != RosterRole::Denied)
        .map(|e| e.ed_pk)
}

/// Checks the roster's `invite_sig` against the epoch secret I hold for `roster.epoch`
/// plus the group's name, policy and leader key: the condition under which a member
/// can re-share a leader-signed invite.
pub fn verify_invite_sig(
    roster: &Roster,
    epoch_secret: &Secret32,
    policy: InvitePolicy,
    group_name: &str,
    leader_ed_pk: &[u8; 32],
) -> Result<()> {
    let m = invite_signed_message(
        &roster.group_id,
        roster.epoch,
        policy,
        group_name,
        leader_ed_pk,
        epoch_secret,
    );
    prim::ed25519_verify(leader_ed_pk, &m, &roster.invite_sig)
}

#[cfg(test)]
mod tests;
