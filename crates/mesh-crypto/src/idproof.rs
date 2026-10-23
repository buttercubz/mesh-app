//! IDPROOF (DESIGN §4.4, §8.4): the first SEC record each way,
//! `ed_pk[32] · x_pk[32] · sig[64]` with
//! `sig = Ed25519(sk, "cvic/v1/idproof" ‖ role u8 ‖ handshake_hash(32))`.
//!
//! It binds the per-group Ed25519 identity to the Noise-authenticated X25519 static key
//! and to this exact handshake (channel binding). Other records are rejected until both
//! proofs verify; then the deny-list, collision and duplicate-link checks run
//! ([`admit_peer`]).

pub use mesh_wire::record::IdProof;

use crate::error::{CryptoError, Result};
use crate::identity::{CollisionOutcome, GroupIdentity, NodeIdRegistry, PublicIdentity};
use crate::keyring::DenyList;
use crate::session::Role;

/// Builds my IDPROOF for a link where I hold `role`.
pub fn create(identity: &GroupIdentity, role: Role, handshake_hash: &[u8; 32]) -> IdProof {
    let msg = IdProof::signed_message(role.wire(), handshake_hash);
    IdProof {
        ed_pk: identity.ed_pk(),
        x_pk: identity.x_pk(),
        sig: identity.sign(&msg),
    }
}

/// Verifies the peer's IDPROOF. `peer_role` is the role of the *signer* (the opposite of
/// mine) and `noise_remote_static` the X25519 key the handshake authenticated.
pub fn verify(
    proof: &IdProof,
    peer_role: Role,
    handshake_hash: &[u8; 32],
    noise_remote_static: &[u8; 32],
) -> Result<PublicIdentity> {
    if &proof.x_pk != noise_remote_static {
        return Err(CryptoError::IdentityMismatch);
    }
    let msg = IdProof::signed_message(peer_role.wire(), handshake_hash);
    let id = PublicIdentity {
        ed_pk: proof.ed_pk,
        x_pk: proof.x_pk,
    };
    id.verify(&msg, &proof.sig)?;
    Ok(id)
}

/// Post-IDPROOF admission: deny list, then `node_id` collision (§8.2). Duplicate-link
/// detection needs the link table and stays with the engine.
pub fn admit_peer(
    peer: &PublicIdentity,
    me: &[u8; 16],
    deny: &DenyList,
    registry: &mut NodeIdRegistry,
) -> Result<CollisionOutcome> {
    let h = peer.id_hash();
    deny.check(&h)?;
    if &h == me {
        return Err(CryptoError::Invalid("peer presents my own identity"));
    }
    let outcome = registry.observe(&h, me);
    match outcome {
        CollisionOutcome::None => Ok(outcome),
        _ => Err(CryptoError::NodeIdCollision),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn binds_role_hash_and_static_key() {
        let mut rng = ChaCha20Rng::from_seed([2; 32]);
        let me = GroupIdentity::generate(&mut rng);
        let other = GroupIdentity::generate(&mut rng);
        let hh = [9u8; 32];
        let p = create(&me, Role::Initiator, &hh);
        let id = verify(&p, Role::Initiator, &hh, &me.x_pk()).unwrap();
        assert_eq!(id.id_hash(), me.id_hash());
        // Wrong role, wrong handshake hash, wrong Noise static key.
        assert_eq!(
            verify(&p, Role::Responder, &hh, &me.x_pk()),
            Err(CryptoError::BadSignature)
        );
        assert!(verify(&p, Role::Initiator, &[8; 32], &me.x_pk()).is_err());
        assert_eq!(
            verify(&p, Role::Initiator, &hh, &other.x_pk()),
            Err(CryptoError::IdentityMismatch)
        );
        // Swapping in another Ed25519 key breaks the signature.
        let mut q = p;
        q.ed_pk = other.ed_pk();
        assert!(verify(&q, Role::Initiator, &hh, &me.x_pk()).is_err());
    }

    #[test]
    fn admission_checks_deny_and_collisions() {
        let mut rng = ChaCha20Rng::from_seed([3; 32]);
        let me = GroupIdentity::generate(&mut rng);
        let peer = GroupIdentity::generate(&mut rng);
        let mut deny = DenyList::new();
        let mut reg = NodeIdRegistry::new();
        assert!(admit_peer(peer.public(), &me.id_hash(), &deny, &mut reg).is_ok());
        assert!(admit_peer(me.public(), &me.id_hash(), &deny, &mut reg).is_err());
        deny.deny(peer.id_hash());
        assert_eq!(
            admit_peer(peer.public(), &me.id_hash(), &deny, &mut reg),
            Err(CryptoError::Denied)
        );
    }
}
