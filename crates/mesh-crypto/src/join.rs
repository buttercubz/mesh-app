//! JOIN_REQ signing and verification (DESIGN §4.7): `ver u8=1 · ed_pk[32] · x_pk[32] ·
//! platform u8 · name_len u8 · name[≤ 24 B] · sig[64]`, Ed25519 by the joiner's
//! per-group key over `"cvic/v1/join" ‖ group_id ‖ preceding bytes`.
//!
//! The leader treats JOIN_REQs idempotently, keyed by `id_hash` ([`JoinBook`]).

use std::collections::BTreeMap;

pub use mesh_wire::mesh::JoinReq;

use crate::error::Result;
use crate::identity::{GroupIdentity, PublicIdentity};

/// Builds and signs my JOIN_REQ.
pub fn create(identity: &GroupIdentity, group_id: &[u8; 16], platform: u8, name: &str) -> JoinReq {
    let mut j = JoinReq {
        ed_pk: identity.ed_pk(),
        x_pk: identity.x_pk(),
        platform,
        name: mesh_wire::name::truncate_graphemes(name, mesh_wire::name::DISPLAY_NAME_MAX)
            .to_owned(),
        sig: [0; 64],
    };
    j.sig = identity.sign(&j.signed_message(group_id));
    j
}

/// Verifies a JOIN_REQ for `group_id`; returns the joiner's public identity (its
/// `id_hash` binds `ed_pk` and `x_pk`).
pub fn verify(join: &JoinReq, group_id: &[u8; 16]) -> Result<PublicIdentity> {
    let id = PublicIdentity {
        ed_pk: join.ed_pk,
        x_pk: join.x_pk,
    };
    id.verify(&join.signed_message(group_id), &join.sig)?;
    Ok(id)
}

/// A verified join request as the leader keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedJoin {
    pub identity: PublicIdentity,
    pub platform: u8,
    pub name: String,
}

/// The leader's idempotent JOIN_REQ book, keyed by `id_hash`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JoinBook {
    joins: BTreeMap<[u8; 16], VerifiedJoin>,
}

impl JoinBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Verifies and records a JOIN_REQ body. Returns the joiner and `true` when it is new
    /// or its name/platform changed (the leader then issues the next roster_version).
    pub fn accept(&mut self, body: &[u8], group_id: &[u8; 16]) -> Result<(PublicIdentity, bool)> {
        let j = JoinReq::decode(body)?;
        let identity = verify(&j, group_id)?;
        let v = VerifiedJoin {
            identity,
            platform: j.platform,
            name: j.name,
        };
        let changed = self.joins.get(&identity.id_hash()) != Some(&v);
        if changed {
            self.joins.insert(identity.id_hash(), v);
        }
        Ok((identity, changed))
    }

    pub fn get(&self, id_hash: &[u8; 16]) -> Option<&VerifiedJoin> {
        self.joins.get(id_hash)
    }

    /// X25519 key of a joiner (REKEY recipient).
    pub fn x_pk(&self, id_hash: &[u8; 16]) -> Option<[u8; 32]> {
        self.joins.get(id_hash).map(|j| j.identity.x_pk)
    }

    pub fn iter(&self) -> impl Iterator<Item = &VerifiedJoin> {
        self.joins.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CryptoError;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn sign_verify_and_idempotent_book() {
        let mut rng = ChaCha20Rng::from_seed([21; 32]);
        let me = GroupIdentity::generate(&mut rng);
        let gid = [3; 16];
        let j = create(&me, &gid, 0, "Ana");
        assert_eq!(verify(&j, &gid).unwrap().id_hash(), me.id_hash());
        assert_eq!(verify(&j, &[4; 16]), Err(CryptoError::BadSignature));
        let mut bad = j.clone();
        bad.name = "Eve".into();
        assert!(verify(&bad, &gid).is_err());
        let mut swapped = j.clone();
        swapped.x_pk = [9; 32];
        assert!(verify(&swapped, &gid).is_err());

        let mut book = JoinBook::new();
        let (id, new) = book.accept(&j.encode(), &gid).unwrap();
        assert!(new);
        assert_eq!(id, *me.public());
        assert!(
            !book.accept(&j.encode(), &gid).unwrap().1,
            "duplicate is idempotent"
        );
        let renamed = create(&me, &gid, 0, "Ana M");
        assert!(book.accept(&renamed.encode(), &gid).unwrap().1);
        assert_eq!(book.iter().count(), 1);
        assert_eq!(book.x_pk(&me.id_hash()), Some(me.x_pk()));
        assert!(book.accept(&bad.encode(), &gid).is_err());
    }
}
