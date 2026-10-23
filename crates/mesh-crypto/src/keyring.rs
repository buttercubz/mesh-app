//! Retained-epoch keyring and deny list (DESIGN §8.6).
//!
//! Nodes retain the last 4 epoch secrets and keep accepting handshakes under any
//! retained older epoch, but only from identities listed (non-denied) in the current
//! roster that are not yet known to hold the newest epoch. There is no time limit.

use std::collections::BTreeSet;

use mesh_wire::mesh::{Roster, RosterRole};

use crate::error::{CryptoError, Result};
use crate::schedule::{KeySchedule, t10, t10_window};

/// Retained epochs (Appendix A).
pub const RETAINED_EPOCHS: usize = 4;

/// The last [`RETAINED_EPOCHS`] key schedules of one group, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochKeyring {
    epochs: Vec<KeySchedule>,
}

impl EpochKeyring {
    pub fn new(first: KeySchedule) -> Self {
        EpochKeyring {
            epochs: vec![first],
        }
    }

    /// Adds an epoch (from a REKEY or an invite). Keeps the newest four; an epoch that
    /// is already present or older than every retained one when the ring is full is
    /// ignored. Returns `true` when it became the newest epoch.
    pub fn insert(&mut self, sched: KeySchedule) -> bool {
        if self.get(sched.epoch()).is_some() {
            return false;
        }
        if sched.group_id() != self.newest().group_id() {
            return false;
        }
        let pos = self
            .epochs
            .iter()
            .position(|s| s.epoch() < sched.epoch())
            .unwrap_or(self.epochs.len());
        if pos >= RETAINED_EPOCHS {
            return false;
        }
        self.epochs.insert(pos, sched);
        self.epochs.truncate(RETAINED_EPOCHS);
        pos == 0
    }

    pub fn newest(&self) -> &KeySchedule {
        &self.epochs[0]
    }

    pub fn get(&self, epoch: u32) -> Option<&KeySchedule> {
        self.epochs.iter().find(|s| s.epoch() == epoch)
    }

    /// The retained epoch whose low 16 bits equal `epoch_lo` (HELLO).
    pub fn by_epoch_lo(&self, epoch_lo: u16) -> Option<&KeySchedule> {
        self.epochs.iter().find(|s| s.epoch_lo() == epoch_lo)
    }

    /// Newest first.
    pub fn iter(&self) -> impl Iterator<Item = &KeySchedule> {
        self.epochs.iter()
    }

    pub fn len(&self) -> usize {
        self.epochs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.epochs.is_empty()
    }

    /// Order in which a responder tries PSKs: the epoch named by the initiator's
    /// HELLO `epoch_lo` first when retained, then the rest newest first.
    pub fn handshake_order(&self, hint_epoch_lo: Option<u16>) -> Vec<&KeySchedule> {
        let mut out: Vec<&KeySchedule> = Vec::with_capacity(self.epochs.len());
        if let Some(s) = hint_epoch_lo.and_then(|lo| self.by_epoch_lo(lo)) {
            out.push(s);
        }
        for s in &self.epochs {
            if !out.iter().any(|o| o.epoch() == s.epoch()) {
                out.push(s);
            }
        }
        out
    }

    /// Finds the retained epoch and slot of an advertised group tag: every retained
    /// epoch newest first, slots `t10, t10 − 1, t10 + 1`.
    pub fn match_group_tag(&self, tag: &[u8; 4], unix_seconds: u64) -> Option<(u32, u32)> {
        let now = t10(unix_seconds);
        self.epochs.iter().find_map(|s| {
            t10_window(now)
                .into_iter()
                .find(|&t| &s.group_tag(t) == tag)
                .map(|t| (s.epoch(), t))
        })
    }

    /// Maps an advertised node token to a member for every retained epoch (newest
    /// first) and slots `t10, t10 − 1, t10 + 1`. Returns `(epoch, id_hash)`.
    pub fn match_node_token(
        &self,
        token: &[u8; 4],
        unix_seconds: u64,
        members: &[[u8; 16]],
    ) -> Option<(u32, [u8; 16])> {
        let now = t10(unix_seconds);
        self.epochs.iter().find_map(|s| {
            s.match_node_token(token, now, members.iter())
                .map(|id| (s.epoch(), id))
        })
    }
}

/// Denied `id_hash` values: accumulated from every accepted ROSTER (role 0xFF).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DenyList {
    denied: BTreeSet<[u8; 16]>,
}

impl DenyList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the roster's deny entries. Returns the newly denied identities, so the
    /// caller can close their links with `CLOSE{5}` (§8.9 rule 7).
    pub fn merge_roster(&mut self, roster: &Roster) -> Vec<[u8; 16]> {
        let mut added = Vec::new();
        for e in roster
            .entries
            .iter()
            .filter(|e| e.role == RosterRole::Denied)
        {
            if self.denied.insert(e.id_hash) {
                added.push(e.id_hash);
            }
        }
        added
    }

    pub fn deny(&mut self, id_hash: [u8; 16]) -> bool {
        self.denied.insert(id_hash)
    }

    pub fn is_denied(&self, id_hash: &[u8; 16]) -> bool {
        self.denied.contains(id_hash)
    }

    /// Fails with [`CryptoError::Denied`] for a denied identity.
    pub fn check(&self, id_hash: &[u8; 16]) -> Result<()> {
        if self.is_denied(id_hash) {
            Err(CryptoError::Denied)
        } else {
            Ok(())
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &[u8; 16]> {
        self.denied.iter()
    }
}

/// Old-epoch admission (§8.6 rule 4), applied once the peer's IDPROOF verified on a link
/// whose handshake used `link_epoch`:
/// - the newest epoch is always acceptable (subject to the deny list);
/// - an older retained epoch is acceptable only for a non-denied member of the current
///   roster that is not yet known to hold the newest epoch.
pub fn admit_epoch(
    link_epoch: u32,
    newest_epoch: u32,
    peer_id_hash: &[u8; 16],
    roster: Option<&Roster>,
    deny: &DenyList,
    known_newest_holders: &BTreeSet<[u8; 16]>,
) -> Result<()> {
    deny.check(peer_id_hash)?;
    if link_epoch == newest_epoch {
        return Ok(());
    }
    if link_epoch > newest_epoch {
        return Err(CryptoError::WrongGroup);
    }
    let listed = roster.is_some_and(|r| {
        r.entries
            .iter()
            .any(|e| &e.id_hash == peer_id_hash && e.role != RosterRole::Denied)
    });
    if !listed {
        return Err(CryptoError::Stale(
            "old epoch from an identity not in the roster",
        ));
    }
    if known_newest_holders.contains(peer_id_hash) {
        return Err(CryptoError::Stale("peer already holds the newest epoch"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::Secret32;
    use mesh_wire::mesh::RosterEntry;

    fn s(epoch: u32) -> KeySchedule {
        KeySchedule::new([1; 16], epoch, Secret32::new([epoch as u8; 32]))
    }

    #[test]
    fn retains_last_four_newest_first() {
        let mut k = EpochKeyring::new(s(0));
        for e in 1..=5 {
            assert!(k.insert(s(e)));
        }
        let epochs: Vec<u32> = k.iter().map(KeySchedule::epoch).collect();
        assert_eq!(epochs, vec![5, 4, 3, 2]);
        assert!(!k.insert(s(1)), "older than every retained epoch");
        assert!(!k.insert(s(4)), "duplicate");
        assert_eq!(k.newest().epoch(), 5);
        assert!(k.get(1).is_none());
        assert_eq!(k.by_epoch_lo(3).map(KeySchedule::epoch), Some(3));
        let order: Vec<u32> = k
            .handshake_order(Some(3))
            .iter()
            .map(|s| s.epoch())
            .collect();
        assert_eq!(order, vec![3, 5, 4, 2]);
        let order: Vec<u32> = k
            .handshake_order(Some(9))
            .iter()
            .map(|s| s.epoch())
            .collect();
        assert_eq!(order, vec![5, 4, 3, 2]);
        // A different group's schedule never enters.
        assert!(!k.insert(KeySchedule::new([2; 16], 9, Secret32::new([0; 32]))));
    }

    #[test]
    fn group_tag_matches_any_retained_epoch() {
        let mut k = EpochKeyring::new(s(0));
        k.insert(s(1));
        let now = 1_000_000u64;
        let tag_old = s(0).group_tag(t10(now) - 1);
        assert_eq!(k.match_group_tag(&tag_old, now), Some((0, t10(now) - 1)));
        let tag_new = s(1).group_tag(t10(now));
        assert_eq!(k.match_group_tag(&tag_new, now), Some((1, t10(now))));
        assert_eq!(k.match_group_tag(&[0; 4], now), None);
        let members = [[3u8; 16], [4u8; 16]];
        let tok = s(0).node_token(t10(now) + 1, &members[1]);
        assert_eq!(
            k.match_node_token(&tok, now, &members),
            Some((0, members[1]))
        );
        let tok = s(1).node_token(t10(now), &members[0]);
        assert_eq!(
            k.match_node_token(&tok, now, &members),
            Some((1, members[0]))
        );
        assert_eq!(k.match_node_token(&[9; 4], now, &members), None);
    }

    fn roster(entries: Vec<([u8; 16], RosterRole)>) -> Roster {
        Roster {
            group_id: [1; 16],
            epoch: 3,
            roster_version: 1,
            invite_sig: [0; 64],
            entries: entries
                .into_iter()
                .map(|(h, role)| RosterEntry {
                    node_id: u16::from_le_bytes([h[0], h[1]]),
                    id_hash: h,
                    ed_pk: [0; 32],
                    role,
                    name: String::new(),
                })
                .collect(),
            sig: [0; 64],
        }
    }

    #[test]
    fn old_epoch_admission_rules() {
        let member = [5; 16];
        let removed = [6; 16];
        let stranger = [7; 16];
        let r = roster(vec![
            (member, RosterRole::Member),
            (removed, RosterRole::Denied),
        ]);
        let mut deny = DenyList::new();
        assert_eq!(deny.merge_roster(&r), vec![removed]);
        assert!(deny.merge_roster(&r).is_empty());
        let mut holders = BTreeSet::new();
        assert!(admit_epoch(3, 3, &member, Some(&r), &deny, &holders).is_ok());
        assert!(admit_epoch(1, 3, &member, Some(&r), &deny, &holders).is_ok());
        assert_eq!(
            admit_epoch(3, 3, &removed, Some(&r), &deny, &holders),
            Err(CryptoError::Denied)
        );
        assert!(admit_epoch(2, 3, &stranger, Some(&r), &deny, &holders).is_err());
        assert!(admit_epoch(2, 3, &member, None, &deny, &holders).is_err());
        holders.insert(member);
        assert!(admit_epoch(2, 3, &member, Some(&r), &deny, &holders).is_err());
        assert!(admit_epoch(4, 3, &member, Some(&r), &deny, &holders).is_err());
    }
}
