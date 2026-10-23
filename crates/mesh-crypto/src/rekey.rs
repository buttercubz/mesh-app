//! REKEY sealing and opening (DESIGN §4.7, §8.6):
//!
//! ```text
//! REKEY  = new_epoch u32 · roster_version u32 · leader_eph_pk[32] · n u8
//!          · n × (node_id u16 · sealed[48]) · sig[64]
//! sealed = ChaCha20-Poly1305(k_i, nonce = 0, epoch_secret_{new_epoch})
//! k_i    = HKDF(X25519(leader_eph, x_pk_i), "cvic/v1/rekey" ‖ new_epoch ‖ node_id)
//! sig    = Ed25519(leader, "cvic/v1/rekey" ‖ group_id ‖ preceding bytes)
//! ```
//!
//! `leader_eph` is fresh per REKEY (drawn from the injected RNG), so each `k_i` is used
//! exactly once and nonce 0 is safe. HKDF here is Extract with an empty salt (RFC 5869
//! zero salt) over the X25519 output, then Expand with the info above to 32 bytes; the
//! AEAD has empty associated data.

pub use mesh_wire::mesh::{Rekey, RekeyEntry};

use mesh_wire::mesh::SEALED_LEN;
use rand_chacha::ChaCha20Rng;

use crate::error::{CryptoError, Result};
use crate::identity::GroupIdentity;
use crate::prim::{self, Secret32};

/// HKDF info prefix of the per-recipient key.
pub const REKEY_KDF_LABEL: &[u8] = b"cvic/v1/rekey";

/// One REKEY recipient: a current roster member's `node_id` and X25519 key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RekeyRecipient {
    pub node_id: u16,
    pub x_pk: [u8; 32],
}

/// `k_i` from the X25519 shared secret.
pub fn entry_key(shared: &Secret32, new_epoch: u32, node_id: u16) -> Secret32 {
    let prk = prim::hkdf_extract(&[], shared.expose());
    prim::hkdf_expand32(
        prk.expose(),
        &[
            REKEY_KDF_LABEL,
            &new_epoch.to_le_bytes(),
            &node_id.to_le_bytes(),
        ],
    )
}

/// The leader seals `new_secret` to every recipient with a fresh ephemeral key and signs
/// the REKEY. Recipients must have distinct `node_id`s.
pub fn seal(
    leader: &GroupIdentity,
    group_id: &[u8; 16],
    new_epoch: u32,
    roster_version: u32,
    new_secret: &Secret32,
    recipients: &[RekeyRecipient],
    rng: &mut ChaCha20Rng,
) -> Result<Rekey> {
    let eph = Secret32::random(rng);
    seal_with_ephemeral(
        leader,
        group_id,
        new_epoch,
        roster_version,
        new_secret,
        recipients,
        &eph,
    )
}

/// [`seal`] with a given ephemeral private key (vectors; production uses [`seal`]).
pub fn seal_with_ephemeral(
    leader: &GroupIdentity,
    group_id: &[u8; 16],
    new_epoch: u32,
    roster_version: u32,
    new_secret: &Secret32,
    recipients: &[RekeyRecipient],
    eph: &Secret32,
) -> Result<Rekey> {
    if recipients.len() > Rekey::MAX_ENTRIES {
        return Err(CryptoError::Invalid("too many REKEY recipients"));
    }
    let mut entries = Vec::with_capacity(recipients.len());
    for (i, r) in recipients.iter().enumerate() {
        if recipients[..i].iter().any(|o| o.node_id == r.node_id) {
            return Err(CryptoError::NodeIdCollision);
        }
        let shared = prim::x25519(eph.expose(), &r.x_pk)?;
        let k = entry_key(&shared, new_epoch, r.node_id);
        let mut sealed = [0u8; SEALED_LEN];
        sealed[..32].copy_from_slice(new_secret.expose());
        let tag = prim::aead_seal(k.expose(), &[0u8; 12], &[], &mut sealed[..32]);
        sealed[32..].copy_from_slice(&tag);
        entries.push(RekeyEntry {
            node_id: r.node_id,
            sealed,
        });
    }
    let mut rekey = Rekey {
        new_epoch,
        roster_version,
        leader_eph_pk: prim::x25519_public(eph.expose()),
        entries,
        sig: [0; 64],
    };
    rekey.sig = leader.sign(&rekey.signed_message(group_id)?);
    Ok(rekey)
}

/// Verifies the leader's signature on a REKEY.
pub fn verify(rekey: &Rekey, group_id: &[u8; 16], leader_ed_pk: &[u8; 32]) -> Result<()> {
    prim::ed25519_verify(leader_ed_pk, &rekey.signed_message(group_id)?, &rekey.sig)
}

/// Opens the entry addressed to `my_node_id` (signature not checked here; see
/// [`open`] and [`crate::accept::rekey`]).
pub fn open_entry(rekey: &Rekey, me: &GroupIdentity, my_node_id: u16) -> Result<Secret32> {
    let sealed = rekey.entry_for(my_node_id).ok_or(CryptoError::NoEntry)?;
    let shared = me.dh(&rekey.leader_eph_pk)?;
    let k = entry_key(&shared, rekey.new_epoch, my_node_id);
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&sealed[..32]);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&sealed[32..]);
    prim::aead_open(k.expose(), &[0u8; 12], &[], &mut buf, &tag)?;
    Ok(Secret32::new(buf))
}

/// Verifies the signature, then opens my entry.
pub fn open(
    rekey: &Rekey,
    group_id: &[u8; 16],
    leader_ed_pk: &[u8; 32],
    me: &GroupIdentity,
) -> Result<Secret32> {
    verify(rekey, group_id, leader_ed_pk)?;
    open_entry(rekey, me, me.node_id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::SeedableRng;

    const GID: [u8; 16] = [0x77; 16];

    #[test]
    fn removed_member_cannot_open_remaining_members_can() {
        let mut rng = ChaCha20Rng::from_seed([41; 32]);
        let leader = GroupIdentity::generate(&mut rng);
        let members: Vec<GroupIdentity> =
            (0..4).map(|_| GroupIdentity::generate(&mut rng)).collect();
        let removed = &members[3];
        let recipients: Vec<RekeyRecipient> = members[..3]
            .iter()
            .chain(core::iter::once(&leader))
            .map(|m| RekeyRecipient {
                node_id: m.node_id(),
                x_pk: m.x_pk(),
            })
            .collect();
        let secret = Secret32::random(&mut rng);
        let rk = seal(&leader, &GID, 5, 9, &secret, &recipients, &mut rng).unwrap();
        assert_eq!(rk.entries.len(), 4);
        let wire = Rekey::decode(&rk.encode().unwrap()).unwrap();
        for m in &members[..3] {
            assert_eq!(open(&wire, &GID, &leader.ed_pk(), m).unwrap(), secret);
        }
        assert_eq!(
            open(&wire, &GID, &leader.ed_pk(), removed),
            Err(CryptoError::NoEntry)
        );
        // Even if the removed member reuses someone else's entry, its key cannot open it.
        assert_eq!(
            open_entry(&wire, removed, members[0].node_id()),
            Err(CryptoError::Decrypt)
        );
        // Signature binds the group and the leader.
        assert!(open(&wire, &[0; 16], &leader.ed_pk(), &members[0]).is_err());
        assert!(open(&wire, &GID, &members[1].ed_pk(), &members[0]).is_err());
        let mut tampered = wire.clone();
        tampered.new_epoch = 6;
        assert!(open(&tampered, &GID, &leader.ed_pk(), &members[0]).is_err());
    }

    #[test]
    fn fresh_ephemeral_per_rekey_and_bounded() {
        let mut rng = ChaCha20Rng::from_seed([42; 32]);
        let leader = GroupIdentity::generate(&mut rng);
        let m = GroupIdentity::generate(&mut rng);
        let r = [RekeyRecipient {
            node_id: m.node_id(),
            x_pk: m.x_pk(),
        }];
        let s = Secret32::new([1; 32]);
        let a = seal(&leader, &GID, 1, 1, &s, &r, &mut rng).unwrap();
        let b = seal(&leader, &GID, 1, 1, &s, &r, &mut rng).unwrap();
        assert_ne!(a.leader_eph_pk, b.leader_eph_pk);
        assert_ne!(a.entries[0].sealed, b.entries[0].sealed);
        let dup = [r[0], r[0]];
        assert!(seal(&leader, &GID, 1, 1, &s, &dup, &mut rng).is_err());
        let low = [RekeyRecipient {
            node_id: 1,
            x_pk: [0; 32],
        }];
        assert_eq!(
            seal(&leader, &GID, 1, 1, &s, &low, &mut rng).unwrap_err(),
            CryptoError::BadKey
        );
    }
}
