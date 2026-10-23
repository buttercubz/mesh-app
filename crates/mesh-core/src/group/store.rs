//! Persistence of groups (DESIGN §3.2 `StoredGroup`, WP-10B interface): one opaque blob
//! per group under `group/<hex id>` (identity, retained epoch secrets, the latest ROSTER /
//! REKEY / GROUP_CTL bodies, the leader key, the verified-joiner book) and the list of
//! group ids under `groups/index`. The runtime keeps the bytes in Keychain/Keystore and
//! hands them back in `EngineConfig::stored_groups`.
//!
//! Layout (little-endian), tag 1 = live group:
//! `tag u8 · ver u8 · group_id[16] · name (u8 len + bytes) · vehicle u8 · policy u8 ·
//! flags u8 (b0 unverified leader, b1 aware pairing, b2 expiry, b3 pending transfer) ·
//! [expires_at u32] · display_name (u8 len + bytes) · identity[64] · invite_leader[32] ·
//! leader_pk[32] · epoch u32 · [transfer pk[32] · node u16] · n u8 · n × (epoch u32 ·
//! secret[32]) · roster (u16 len + body) · rekey (u16 len + body) · group_ctl (u16 len +
//! body) · m u8 · m × (ed_pk[32] · x_pk[32] · platform u8 · name (u8 len + bytes))`.
//! Tag 2 = closed group: `tag · ver · group_id[16] · name · vehicle · role u8 ·
//! members u8 · epoch u32` (no secrets).

use mesh_api::{MemberRole, Vehicle};
use mesh_crypto::invite::InvitePolicy;
use mesh_crypto::{EpochKeyring, GroupIdentity, KeySchedule, PublicIdentity, Secret32};
use mesh_wire::record::SyncKind;

use super::control::Control;
use super::state::{ClosedGroup, Group, Joiner};

const TAG_LIVE: u8 = 1;
const TAG_CLOSED: u8 = 2;
const VERSION: u8 = 1;

/// Persist key of a group.
pub fn group_key(id: &[u8; 16]) -> String {
    format!("group/{}", hex(id))
}

/// Persist key of the group index.
pub const INDEX_KEY: &str = "groups/index";

/// Lowercase hex of a group id (the UI's `groupId`).
pub fn hex(id: &[u8; 16]) -> String {
    mesh_wire::codec::to_hex(id)
}

/// Parses a UI `groupId`.
pub fn parse_hex(s: &str) -> Option<[u8; 16]> {
    let v = mesh_wire::codec::from_hex(s)?;
    v.try_into().ok()
}

/// A decoded blob.
#[derive(Debug, Clone)]
pub enum Stored {
    Live(Box<Group>),
    Closed(ClosedGroup),
}

/// Why a blob could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadBlob(pub &'static str);

struct Cur<'a> {
    b: &'a [u8],
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], BadBlob> {
        if self.b.len() < n {
            return Err(BadBlob(what));
        }
        let (h, t) = self.b.split_at(n);
        self.b = t;
        Ok(h)
    }
    fn u8(&mut self, w: &'static str) -> Result<u8, BadBlob> {
        Ok(self.take(1, w)?[0])
    }
    fn u16(&mut self, w: &'static str) -> Result<u16, BadBlob> {
        let s = self.take(2, w)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self, w: &'static str) -> Result<u32, BadBlob> {
        let s = self.take(4, w)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn arr<const N: usize>(&mut self, w: &'static str) -> Result<[u8; N], BadBlob> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N, w)?);
        Ok(a)
    }
    fn name(&mut self, w: &'static str) -> Result<String, BadBlob> {
        let n = usize::from(self.u8(w)?);
        let s = self.take(n, w)?;
        String::from_utf8(s.to_vec()).map_err(|_| BadBlob(w))
    }
    fn body(&mut self, w: &'static str) -> Result<Option<&'a [u8]>, BadBlob> {
        let n = usize::from(self.u16(w)?);
        Ok((n > 0).then_some(self.take(n, w)?))
    }
}

fn put_name(o: &mut Vec<u8>, s: &str) {
    let s = mesh_wire::name::truncate_graphemes(s, 255);
    o.push(s.len() as u8);
    o.extend_from_slice(s.as_bytes());
}

fn put_body(o: &mut Vec<u8>, b: Option<&[u8]>) {
    let b = b.unwrap_or(&[]);
    o.extend_from_slice(&(b.len().min(usize::from(u16::MAX)) as u16).to_le_bytes());
    o.extend_from_slice(&b[..b.len().min(usize::from(u16::MAX))]);
}

fn vehicle_byte(v: Vehicle) -> u8 {
    match v {
        Vehicle::Moto => 0,
        Vehicle::Car => 1,
    }
}

fn vehicle_of(b: u8) -> Vehicle {
    if b == 1 { Vehicle::Car } else { Vehicle::Moto }
}

/// Encodes a live group.
pub fn encode(g: &Group) -> Vec<u8> {
    let mut o = Vec::with_capacity(512);
    o.extend_from_slice(&[TAG_LIVE, VERSION]);
    o.extend_from_slice(&g.id);
    put_name(&mut o, &g.name);
    o.push(vehicle_byte(g.vehicle));
    o.push(g.policy.0);
    let transfer = g.control.pending_transfer();
    let flags = u8::from(g.unverified_leader)
        | (u8::from(g.aware_pairing) << 1)
        | (u8::from(g.expires_at.is_some()) << 2)
        | (u8::from(transfer.is_some()) << 3);
    o.push(flags);
    if let Some(e) = g.expires_at {
        o.extend_from_slice(&e.to_le_bytes());
    }
    put_name(&mut o, &g.display_name);
    o.extend_from_slice(&g.identity.to_secret_bytes());
    o.extend_from_slice(&g.invite_leader);
    o.extend_from_slice(g.control.leader_pk());
    o.extend_from_slice(&g.control.epoch().to_le_bytes());
    if let Some((pk, node)) = transfer {
        o.extend_from_slice(&pk);
        o.extend_from_slice(&node.to_le_bytes());
    }
    o.push(g.keyring.len() as u8);
    for s in g.keyring.iter() {
        o.extend_from_slice(&s.epoch().to_le_bytes());
        o.extend_from_slice(s.epoch_secret().expose());
    }
    for k in [SyncKind::Roster, SyncKind::Rekey, SyncKind::GroupCtl] {
        put_body(&mut o, g.control.body(k));
    }
    let book: Vec<&Joiner> = g.book.values().take(255).collect();
    o.push(book.len() as u8);
    for j in book {
        o.extend_from_slice(&j.identity.ed_pk);
        o.extend_from_slice(&j.identity.x_pk);
        o.push(j.platform);
        put_name(&mut o, &j.name);
    }
    o
}

/// Encodes a closed-group tombstone.
pub fn encode_closed(c: &ClosedGroup) -> Vec<u8> {
    let mut o = vec![TAG_CLOSED, VERSION];
    o.extend_from_slice(&c.id);
    put_name(&mut o, &c.name);
    o.push(vehicle_byte(c.vehicle));
    o.push(u8::from(c.role == MemberRole::Leader));
    o.push(c.members);
    o.extend_from_slice(&c.epoch.to_le_bytes());
    o
}

/// Decodes a blob written by [`encode`] or [`encode_closed`].
pub fn decode(blob: &[u8]) -> Result<Stored, BadBlob> {
    let mut c = Cur { b: blob };
    let tag = c.u8("tag")?;
    if c.u8("version")? != VERSION {
        return Err(BadBlob("version"));
    }
    let id: [u8; 16] = c.arr("group_id")?;
    let name = c.name("name")?;
    let vehicle = vehicle_of(c.u8("vehicle")?);
    if tag == TAG_CLOSED {
        let role = if c.u8("role")? == 1 {
            MemberRole::Leader
        } else {
            MemberRole::Member
        };
        return Ok(Stored::Closed(ClosedGroup {
            id,
            name,
            vehicle,
            role,
            members: c.u8("members")?,
            epoch: c.u32("epoch")?,
        }));
    }
    if tag != TAG_LIVE {
        return Err(BadBlob("tag"));
    }
    let policy = InvitePolicy(c.u8("policy")?);
    let flags = c.u8("flags")?;
    let expires_at = if flags & 4 != 0 {
        Some(c.u32("expires_at")?)
    } else {
        None
    };
    let display_name = c.name("display_name")?;
    let identity = GroupIdentity::from_secret_bytes(&c.arr("identity")?);
    let invite_leader: [u8; 32] = c.arr("invite_leader")?;
    let leader_pk: [u8; 32] = c.arr("leader_pk")?;
    let epoch = c.u32("epoch")?;
    let transfer = if flags & 8 != 0 {
        Some((c.arr("transfer pk")?, c.u16("transfer node")?))
    } else {
        None
    };
    let n = c.u8("epochs")?;
    let mut keyring: Option<EpochKeyring> = None;
    for _ in 0..n {
        let e = c.u32("epoch")?;
        let s = KeySchedule::new(id, e, Secret32::new(c.arr("secret")?));
        match keyring.as_mut() {
            Some(k) => {
                k.insert(s);
            }
            None => keyring = Some(EpochKeyring::new(s)),
        }
    }
    let keyring = keyring.ok_or(BadBlob("no epoch"))?;
    let roster = c.body("roster")?;
    let rekey = c.body("rekey")?;
    let ctl = c.body("group_ctl")?;
    let control = Control::restore(id, leader_pk, epoch, [roster, rekey, ctl], transfer);
    let m = c.u8("book")?;
    let mut book = std::collections::BTreeMap::new();
    for _ in 0..m {
        let identity = PublicIdentity {
            ed_pk: c.arr("ed_pk")?,
            x_pk: c.arr("x_pk")?,
        };
        let platform = c.u8("platform")?;
        let name = c.name("joiner name")?;
        book.insert(
            identity.id_hash(),
            Joiner {
                identity,
                platform,
                name,
            },
        );
    }
    Ok(Stored::Live(Box::new(Group {
        id,
        name,
        vehicle,
        policy,
        display_name,
        identity,
        keyring,
        control,
        invite_leader,
        unverified_leader: flags & 1 != 0,
        book,
        expires_at,
        aware_pairing: flags & 2 != 0,
    })))
}

/// `groups/index`: `n u8 · n × group_id[16]`.
pub fn encode_index<'a>(ids: impl Iterator<Item = &'a [u8; 16]>) -> Vec<u8> {
    let ids: Vec<&[u8; 16]> = ids.take(255).collect();
    let mut o = vec![ids.len() as u8];
    for id in ids {
        o.extend_from_slice(id);
    }
    o
}
