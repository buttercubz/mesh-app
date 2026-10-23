//! Control message bodies (DESIGN §4.7), logical form before segmentation: ROSTER,
//! JOIN_REQ and REKEY. This module lays out the bytes and the exact signed messages;
//! signing, verification, sealing and the monotonic acceptance rules are mesh-crypto's
//! (WP-07).

use super::signed::SIG_LEN;
use super::voice::u8_enum;
use crate::codec::{PutLe, Reader, check_max};
use crate::error::{Result, WireError};
use crate::mesh::segment::MAX_LOGICAL_BODY;
use crate::name::{DISPLAY_NAME_MAX, get_name, put_name};

/// Version byte of ROSTER and JOIN_REQ bodies.
pub const CONTROL_BODY_VERSION: u8 = 1;
/// Roster members with an index (non-denied entries), bitmap `heard u32`.
pub const MAX_ROSTER_MEMBERS: usize = 32;
/// REKEY sealed entry: ChaCha20-Poly1305 of a 32-byte epoch secret.
pub const SEALED_LEN: usize = 48;

u8_enum!(
    /// ROSTER entry role.
    RosterRole, "roster role" { Member = 0, Leader = 1, Denied = 0xFF }
);

/// One ROSTER entry: `node_id u16 · id_hash[16] · ed_pk[32] · role u8 · name_len u8 ·
/// name[≤ 24 B]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub node_id: u16,
    pub id_hash: [u8; 16],
    pub ed_pk: [u8; 32],
    pub role: RosterRole,
    /// Display name; encoders truncate it to 24 bytes at a grapheme boundary.
    pub name: String,
}

/// ROSTER: `ver u8=1 · group_id[16] · epoch u32 · roster_version u32 · invite_sig[64] ·
/// n u8 · n × entry · sig[64]`; the leader signs `"cvic/v1/roster" ‖ preceding bytes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roster {
    pub group_id: [u8; 16],
    pub epoch: u32,
    pub roster_version: u32,
    pub invite_sig: [u8; 64],
    pub entries: Vec<RosterEntry>,
    pub sig: [u8; 64],
}

impl Roster {
    pub const CONTEXT: &'static [u8] = b"cvic/v1/roster";

    /// The bytes covered by `sig` (everything before it).
    pub fn unsigned_bytes(&self) -> Result<Vec<u8>> {
        check_max("ROSTER entries", self.entries.len(), usize::from(u8::MAX))?;
        let members = self
            .entries
            .iter()
            .filter(|e| e.role != RosterRole::Denied)
            .count();
        check_max("ROSTER members", members, MAX_ROSTER_MEMBERS)?;
        let mut o = Vec::with_capacity(94 + self.entries.len() * 76);
        o.put_u8(CONTROL_BODY_VERSION);
        o.put(&self.group_id);
        o.put_u32(self.epoch);
        o.put_u32(self.roster_version);
        o.put(&self.invite_sig);
        o.put_u8(self.entries.len() as u8);
        for e in &self.entries {
            o.put_u16(e.node_id);
            o.put(&e.id_hash);
            o.put(&e.ed_pk);
            o.put_u8(e.role as u8);
            put_name(&mut o, &e.name, DISPLAY_NAME_MAX);
        }
        check_max("ROSTER body", o.len() + SIG_LEN, MAX_LOGICAL_BODY)?;
        Ok(o)
    }

    /// `"cvic/v1/roster" ‖ unsigned_bytes`.
    pub fn signed_message(&self) -> Result<Vec<u8>> {
        let mut m = Self::CONTEXT.to_vec();
        m.put(&self.unsigned_bytes()?);
        Ok(m)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut o = self.unsigned_bytes()?;
        o.put(&self.sig);
        Ok(o)
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        check_max("ROSTER body", b.len(), MAX_LOGICAL_BODY)?;
        let mut r = Reader::new(b);
        expect_version(&mut r, "ROSTER ver")?;
        let group_id = r.array("ROSTER group_id")?;
        let epoch = r.u32("ROSTER epoch")?;
        let roster_version = r.u32("ROSTER roster_version")?;
        let invite_sig = r.array("ROSTER invite_sig")?;
        let n = r.u8("ROSTER n")?;
        let mut entries = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            entries.push(RosterEntry {
                node_id: r.u16("ROSTER node_id")?,
                id_hash: r.array("ROSTER id_hash")?,
                ed_pk: r.array("ROSTER ed_pk")?,
                role: RosterRole::from_u8(r.u8("ROSTER role")?)?,
                name: get_name(&mut r, DISPLAY_NAME_MAX)?.to_owned(),
            });
        }
        let members = entries
            .iter()
            .filter(|e| e.role != RosterRole::Denied)
            .count();
        check_max("ROSTER members", members, MAX_ROSTER_MEMBERS)?;
        let sig = r.array("ROSTER sig")?;
        r.finish("ROSTER")?;
        Ok(Roster {
            group_id,
            epoch,
            roster_version,
            invite_sig,
            entries,
            sig,
        })
    }

    /// Roster index of `node_id`: its position among non-denied entries (0-31).
    pub fn index_of(&self, node_id: u16) -> Option<u8> {
        self.entries
            .iter()
            .filter(|e| e.role != RosterRole::Denied)
            .position(|e| e.node_id == node_id)
            .map(|i| i as u8)
    }

    /// `true` when `id_hash` is listed as denied.
    pub fn is_denied(&self, id_hash: &[u8; 16]) -> bool {
        self.entries
            .iter()
            .any(|e| e.role == RosterRole::Denied && &e.id_hash == id_hash)
    }
}

/// JOIN_REQ: `ver u8=1 · ed_pk[32] · x_pk[32] · platform u8 · name_len u8 ·
/// name[≤ 24 B] · sig[64]`, signed over `"cvic/v1/join" ‖ group_id ‖ preceding bytes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinReq {
    pub ed_pk: [u8; 32],
    pub x_pk: [u8; 32],
    /// 0 ios, 1 android, 2 websim (as CAPS).
    pub platform: u8,
    pub name: String,
    pub sig: [u8; 64],
}

impl JoinReq {
    pub const CONTEXT: &'static [u8] = b"cvic/v1/join";

    pub fn unsigned_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(67 + DISPLAY_NAME_MAX);
        o.put_u8(CONTROL_BODY_VERSION);
        o.put(&self.ed_pk);
        o.put(&self.x_pk);
        o.put_u8(self.platform);
        put_name(&mut o, &self.name, DISPLAY_NAME_MAX);
        o
    }

    /// `"cvic/v1/join" ‖ group_id ‖ unsigned_bytes`.
    pub fn signed_message(&self, group_id: &[u8; 16]) -> Vec<u8> {
        let mut m = Self::CONTEXT.to_vec();
        m.put(group_id);
        m.put(&self.unsigned_bytes());
        m
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = self.unsigned_bytes();
        o.put(&self.sig);
        o
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        expect_version(&mut r, "JOIN_REQ ver")?;
        let v = JoinReq {
            ed_pk: r.array("JOIN_REQ ed_pk")?,
            x_pk: r.array("JOIN_REQ x_pk")?,
            platform: r.u8("JOIN_REQ platform")?,
            name: get_name(&mut r, DISPLAY_NAME_MAX)?.to_owned(),
            sig: r.array("JOIN_REQ sig")?,
        };
        r.finish("JOIN_REQ")?;
        Ok(v)
    }
}

/// One REKEY entry: `node_id u16 · sealed[48]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RekeyEntry {
    pub node_id: u16,
    pub sealed: [u8; SEALED_LEN],
}

/// REKEY: `new_epoch u32 · roster_version u32 · leader_eph_pk[32] · n u8 · n × entry ·
/// sig[64]`. The leader signs `"cvic/v1/rekey" ‖ group_id ‖ preceding bytes` (same
/// domain-separation pattern as GROUP_CTL; §4.7 leaves the context implicit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rekey {
    pub new_epoch: u32,
    pub roster_version: u32,
    pub leader_eph_pk: [u8; 32],
    pub entries: Vec<RekeyEntry>,
    pub sig: [u8; 64],
}

impl Rekey {
    pub const CONTEXT: &'static [u8] = b"cvic/v1/rekey";
    /// At most one sealed entry per roster member.
    pub const MAX_ENTRIES: usize = MAX_ROSTER_MEMBERS;

    pub fn unsigned_bytes(&self) -> Result<Vec<u8>> {
        check_max("REKEY entries", self.entries.len(), Self::MAX_ENTRIES)?;
        let mut o = Vec::with_capacity(41 + self.entries.len() * 50);
        o.put_u32(self.new_epoch);
        o.put_u32(self.roster_version);
        o.put(&self.leader_eph_pk);
        o.put_u8(self.entries.len() as u8);
        for e in &self.entries {
            o.put_u16(e.node_id);
            o.put(&e.sealed);
        }
        Ok(o)
    }

    /// `"cvic/v1/rekey" ‖ group_id ‖ unsigned_bytes`.
    pub fn signed_message(&self, group_id: &[u8; 16]) -> Result<Vec<u8>> {
        let mut m = Self::CONTEXT.to_vec();
        m.put(group_id);
        m.put(&self.unsigned_bytes()?);
        Ok(m)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut o = self.unsigned_bytes()?;
        o.put(&self.sig);
        Ok(o)
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let new_epoch = r.u32("REKEY new_epoch")?;
        let roster_version = r.u32("REKEY roster_version")?;
        let leader_eph_pk = r.array("REKEY leader_eph_pk")?;
        let n = r.u8("REKEY n")?;
        check_max("REKEY entries", usize::from(n), Self::MAX_ENTRIES)?;
        let mut entries = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            entries.push(RekeyEntry {
                node_id: r.u16("REKEY node_id")?,
                sealed: r.array("REKEY sealed")?,
            });
        }
        let sig = r.array("REKEY sig")?;
        r.finish("REKEY")?;
        Ok(Rekey {
            new_epoch,
            roster_version,
            leader_eph_pk,
            entries,
            sig,
        })
    }

    /// The sealed secret addressed to `node_id`.
    pub fn entry_for(&self, node_id: u16) -> Option<&[u8; SEALED_LEN]> {
        self.entries
            .iter()
            .find(|e| e.node_id == node_id)
            .map(|e| &e.sealed)
    }
}

fn expect_version(r: &mut Reader<'_>, what: &'static str) -> Result<()> {
    let v = r.u8(what)?;
    if v != CONTROL_BODY_VERSION {
        return Err(WireError::BadValue {
            what,
            value: u32::from(v),
        });
    }
    Ok(())
}
