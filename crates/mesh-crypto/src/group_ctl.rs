//! GROUP_CTL signing and verification (DESIGN §4.6, §8.6): `op u8 (1 close-group,
//! 2 transfer-leader) · ctl_version u32 · epoch u32 · [new_leader_ed_pk[32] ·
//! new_leader_node u16 if op 2] · sig[64]`, signed by the current leader over
//! `"cvic/v1/groupctl" ‖ group_id ‖ preceding bytes`.

pub use mesh_wire::mesh::{GroupCtl, GroupCtlOp};

use crate::error::Result;
use crate::identity::GroupIdentity;
use crate::prim;

fn signed(leader: &GroupIdentity, group_id: &[u8; 16], mut ctl: GroupCtl) -> Result<GroupCtl> {
    ctl.sig = leader.sign(&ctl.signed_message(group_id)?);
    Ok(ctl)
}

/// "Close group": members mark the group closed and delete its secrets.
pub fn close(
    leader: &GroupIdentity,
    group_id: &[u8; 16],
    ctl_version: u32,
    epoch: u32,
) -> Result<GroupCtl> {
    signed(
        leader,
        group_id,
        GroupCtl {
            op: GroupCtlOp::Close,
            ctl_version,
            epoch,
            new_leader: None,
            sig: [0; 64],
        },
    )
}

/// "Transfer leadership" to a present roster member (followed by a ROSTER, version + 1,
/// with the roles swapped, signed by the old key).
pub fn transfer_leader(
    leader: &GroupIdentity,
    group_id: &[u8; 16],
    ctl_version: u32,
    epoch: u32,
    new_leader_ed_pk: [u8; 32],
    new_leader_node: u16,
) -> Result<GroupCtl> {
    signed(
        leader,
        group_id,
        GroupCtl {
            op: GroupCtlOp::TransferLeader,
            ctl_version,
            epoch,
            new_leader: Some((new_leader_ed_pk, new_leader_node)),
            sig: [0; 64],
        },
    )
}

/// Verifies the current leader's signature.
pub fn verify(ctl: &GroupCtl, group_id: &[u8; 16], leader_ed_pk: &[u8; 32]) -> Result<()> {
    prim::ed25519_verify(leader_ed_pk, &ctl.signed_message(group_id)?, &ctl.sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn close_and_transfer_sign_and_verify() {
        let mut rng = ChaCha20Rng::from_seed([51; 32]);
        let leader = GroupIdentity::generate(&mut rng);
        let next = GroupIdentity::generate(&mut rng);
        let gid = [8; 16];
        let c = close(&leader, &gid, 1, 0).unwrap();
        assert_eq!(c.encode().unwrap().len(), GroupCtl::LEN_CLOSE);
        assert!(verify(&c, &gid, &leader.ed_pk()).is_ok());
        assert!(verify(&c, &[9; 16], &leader.ed_pk()).is_err());
        assert!(verify(&c, &gid, &next.ed_pk()).is_err());
        let t = transfer_leader(&leader, &gid, 2, 0, next.ed_pk(), next.node_id()).unwrap();
        let wire = GroupCtl::decode(&t.encode().unwrap()).unwrap();
        assert_eq!(t.encode().unwrap().len(), GroupCtl::LEN_TRANSFER);
        assert!(verify(&wire, &gid, &leader.ed_pk()).is_ok());
        let mut forged = wire;
        forged.new_leader = Some((leader.ed_pk(), leader.node_id()));
        assert!(verify(&forged, &gid, &leader.ed_pk()).is_err());
    }
}
