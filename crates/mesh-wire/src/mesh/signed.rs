//! Signed control payloads (DESIGN §4.6): LEAVE (65 B), RIDE_CTL (69 B), GROUP_CTL
//! (close 73 B / transfer-leader 107 B). This module builds the byte layouts and the
//! exact signed messages; signing and verification are mesh-crypto's (WP-07).

use super::voice::u8_enum;
use crate::codec::{PutLe, Reader};
use crate::error::{Result, WireError};

/// Ed25519 signature length.
pub const SIG_LEN: usize = 64;

u8_enum!(
    /// LEAVE `reason`.
    LeaveReason, "LEAVE reason" { User = 1, Removed = 2, AppExit = 3 }
);
u8_enum!(
    /// RIDE_CTL `op`.
    RideCtlOp, "RIDE_CTL op" { Start = 1, End = 2 }
);
u8_enum!(
    /// GROUP_CTL `op`.
    GroupCtlOp, "GROUP_CTL op" { Close = 1, TransferLeader = 2 }
);

/// 0x08 LEAVE: `reason u8, sig[64]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leave {
    pub reason: LeaveReason,
    pub sig: [u8; 64],
}

impl Leave {
    pub const LEN: usize = 65;
    pub const CONTEXT: &'static [u8] = b"cvic/v1/leave";

    /// Signed message: `"cvic/v1/leave" ‖ group_id ‖ origin u16 ‖ sess u8 ‖ reason u8`.
    pub fn signed_message(
        group_id: &[u8; 16],
        origin: u16,
        sess: u8,
        reason: LeaveReason,
    ) -> Vec<u8> {
        let mut m = Vec::with_capacity(Self::CONTEXT.len() + 20);
        m.put(Self::CONTEXT);
        m.put(group_id);
        m.put_u16(origin);
        m.put_u8(sess);
        m.put_u8(reason as u8);
        m
    }

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let v = Leave {
            reason: LeaveReason::from_u8(r.u8("LEAVE reason")?)?,
            sig: r.array("LEAVE sig")?,
        };
        r.finish("LEAVE")?;
        Ok(v)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u8(self.reason as u8);
        o.put(&self.sig);
        o
    }
}

/// 0x09 RIDE_CTL: `op u8, ride_id u32, sig[64]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RideCtl {
    pub op: RideCtlOp,
    pub ride_id: u32,
    pub sig: [u8; 64],
}

impl RideCtl {
    pub const LEN: usize = 69;
    pub const CONTEXT: &'static [u8] = b"cvic/v1/ridectl";

    /// Signed message: `"cvic/v1/ridectl" ‖ group_id ‖ epoch u32 ‖ ride_id u32 ‖ op u8`.
    pub fn signed_message(group_id: &[u8; 16], epoch: u32, ride_id: u32, op: RideCtlOp) -> Vec<u8> {
        let mut m = Vec::with_capacity(Self::CONTEXT.len() + 25);
        m.put(Self::CONTEXT);
        m.put(group_id);
        m.put_u32(epoch);
        m.put_u32(ride_id);
        m.put_u8(op as u8);
        m
    }

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let v = RideCtl {
            op: RideCtlOp::from_u8(r.u8("RIDE_CTL op")?)?,
            ride_id: r.u32("RIDE_CTL ride_id")?,
            sig: r.array("RIDE_CTL sig")?,
        };
        r.finish("RIDE_CTL")?;
        Ok(v)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u8(self.op as u8);
        o.put_u32(self.ride_id);
        o.put(&self.sig);
        o
    }
}

/// 0x0C GROUP_CTL: `op u8, ctl_version u32, epoch u32, [new_leader_ed_pk[32],
/// new_leader_node u16 if op 2], sig[64]`, signed by the current leader over
/// `"cvic/v1/groupctl" ‖ group_id ‖ preceding bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupCtl {
    pub op: GroupCtlOp,
    pub ctl_version: u32,
    pub epoch: u32,
    /// `Some((new_leader_ed_pk, new_leader_node))` exactly when `op` is TransferLeader.
    pub new_leader: Option<([u8; 32], u16)>,
    pub sig: [u8; 64],
}

impl GroupCtl {
    pub const LEN_CLOSE: usize = 73;
    pub const LEN_TRANSFER: usize = 107;
    pub const CONTEXT: &'static [u8] = b"cvic/v1/groupctl";

    /// The bytes before `sig` (the "preceding bytes" of the signature).
    pub fn unsigned_bytes(&self) -> Result<Vec<u8>> {
        let mut o = Vec::with_capacity(Self::LEN_TRANSFER - SIG_LEN);
        o.put_u8(self.op as u8);
        o.put_u32(self.ctl_version);
        o.put_u32(self.epoch);
        match (self.op, self.new_leader) {
            (GroupCtlOp::Close, None) => {}
            (GroupCtlOp::TransferLeader, Some((pk, node))) => {
                o.put(&pk);
                o.put_u16(node);
            }
            _ => {
                return Err(WireError::BadValue {
                    what: "GROUP_CTL new_leader for op",
                    value: self.op as u32,
                });
            }
        }
        Ok(o)
    }

    /// Signed message: `"cvic/v1/groupctl" ‖ group_id ‖ unsigned_bytes`.
    pub fn signed_message(&self, group_id: &[u8; 16]) -> Result<Vec<u8>> {
        let mut m = Vec::with_capacity(Self::CONTEXT.len() + 16 + 43);
        m.put(Self::CONTEXT);
        m.put(group_id);
        m.put(&self.unsigned_bytes()?);
        Ok(m)
    }

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let op = GroupCtlOp::from_u8(r.u8("GROUP_CTL op")?)?;
        let ctl_version = r.u32("GROUP_CTL ctl_version")?;
        let epoch = r.u32("GROUP_CTL epoch")?;
        let new_leader = match op {
            GroupCtlOp::Close => None,
            GroupCtlOp::TransferLeader => {
                Some((r.array("new_leader_ed_pk")?, r.u16("new_leader_node")?))
            }
        };
        let sig = r.array("GROUP_CTL sig")?;
        r.finish("GROUP_CTL")?;
        Ok(GroupCtl {
            op,
            ctl_version,
            epoch,
            new_leader,
            sig,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut o = self.unsigned_bytes()?;
        o.put(&self.sig);
        Ok(o)
    }
}
