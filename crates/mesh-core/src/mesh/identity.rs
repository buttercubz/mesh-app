//! The node's identity in the open group, as the mesh glue needs it: per-group
//! `node_id`, leader flag, epoch, group policy and the roster.
//!
//! The group module (WP-10B) owns the stored groups and mirrors every persisted change
//! into `EngineConfig::stored_groups` (identity regeneration on a `node_id` collision,
//! new epoch, leader handover, roster updates). The mesh glue derives its identity from
//! that blob, read-only, and re-derives it whenever the blob of the active group
//! changes. Without a decodable group (or while the session does not run: closed group,
//! removed member) the glue stays inactive.

use std::collections::BTreeSet;

use mesh_crypto::keyring::EpochKeyring;
use mesh_crypto::schedule::t10_window;
use mesh_wire::mesh::RosterRole;

use crate::engine::{Ctx, EngineMode};
use crate::group::store::{self, Stored};

/// What the mesh glue knows about this node in the open group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshIdentity {
    pub group_id: [u8; 16],
    /// Per-group `node_id` (MeshPacket `origin`).
    pub node: u16,
    pub leader: bool,
    pub epoch: u32,
    /// Group policy: leader priority (§6.1) and voice TTL (§5.5).
    pub leader_priority: bool,
    pub voice_ttl: u8,
    /// The group's Aware pairing is set up (Phase 4 upgrades allowed).
    pub aware_pairing: bool,
    /// Non-denied roster members, me included.
    pub roster: Vec<u16>,
    /// Non-denied roster members other than me: `(node_id, id_hash)` (their advertising
    /// tokens, §8.3).
    pub members: Vec<(u16, [u8; 16])>,
    /// The retained epochs' discovery keys (advertising tokens of members that may be
    /// behind, §9.3).
    pub keyring: EpochKeyring,
}

impl MeshIdentity {
    /// The identity the media module needs (session → media seam).
    pub fn media(&self) -> crate::media::MediaIdentity {
        crate::media::MediaIdentity {
            node: self.node,
            leader: self.leader,
            epoch: self.epoch,
            leader_priority: self.leader_priority,
        }
    }

    /// The advertising tokens `nodes` may show now: every retained epoch, slots
    /// `t10 − 1 … t10 + 1` (§8.3, §9.3, the same window the HELLO check accepts).
    pub fn member_tokens(&self, t10: u32, nodes: &[u16]) -> BTreeSet<[u8; 4]> {
        let mut out = BTreeSet::new();
        for (_, id_hash) in self.members.iter().filter(|(n, _)| nodes.contains(n)) {
            for sched in self.keyring.iter() {
                for t in t10_window(t10) {
                    out.insert(sched.node_token(t, id_hash));
                }
            }
        }
        out
    }

    /// Decodes the identity from a stored group blob, if the session can run on it.
    pub fn from_blob(group_id: [u8; 16], blob: &[u8]) -> Option<MeshIdentity> {
        let Ok(Stored::Live(g)) = store::decode(blob) else {
            return None;
        };
        if g.id != group_id || !g.session_runs() {
            return None;
        }
        let mut roster: Vec<u16> = g.roster_indices().into_iter().map(|(n, _)| n).collect();
        let me = g.my_node_id();
        if !roster.contains(&me) {
            roster.push(me);
        }
        roster.sort_unstable();
        let mine = g.my_id_hash();
        let members = g
            .roster()
            .map(|r| {
                r.entries
                    .iter()
                    .filter(|e| e.role != RosterRole::Denied && e.id_hash != mine)
                    .map(|e| (e.node_id, e.id_hash))
                    .collect()
            })
            .unwrap_or_default();
        Some(MeshIdentity {
            group_id,
            node: me,
            leader: g.is_leader(),
            epoch: g.epoch(),
            leader_priority: g.policy.leader_priority(),
            voice_ttl: g.policy.voice_ttl(),
            aware_pairing: g.aware_pairing,
            roster,
            members,
            keyring: g.keyring.clone(),
        })
    }
}

/// Tracks the blob of the active group and re-derives the identity on change.
#[derive(Debug, Default)]
pub struct IdentitySource {
    blob: Option<Vec<u8>>,
    current: Option<MeshIdentity>,
}

impl IdentitySource {
    pub fn new() -> Self {
        Self::default()
    }

    /// The identity derived last.
    pub fn current(&self) -> Option<&MeshIdentity> {
        self.current.as_ref()
    }

    /// Forgets everything (engine left the session).
    pub fn clear(&mut self) {
        self.blob = None;
        self.current = None;
    }

    /// Re-reads the stored blob of the session's group. Returns `true` when the
    /// identity changed (appeared, disappeared or any field differs).
    pub fn refresh(&mut self, ctx: &Ctx) -> bool {
        let EngineMode::Session { group_id } = ctx.mode else {
            let changed = self.current.is_some();
            self.clear();
            return changed;
        };
        let blob = ctx
            .cfg
            .stored_groups
            .iter()
            .find(|g| g.group_id == group_id)
            .map(|g| g.blob.as_slice());
        if blob == self.blob.as_deref() {
            return false;
        }
        self.blob = blob.map(<[u8]>::to_vec);
        let next = blob.and_then(|b| MeshIdentity::from_blob(group_id, b));
        let changed = next != self.current;
        self.current = next;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::state::Group;
    use mesh_api::Vehicle;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn identity_follows_the_stored_group() {
        let mut rng = ChaCha20Rng::from_seed([4; 32]);
        let leader = Group::create("Ride", Vehicle::Moto, "Ana", 0, &mut rng);
        let member = Group::join(&leader.invite().unwrap(), "Ben", Vehicle::Car, &mut rng);
        let id = MeshIdentity::from_blob(leader.id, &store::encode(&leader)).unwrap();
        assert_eq!(id.node, leader.my_node_id());
        assert!(id.leader && id.leader_priority);
        assert_eq!((id.epoch, id.voice_ttl), (0, 4));
        assert_eq!(id.roster, vec![id.node]);
        let m = MeshIdentity::from_blob(member.id, &store::encode(&member)).unwrap();
        assert!(!m.leader);
        assert_ne!(m.node, id.node);
        assert_eq!(m.media().node, m.node);
        // Wrong group id or garbage: no identity (the glue stays inactive).
        assert!(MeshIdentity::from_blob([0; 16], &store::encode(&leader)).is_none());
        assert!(MeshIdentity::from_blob(leader.id, &[1, 2, 3]).is_none());
    }
}
