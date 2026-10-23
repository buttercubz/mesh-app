//! Per-group identities (DESIGN §8.2): an Ed25519 signing key plus an independent X25519
//! static key, generated at group create or join. `id_hash = SHA-256(ed_pk ‖ x_pk)[0..16]`
//! and `node_id = u16::from_le_bytes(id_hash[0..2])`. Two groups never share an
//! identity, so membership in two groups is not linkable.

use core::cmp::Ordering;
use std::collections::BTreeMap;

use rand_chacha::ChaCha20Rng;

use crate::error::{CryptoError, Result};
use crate::prim::{self, Secret32};

/// Public half of a per-group identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicIdentity {
    pub ed_pk: [u8; 32],
    pub x_pk: [u8; 32],
}

impl PublicIdentity {
    /// `SHA-256(ed_pk ‖ x_pk)[0..16]`.
    pub fn id_hash(&self) -> [u8; 16] {
        id_hash(&self.ed_pk, &self.x_pk)
    }

    /// `u16::from_le_bytes(id_hash[0..2])`.
    pub fn node_id(&self) -> u16 {
        node_id_of(&self.id_hash())
    }

    /// Verifies an Ed25519 signature by this identity.
    pub fn verify(&self, msg: &[u8], sig: &[u8; 64]) -> Result<()> {
        prim::ed25519_verify(&self.ed_pk, msg, sig)
    }
}

/// `SHA-256(ed_pk ‖ x_pk)[0..16]`.
pub fn id_hash(ed_pk: &[u8; 32], x_pk: &[u8; 32]) -> [u8; 16] {
    let d = prim::sha256(&[ed_pk, x_pk]);
    let mut out = [0u8; 16];
    out.copy_from_slice(&d[..16]);
    out
}

/// `node_id` of an `id_hash`.
pub fn node_id_of(id_hash: &[u8; 16]) -> u16 {
    u16::from_le_bytes([id_hash[0], id_hash[1]])
}

/// A per-group identity with its secret keys. Secrets zeroize on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupIdentity {
    ed_seed: Secret32,
    x_sk: Secret32,
    public: PublicIdentity,
}

impl core::fmt::Debug for GroupIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GroupIdentity")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

impl GroupIdentity {
    /// Generates a fresh identity from the injected RNG (Ed25519 seed first, then the
    /// X25519 private key).
    pub fn generate(rng: &mut ChaCha20Rng) -> Self {
        let ed_seed = Secret32::random(rng);
        let x_sk = Secret32::random(rng);
        Self::from_secrets(ed_seed, x_sk)
    }

    /// Rebuilds an identity from its stored secrets (Keychain/Keystore blob).
    pub fn from_secrets(ed_seed: Secret32, x_sk: Secret32) -> Self {
        let public = PublicIdentity {
            ed_pk: prim::ed25519_public(ed_seed.expose()),
            x_pk: prim::x25519_public(x_sk.expose()),
        };
        GroupIdentity {
            ed_seed,
            x_sk,
            public,
        }
    }

    /// Serializes the secrets for platform storage: `ed_seed[32] ‖ x_sk[32]`.
    pub fn to_secret_bytes(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(self.ed_seed.expose());
        out[32..].copy_from_slice(self.x_sk.expose());
        out
    }

    /// Inverse of [`GroupIdentity::to_secret_bytes`].
    pub fn from_secret_bytes(bytes: &[u8; 64]) -> Self {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        a.copy_from_slice(&bytes[..32]);
        b.copy_from_slice(&bytes[32..]);
        Self::from_secrets(Secret32::new(a), Secret32::new(b))
    }

    pub fn public(&self) -> &PublicIdentity {
        &self.public
    }
    pub fn ed_pk(&self) -> [u8; 32] {
        self.public.ed_pk
    }
    pub fn x_pk(&self) -> [u8; 32] {
        self.public.x_pk
    }
    pub fn id_hash(&self) -> [u8; 16] {
        self.public.id_hash()
    }
    pub fn node_id(&self) -> u16 {
        self.public.node_id()
    }

    /// The X25519 static private key (fed to Noise and to REKEY opening).
    pub fn x25519_secret(&self) -> &Secret32 {
        &self.x_sk
    }

    /// Ed25519 signature of `msg`.
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        prim::ed25519_sign(self.ed_seed.expose(), msg)
    }

    /// X25519 with this identity's static key.
    pub fn dh(&self, peer_x_pk: &[u8; 32]) -> Result<Secret32> {
        prim::x25519(self.x_sk.expose(), peer_x_pk)
    }
}

/// What to do about a `node_id` collision between two distinct identities (§8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionOutcome {
    /// No collision: the `node_id` is free or already bound to this same `id_hash`.
    None,
    /// The new link must be rejected with `REJECT{8}`; the remote identity has the
    /// lexicographically larger `id_hash` and must regenerate.
    RejectPeerRegenerates,
    /// The new link must be rejected with `REJECT{8}`; my identity has the larger
    /// `id_hash`, so I regenerate my per-group identity and rejoin with the same invite.
    RejectSelfRegenerates,
    /// Two other identities collide; reject the newcomer (its owner resolves it).
    RejectNewcomer,
}

/// Which of two colliding identities regenerates: the lexicographically larger
/// `id_hash`. `None` when they are equal (same identity, no collision).
pub fn collision_loser(a: &[u8; 16], b: &[u8; 16]) -> Option<Ordering> {
    match a.cmp(b) {
        Ordering::Equal => None,
        o => Some(o),
    }
}

/// Deterministic `node_id` collision detector (§8.2). Remembers which `id_hash` owns
/// each `node_id` among identities seen through JOIN_REQ, IDPROOF or ROSTER.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeIdRegistry {
    owners: BTreeMap<u16, [u8; 16]>,
}

impl NodeIdRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The `id_hash` bound to `node_id`.
    pub fn owner(&self, node_id: u16) -> Option<&[u8; 16]> {
        self.owners.get(&node_id)
    }

    /// Records `id_hash` (its `node_id` is derived) unless that `node_id` is already
    /// bound to a different identity. `me` is this node's own `id_hash` in the group.
    pub fn observe(&mut self, id_hash: &[u8; 16], me: &[u8; 16]) -> CollisionOutcome {
        let node = node_id_of(id_hash);
        if node == node_id_of(me) && id_hash != me {
            return match collision_loser(me, id_hash) {
                Some(Ordering::Greater) => CollisionOutcome::RejectSelfRegenerates,
                _ => CollisionOutcome::RejectPeerRegenerates,
            };
        }
        match self.owners.get(&node) {
            None => {
                self.owners.insert(node, *id_hash);
                CollisionOutcome::None
            }
            Some(existing) if existing == id_hash => CollisionOutcome::None,
            Some(_) => CollisionOutcome::RejectNewcomer,
        }
    }

    /// Forgets an identity (it left, was denied, or regenerated).
    pub fn forget(&mut self, id_hash: &[u8; 16]) {
        let node = node_id_of(id_hash);
        if self.owners.get(&node) == Some(id_hash) {
            self.owners.remove(&node);
        }
    }
}

/// Fails with [`CryptoError::NodeIdCollision`] unless the outcome is `None`.
pub fn check_collision(outcome: CollisionOutcome) -> Result<()> {
    match outcome {
        CollisionOutcome::None => Ok(()),
        _ => Err(CryptoError::NodeIdCollision),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::SeedableRng;

    #[test]
    fn identities_are_independent_and_deterministic() {
        let mut rng = ChaCha20Rng::from_seed([7; 32]);
        let a = GroupIdentity::generate(&mut rng);
        let b = GroupIdentity::generate(&mut rng);
        assert_ne!(a.ed_pk(), b.ed_pk());
        assert_ne!(a.x_pk(), b.x_pk());
        assert_ne!(a.id_hash(), b.id_hash());
        // Ed25519 and X25519 keys of one identity are independent draws.
        assert_ne!(a.ed_pk(), a.x_pk());
        let mut rng2 = ChaCha20Rng::from_seed([7; 32]);
        assert_eq!(GroupIdentity::generate(&mut rng2), a);
        let restored = GroupIdentity::from_secret_bytes(&a.to_secret_bytes());
        assert_eq!(restored, a);
        assert_eq!(
            a.node_id(),
            u16::from_le_bytes([a.id_hash()[0], a.id_hash()[1]])
        );
    }

    #[test]
    fn signatures_verify_only_with_their_key() {
        let mut rng = ChaCha20Rng::from_seed([1; 32]);
        let a = GroupIdentity::generate(&mut rng);
        let b = GroupIdentity::generate(&mut rng);
        let sig = a.sign(b"msg");
        assert!(a.public().verify(b"msg", &sig).is_ok());
        assert!(b.public().verify(b"msg", &sig).is_err());
        assert_eq!(a.dh(&b.x_pk()).unwrap(), b.dh(&a.x_pk()).unwrap());
    }

    fn with_node(node: u16, tail: u8) -> [u8; 16] {
        let mut h = [tail; 16];
        h[..2].copy_from_slice(&node.to_le_bytes());
        h
    }

    #[test]
    fn collision_detection_is_deterministic() {
        let me = with_node(0x0101, 0x50);
        let mut reg = NodeIdRegistry::new();
        let p = with_node(0x2222, 1);
        assert_eq!(reg.observe(&p, &me), CollisionOutcome::None);
        assert_eq!(reg.observe(&p, &me), CollisionOutcome::None);
        let q = with_node(0x2222, 2);
        assert_eq!(reg.observe(&q, &me), CollisionOutcome::RejectNewcomer);
        // Colliding with me: the larger id_hash regenerates.
        let smaller = with_node(0x0101, 0x10);
        let larger = with_node(0x0101, 0x90);
        assert_eq!(
            reg.observe(&smaller, &me),
            CollisionOutcome::RejectSelfRegenerates
        );
        assert_eq!(
            reg.observe(&larger, &me),
            CollisionOutcome::RejectPeerRegenerates
        );
        // Both sides reach the same verdict.
        let mut other = NodeIdRegistry::new();
        assert_eq!(
            other.observe(&me, &smaller),
            CollisionOutcome::RejectPeerRegenerates
        );
        assert_eq!(collision_loser(&me, &me), None);
        reg.forget(&p);
        assert_eq!(reg.observe(&q, &me), CollisionOutcome::None);
        assert!(check_collision(CollisionOutcome::RejectNewcomer).is_err());
    }
}
