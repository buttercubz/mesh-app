//! Origin-signed LEAVE and RIDE_CTL (DESIGN §4.6, §8.1). Both are signed by their
//! origin's per-group Ed25519 key and ignored if the signer key is unknown or the
//! signature fails.
//!
//! - LEAVE: `reason u8, sig[64]` over `"cvic/v1/leave" ‖ group_id ‖ origin ‖ sess ‖ reason`.
//! - RIDE_CTL: `op u8, ride_id u32, sig[64]` over
//!   `"cvic/v1/ridectl" ‖ group_id ‖ epoch ‖ ride_id ‖ op`.

use mesh_wire::mesh::{Leave, LeaveReason, RideCtl, RideCtlOp, Roster};

use crate::error::{CryptoError, Result};
use crate::identity::GroupIdentity;
use crate::prim;
use crate::roster;

/// A LEAVE payload together with the header fields its signature covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedLeave {
    /// Origin `node_id` (mesh header).
    pub origin: u16,
    /// Origin session tag (mesh header).
    pub sess: u8,
    pub leave: Leave,
}

impl SignedLeave {
    /// Signs a LEAVE for my origin and session.
    pub fn sign(me: &GroupIdentity, group_id: &[u8; 16], sess: u8, reason: LeaveReason) -> Self {
        let origin = me.node_id();
        let msg = Leave::signed_message(group_id, origin, sess, reason);
        SignedLeave {
            origin,
            sess,
            leave: Leave {
                reason,
                sig: me.sign(&msg),
            },
        }
    }

    /// Verifies with the origin's Ed25519 key.
    pub fn verify(&self, group_id: &[u8; 16], origin_ed_pk: &[u8; 32]) -> Result<()> {
        let msg = Leave::signed_message(group_id, self.origin, self.sess, self.leave.reason);
        prim::ed25519_verify(origin_ed_pk, &msg, &self.leave.sig)
    }

    /// Verifies with the key the roster lists for `origin`; an unknown signer fails.
    pub fn verify_with_roster(&self, roster: &Roster) -> Result<()> {
        let pk = roster::ed_pk_of(roster, self.origin).ok_or(CryptoError::BadSignature)?;
        self.verify(&roster.group_id, &pk)
    }
}

/// A RIDE_CTL payload with its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedRideCtl {
    pub origin: u16,
    pub ctl: RideCtl,
}

impl SignedRideCtl {
    /// Signs a RIDE_CTL (`ride_id` is random per Start ride, drawn by the caller).
    pub fn sign(
        me: &GroupIdentity,
        group_id: &[u8; 16],
        epoch: u32,
        ride_id: u32,
        op: RideCtlOp,
    ) -> Self {
        let msg = RideCtl::signed_message(group_id, epoch, ride_id, op);
        SignedRideCtl {
            origin: me.node_id(),
            ctl: RideCtl {
                op,
                ride_id,
                sig: me.sign(&msg),
            },
        }
    }

    /// Verifies with the origin's key for the given group epoch.
    pub fn verify(&self, group_id: &[u8; 16], epoch: u32, origin_ed_pk: &[u8; 32]) -> Result<()> {
        let msg = RideCtl::signed_message(group_id, epoch, self.ctl.ride_id, self.ctl.op);
        prim::ed25519_verify(origin_ed_pk, &msg, &self.ctl.sig)
    }

    /// Verifies with the key the roster lists for `origin`.
    pub fn verify_with_roster(&self, roster: &Roster, epoch: u32) -> Result<()> {
        let pk = roster::ed_pk_of(roster, self.origin).ok_or(CryptoError::BadSignature)?;
        self.verify(&roster.group_id, epoch, &pk)
    }

    /// `end` applies only to the current ride.
    pub fn ends_ride(&self, current_ride: Option<u32>) -> bool {
        self.ctl.op == RideCtlOp::End && current_ride == Some(self.ctl.ride_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invite::InviteSig;
    use crate::roster::{RosterRole, entry_for_identity};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn leave_and_ride_ctl_signatures() {
        let mut rng = ChaCha20Rng::from_seed([61; 32]);
        let leader = GroupIdentity::generate(&mut rng);
        let a = GroupIdentity::generate(&mut rng);
        let b = GroupIdentity::generate(&mut rng);
        let gid = [6; 16];
        let r = roster::build(
            &leader,
            gid,
            2,
            1,
            InviteSig([0; 64]),
            vec![
                entry_for_identity(leader.public(), RosterRole::Leader, "L"),
                entry_for_identity(a.public(), RosterRole::Member, "A"),
            ],
        )
        .unwrap();

        let l = SignedLeave::sign(&a, &gid, 0x33, LeaveReason::User);
        assert_eq!(l.leave.encode().len(), Leave::LEN);
        assert!(l.verify(&gid, &a.ed_pk()).is_ok());
        assert!(l.verify_with_roster(&r).is_ok());
        assert!(l.verify(&gid, &b.ed_pk()).is_err());
        let mut forged = l;
        forged.sess = 0x34;
        assert!(forged.verify_with_roster(&r).is_err());
        let mut other = l;
        other.origin = leader.node_id();
        assert!(
            other.verify_with_roster(&r).is_err(),
            "cannot speak for another origin"
        );
        let unknown = SignedLeave::sign(&b, &gid, 1, LeaveReason::AppExit);
        assert!(
            unknown.verify_with_roster(&r).is_err(),
            "unknown signer is ignored"
        );

        let s = SignedRideCtl::sign(&leader, &gid, 2, 0xDEAD_BEEF, RideCtlOp::Start);
        assert_eq!(s.ctl.encode().len(), RideCtl::LEN);
        assert!(s.verify_with_roster(&r, 2).is_ok());
        assert!(s.verify_with_roster(&r, 3).is_err(), "bound to the epoch");
        let e = SignedRideCtl::sign(&leader, &gid, 2, 0xDEAD_BEEF, RideCtlOp::End);
        assert!(e.ends_ride(Some(0xDEAD_BEEF)));
        assert!(!e.ends_ride(Some(1)));
        assert!(!s.ends_ride(Some(0xDEAD_BEEF)));
    }
}
