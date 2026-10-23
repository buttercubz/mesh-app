//! One stored group (DESIGN §8.2-§8.6): per-group identity, retained epoch keyring,
//! signed control state, the leader's book of verified joiners, invite settings. The
//! leader-side builders (roster, rekey, group control) live in `leader.rs`.

use std::collections::BTreeMap;

use mesh_api::{MemberRole, Vehicle};
use mesh_crypto::invite::{Invite, InvitePolicy, InviteSig, sign_invite};
use mesh_crypto::{EpochKeyring, GroupIdentity, KeySchedule, PublicIdentity, Secret32};
use mesh_wire::mesh::{Roster, RosterRole};
use rand_chacha::ChaCha20Rng;

use super::control::Control;

/// A known identity of the group (from a verified JOIN_REQ or IDPROOF): what the leader
/// needs to seal a REKEY to it and to list it in the roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joiner {
    pub identity: PublicIdentity,
    /// CAPS platform byte (0 iOS, 1 Android, 2 websim).
    pub platform: u8,
    pub name: String,
}

/// A live stored group.
#[derive(Debug, Clone)]
pub struct Group {
    pub id: [u8; 16],
    pub name: String,
    pub vehicle: Vehicle,
    pub policy: InvitePolicy,
    /// My display name in this group (JOIN_REQ, roster entry).
    pub display_name: String,
    pub identity: GroupIdentity,
    pub keyring: EpochKeyring,
    pub control: Control,
    /// `leader_ed_pk` of the invite this node joined with (first-ROSTER cross-check).
    pub invite_leader: [u8; 32],
    /// The first verified ROSTER did not match the invite's leader key.
    pub unverified_leader: bool,
    /// Verified identities by `id_hash`.
    pub book: BTreeMap<[u8; 16], Joiner>,
    /// Leader-local invite expiry (unix minutes).
    pub expires_at: Option<u32>,
    pub aware_pairing: bool,
}

/// A closed group kept only for the group list (its secrets are deleted, §8.6 step 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedGroup {
    pub id: [u8; 16],
    pub name: String,
    pub vehicle: Vehicle,
    pub role: MemberRole,
    pub members: u8,
    pub epoch: u32,
}

impl Group {
    /// F1 Create group: random `group_id`, epoch-0 secret and the leader's identity;
    /// roster version 1 lists the leader.
    pub fn create(
        name: &str,
        vehicle: Vehicle,
        display_name: &str,
        platform: u8,
        rng: &mut ChaCha20Rng,
    ) -> Group {
        let mut id = [0u8; 16];
        rand_core::RngCore::fill_bytes(rng, &mut id);
        let identity = GroupIdentity::generate(rng);
        let secret = Secret32::random(rng);
        let keyring = EpochKeyring::new(KeySchedule::new(id, 0, secret));
        let name = mesh_wire::name::truncate_graphemes(name, mesh_wire::name::GROUP_NAME_MAX);
        let mut g = Group {
            id,
            name: name.to_owned(),
            vehicle,
            policy: InvitePolicy::DEFAULT,
            display_name: display_name.to_owned(),
            control: Control::new(id, identity.ed_pk(), 0),
            invite_leader: identity.ed_pk(),
            identity,
            keyring,
            unverified_leader: false,
            book: BTreeMap::new(),
            expires_at: None,
            aware_pairing: false,
        };
        g.remember(*g.identity.public(), platform, display_name);
        g
    }

    /// F2 Join from a parsed invite: a fresh per-group identity, the invite's epoch.
    pub fn join(
        invite: &Invite,
        display_name: &str,
        vehicle: Vehicle,
        rng: &mut ChaCha20Rng,
    ) -> Group {
        let identity = GroupIdentity::generate(rng);
        let sched = KeySchedule::new(invite.group_id, invite.epoch, invite.epoch_secret.clone());
        Group {
            id: invite.group_id,
            name: invite.name.clone(),
            vehicle,
            policy: invite.policy,
            display_name: display_name.to_owned(),
            identity,
            keyring: EpochKeyring::new(sched),
            control: Control::new(invite.group_id, invite.leader_ed_pk, invite.epoch),
            invite_leader: invite.leader_ed_pk,
            unverified_leader: false,
            book: BTreeMap::new(),
            expires_at: None,
            aware_pairing: invite.aware_pairing(),
        }
    }

    /// Replaces my per-group identity (`node_id` collision, §8.2): I rejoin with the
    /// same epoch secret.
    pub fn regenerate_identity(&mut self, rng: &mut ChaCha20Rng) {
        self.identity = GroupIdentity::generate(rng);
    }

    /// Records a verified identity. Returns `true` when it is new or changed.
    pub fn remember(&mut self, identity: PublicIdentity, platform: u8, name: &str) -> bool {
        let j = Joiner {
            identity,
            platform,
            name: mesh_wire::name::truncate_graphemes(name, mesh_wire::name::DISPLAY_NAME_MAX)
                .to_owned(),
        };
        let h = identity.id_hash();
        if self.book.get(&h) == Some(&j) {
            return false;
        }
        self.book.insert(h, j);
        true
    }

    /// Records an identity proven by IDPROOF (no name or platform): keeps a known entry.
    pub fn remember_identity(&mut self, identity: PublicIdentity, platform: u8) -> bool {
        if self.book.contains_key(&identity.id_hash()) {
            return false;
        }
        self.remember(identity, platform, "")
    }

    pub fn my_id_hash(&self) -> [u8; 16] {
        self.identity.id_hash()
    }

    pub fn my_node_id(&self) -> u16 {
        self.identity.node_id()
    }

    /// I hold the current trust anchor key.
    pub fn is_leader(&self) -> bool {
        &self.identity.ed_pk() == self.control.leader_pk()
    }

    pub fn role(&self) -> MemberRole {
        if self.is_leader() {
            MemberRole::Leader
        } else {
            MemberRole::Member
        }
    }

    /// Newest epoch I hold a secret for.
    pub fn epoch(&self) -> u32 {
        self.keyring.newest().epoch()
    }

    pub fn roster(&self) -> Option<&Roster> {
        self.control.roster()
    }

    /// I am listed (non-denied) in the verified roster.
    pub fn in_roster(&self) -> bool {
        let me = self.my_id_hash();
        self.roster().is_some_and(|r| {
            r.entries
                .iter()
                .any(|e| e.id_hash == me && e.role != RosterRole::Denied)
        })
    }

    /// I am denied in the verified roster.
    pub fn am_denied(&self) -> bool {
        self.control.deny().is_denied(&self.my_id_hash())
    }

    /// The group session may run: the group is not closed and I was not removed.
    pub fn session_runs(&self) -> bool {
        !self.control.is_closed() && !self.am_denied()
    }

    /// `(node_id, roster index)` of every non-denied entry.
    pub fn roster_indices(&self) -> Vec<(u16, u8)> {
        self.roster()
            .map(|r| {
                r.entries
                    .iter()
                    .filter(|e| e.role != RosterRole::Denied)
                    .enumerate()
                    .map(|(i, e)| (e.node_id, i.min(255) as u8))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Non-denied roster members (count, ≥ 1 for me).
    pub fn member_count(&self) -> u8 {
        self.roster_indices().len().clamp(1, 255) as u8
    }

    /// Display name of `node_id` from the roster (then the book).
    pub fn name_of(&self, node_id: u16) -> Option<String> {
        if let Some(e) = self.roster().and_then(|r| {
            r.entries
                .iter()
                .find(|e| e.node_id == node_id && e.role != RosterRole::Denied)
        }) {
            return Some(e.name.clone());
        }
        self.book
            .values()
            .find(|j| j.identity.node_id() == node_id)
            .map(|j| j.name.clone())
    }

    /// The invite signature of the newest epoch: my own when I lead, otherwise the
    /// verified roster's `invite_sig` when it is for my newest epoch and verifies.
    pub fn invite_sig(&self) -> Option<InviteSig> {
        let sched = self.keyring.newest();
        if self.is_leader() {
            return Some(sign_invite(
                &self.identity,
                &self.id,
                sched.epoch(),
                self.policy,
                &self.name,
                sched.epoch_secret(),
            ));
        }
        let r = self.roster()?;
        if r.epoch != sched.epoch() {
            return None;
        }
        mesh_crypto::roster::verify_invite_sig(
            r,
            sched.epoch_secret(),
            self.policy,
            &self.name,
            self.control.leader_pk(),
        )
        .ok()?;
        Some(InviteSig(r.invite_sig))
    }

    /// The leader-signed invite of the newest epoch (byte-identical for every member).
    pub fn invite(&self) -> Option<Invite> {
        let sched = self.keyring.newest();
        let sig = self.invite_sig()?;
        let expires = if self.is_leader() {
            self.expires_at
        } else {
            None
        };
        Some(Invite::assemble(
            self.id,
            sched.epoch(),
            sched.epoch_secret().clone(),
            *self.control.leader_pk(),
            self.policy,
            &self.name,
            sig,
            self.aware_pairing,
            expires,
        ))
    }

    /// Tombstone kept after Close group.
    pub fn closed(&self) -> ClosedGroup {
        ClosedGroup {
            id: self.id,
            name: self.name.clone(),
            vehicle: self.vehicle,
            role: self.role(),
            members: self.member_count(),
            epoch: self.epoch(),
        }
    }
}
