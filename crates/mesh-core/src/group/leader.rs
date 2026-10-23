//! Leader-side builders (DESIGN §8.5, §8.6): the next signed ROSTER (JOIN_REQ admission,
//! remove, transfer), REKEY to every current roster member (lock, remove) and the signed
//! GROUP_CTL (close, transfer-leader). Every body the leader issues is fed back through
//! the same acceptance rules as a received one, so the leader's stored state is exactly
//! what anti-entropy hands out.

use mesh_crypto::accept::ControlEffect;
use mesh_crypto::rekey::{self, RekeyRecipient};
use mesh_crypto::roster::{self, entry_for_identity};
use mesh_crypto::{CryptoError, KeySchedule, Secret32, group_ctl};
use mesh_wire::mesh::{RosterEntry, RosterRole};
use rand_chacha::ChaCha20Rng;

use super::state::{Group, Joiner};

/// Bodies produced by a leader operation, to be flooded in this order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Issued {
    pub rekey: Option<Vec<u8>>,
    pub roster: Option<Vec<u8>>,
    pub group_ctl: Option<Vec<u8>>,
    /// Effects of accepting them locally.
    pub effects: Vec<ControlEffect>,
}

fn entries(g: &Group) -> Vec<RosterEntry> {
    g.roster().map(|r| r.entries.clone()).unwrap_or_default()
}

/// Signs `entries` as roster version + 1 for the newest epoch and accepts it locally.
fn issue_roster(g: &mut Group, entries: Vec<RosterEntry>) -> Result<Issued, CryptoError> {
    let sched = g.keyring.newest();
    let sig = mesh_crypto::invite::sign_invite(
        &g.identity,
        &g.id,
        sched.epoch(),
        g.policy,
        &g.name,
        sched.epoch_secret(),
    );
    let r = roster::build(
        &g.identity,
        g.id,
        sched.epoch(),
        g.control.roster_version() + 1,
        sig,
        entries,
    )?;
    let body = r.encode()?;
    let effects = g.control.on_roster(&body)?;
    Ok(Issued {
        roster: Some(body),
        effects,
        ..Issued::default()
    })
}

/// The first roster of a new group: the leader alone.
pub fn initial_roster(g: &mut Group) -> Result<Issued, CryptoError> {
    let me = entry_for_identity(g.identity.public(), RosterRole::Leader, &g.display_name);
    issue_roster(g, vec![me])
}

/// Admits a verified joiner (JOIN_REQ is idempotent by `id_hash`). `None` when nothing
/// changes, the identity is denied, or its `node_id` collides with a listed member (the
/// leader never assigns a colliding `node_id`, §8.2).
pub fn admit(g: &mut Group, j: &Joiner) -> Result<Option<Issued>, CryptoError> {
    if !g.is_leader() {
        return Err(CryptoError::Invalid("not the leader"));
    }
    let h = j.identity.id_hash();
    if g.control.deny().is_denied(&h) {
        return Ok(None);
    }
    let mut list = entries(g);
    let node = j.identity.node_id();
    if list
        .iter()
        .any(|e| e.node_id == node && e.id_hash != h && e.role != RosterRole::Denied)
    {
        return Ok(None);
    }
    let entry = entry_for_identity(&j.identity, RosterRole::Member, &j.name);
    match list.iter_mut().find(|e| e.id_hash == h) {
        Some(e) if e.name == entry.name || j.name.is_empty() => return Ok(None),
        Some(e) => e.name = entry.name,
        None => list.push(entry),
    }
    issue_roster(g, list).map(Some)
}

/// A REKEY for `epoch + 1` sealed to every non-denied roster member whose X25519 key I
/// know, plus the matching roster (new epoch, new `invite_sig`). Lock = no entry change;
/// Remove = `deny` set.
fn rekey_with(
    g: &mut Group,
    deny: Option<[u8; 16]>,
    rng: &mut ChaCha20Rng,
) -> Result<Issued, CryptoError> {
    if !g.is_leader() {
        return Err(CryptoError::Invalid("not the leader"));
    }
    let mut list = entries(g);
    if let Some(h) = deny
        && !roster::deny(&mut list, &h)
    {
        return Err(CryptoError::Invalid("not in the roster"));
    }
    let new_epoch = g.control.epoch().max(g.epoch()) + 1;
    let secret = Secret32::random(rng);
    g.keyring
        .insert(KeySchedule::new(g.id, new_epoch, secret.clone()));
    let mut issued = issue_roster(g, list)?;
    let recipients: Vec<RekeyRecipient> = g
        .roster()
        .map(|r| {
            r.entries
                .iter()
                .filter(|e| e.role != RosterRole::Denied)
                .filter_map(|e| {
                    let x_pk = if e.id_hash == g.my_id_hash() {
                        Some(g.identity.x_pk())
                    } else {
                        g.book.get(&e.id_hash).map(|j| j.identity.x_pk)
                    }?;
                    Some(RekeyRecipient {
                        node_id: e.node_id,
                        x_pk,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let rk = rekey::seal(
        &g.identity,
        &g.id,
        new_epoch,
        g.control.roster_version(),
        &secret,
        &recipients,
        rng,
    )?;
    let body = rk.encode()?;
    g.control.record_own_rekey(&body)?;
    issued.rekey = Some(body);
    issued.effects.push(ControlEffect::EpochAdvanced {
        epoch: new_epoch,
        secret,
    });
    Ok(issued)
}

/// Lock group: rekey without removing anyone (old QRs stop working).
pub fn lock(g: &mut Group, rng: &mut ChaCha20Rng) -> Result<Issued, CryptoError> {
    rekey_with(g, None, rng)
}

/// Remove member: deny entry + rekey sealed only to the remaining members.
pub fn remove(g: &mut Group, node_id: u16, rng: &mut ChaCha20Rng) -> Result<Issued, CryptoError> {
    let h = g
        .roster()
        .and_then(|r| {
            r.entries
                .iter()
                .find(|e| e.node_id == node_id && e.role != RosterRole::Denied)
        })
        .map(|e| e.id_hash)
        .ok_or(CryptoError::Invalid("not in the roster"))?;
    if h == g.my_id_hash() {
        return Err(CryptoError::Invalid("the leader cannot remove itself"));
    }
    rekey_with(g, Some(h), rng)
}

/// Close group: signed GROUP_CTL{close}.
pub fn close(g: &mut Group) -> Result<Issued, CryptoError> {
    if !g.is_leader() {
        return Err(CryptoError::Invalid("not the leader"));
    }
    let c = group_ctl::close(
        &g.identity,
        &g.id,
        g.control.ctl_version() + 1,
        g.control.epoch(),
    )?;
    let body = c.encode()?;
    let effects = g.control.on_group_ctl(&body)?;
    Ok(Issued {
        group_ctl: Some(body),
        effects,
        ..Issued::default()
    })
}

/// Transfer leadership to the roster member `node_id`: GROUP_CTL{transfer} then a roster
/// (version + 1) with the roles swapped, both signed by the current (old) key.
pub fn transfer(g: &mut Group, node_id: u16) -> Result<Issued, CryptoError> {
    if !g.is_leader() {
        return Err(CryptoError::Invalid("not the leader"));
    }
    let me = g.my_id_hash();
    let mut list = entries(g);
    let target = list
        .iter()
        .find(|e| e.node_id == node_id && e.role != RosterRole::Denied && e.id_hash != me)
        .map(|e| e.ed_pk)
        .ok_or(CryptoError::Invalid("not a roster member"))?;
    for e in &mut list {
        if e.id_hash == me {
            e.role = RosterRole::Member;
        } else if e.node_id == node_id && e.role != RosterRole::Denied {
            e.role = RosterRole::Leader;
        }
    }
    let c = group_ctl::transfer_leader(
        &g.identity,
        &g.id,
        g.control.ctl_version() + 1,
        g.control.epoch(),
        target,
        node_id,
    )?;
    let ctl_body = c.encode()?;
    let mut effects = g.control.on_group_ctl(&ctl_body)?;
    // The roster is signed by the old key: build it before the anchor switches.
    let sched = g.keyring.newest();
    let sig = mesh_crypto::invite::sign_invite(
        &g.identity,
        &g.id,
        sched.epoch(),
        g.policy,
        &g.name,
        sched.epoch_secret(),
    );
    let mut r = roster::build(
        &g.identity,
        g.id,
        sched.epoch(),
        g.control.roster_version() + 1,
        sig,
        list,
    )?;
    roster::sign(&g.identity, &mut r)?;
    let body = r.encode()?;
    effects.extend(g.control.on_roster(&body)?);
    Ok(Issued {
        rekey: None,
        roster: Some(body),
        group_ctl: Some(ctl_body),
        effects,
    })
}

/// Re-signs the current entries as the next roster version (a new leader publishes its
/// own `invite_sig` after a transfer).
pub fn reissue(g: &mut Group) -> Result<Issued, CryptoError> {
    if !g.is_leader() {
        return Err(CryptoError::Invalid("not the leader"));
    }
    let list = entries(g);
    issue_roster(g, list)
}
