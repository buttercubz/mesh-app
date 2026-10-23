//! Stored control state of one group (DESIGN §8.9): the latest accepted ROSTER, REKEY
//! and GROUP_CTL bodies verbatim (self-authenticating, so they can be re-sent as SYNC),
//! the leader trust anchor, the current epoch and the deny list.

pub use mesh_wire::record::SyncKind;

use mesh_wire::mesh::{GroupCtl, GroupCtlOp, Rekey, Roster, RosterRole};
use mesh_wire::serial::peer_behind;

use crate::error::{CryptoError, Result};
use crate::identity::GroupIdentity;
use crate::keyring::DenyList;
use crate::prim::Secret32;

/// What accepting a control body changed (§8.9 rule 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEffect {
    /// A newer roster was stored; close links to `newly_denied` with `CLOSE{5}`.
    RosterAccepted {
        roster_version: u32,
        newly_denied: Vec<[u8; 16]>,
    },
    /// A newer epoch: add it to the keyring and switch new handshakes to it.
    EpochAdvanced { epoch: u32, secret: Secret32 },
    /// GROUP_CTL{close}: the group is closed; delete its secrets.
    GroupClosed,
    /// GROUP_CTL{transfer-leader} verified; waiting for the matching ROSTER.
    TransferPending {
        new_leader_ed_pk: [u8; 32],
        new_leader_node: u16,
    },
    /// The trust anchor switched to a new leader key.
    LeaderChanged { new_leader_ed_pk: [u8; 32] },
}

/// Control state of one group as one node holds it.
#[derive(Debug, Clone)]
pub struct ControlState {
    group_id: [u8; 16],
    leader_ed_pk: [u8; 32],
    epoch: u32,
    roster: Option<(Roster, Vec<u8>)>,
    rekey: Option<(u32, Vec<u8>)>,
    group_ctl: Option<(GroupCtl, Vec<u8>)>,
    pending_transfer: Option<([u8; 32], u16)>,
    closed: bool,
    deny: DenyList,
}

impl ControlState {
    /// Starts from the invite: `leader_ed_pk` and the invite's epoch.
    pub fn new(group_id: [u8; 16], leader_ed_pk: [u8; 32], epoch: u32) -> Self {
        ControlState {
            group_id,
            leader_ed_pk,
            epoch,
            roster: None,
            rekey: None,
            group_ctl: None,
            pending_transfer: None,
            closed: false,
            deny: DenyList::new(),
        }
    }

    pub fn group_id(&self) -> &[u8; 16] {
        &self.group_id
    }
    /// Current trust anchor.
    pub fn leader_ed_pk(&self) -> &[u8; 32] {
        &self.leader_ed_pk
    }
    pub fn epoch(&self) -> u32 {
        self.epoch
    }
    pub fn roster(&self) -> Option<&Roster> {
        self.roster.as_ref().map(|(r, _)| r)
    }
    /// Stored roster version (0 before the first roster).
    pub fn roster_version(&self) -> u32 {
        self.roster.as_ref().map_or(0, |(r, _)| r.roster_version)
    }
    /// Stored GROUP_CTL version (0 before the first).
    pub fn ctl_version(&self) -> u32 {
        self.group_ctl.as_ref().map_or(0, |(c, _)| c.ctl_version)
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    pub fn deny(&self) -> &DenyList {
        &self.deny
    }
    pub fn pending_transfer(&self) -> Option<([u8; 32], u16)> {
        self.pending_transfer
    }

    /// The CAPS anti-entropy summary `(epoch, roster_version, ctl_version)`.
    pub fn summary(&self) -> (u32, u32, u32) {
        (self.epoch, self.roster_version(), self.ctl_version())
    }

    /// Verbatim stored body of `kind` (for SYNC).
    pub fn body(&self, kind: SyncKind) -> Option<&[u8]> {
        match kind {
            SyncKind::Roster => self.roster.as_ref().map(|(_, b)| b.as_slice()),
            SyncKind::Rekey => self.rekey.as_ref().map(|(_, b)| b.as_slice()),
            SyncKind::GroupCtl => self.group_ctl.as_ref().map(|(_, b)| b.as_slice()),
            SyncKind::JoinReq => None,
        }
    }

    /// Bodies to push to a peer whose CAPS summary is `peer`, in the order REKEY →
    /// ROSTER → GROUP_CTL (§8.9 rule 2).
    pub fn to_push(&self, peer: (u32, u32, u32)) -> Vec<SyncKind> {
        let mut out = Vec::new();
        if peer.0 < self.epoch && self.rekey.is_some() {
            out.push(SyncKind::Rekey);
        }
        if peer.1 < self.roster_version() {
            out.push(SyncKind::Roster);
        }
        if peer.2 < self.ctl_version() {
            out.push(SyncKind::GroupCtl);
        }
        out
    }

    /// The same decision from a direct neighbor's PRESENCE (`epoch16`, `roster_ver16`,
    /// serial-number arithmetic, §8.9 rule 3).
    pub fn to_push_presence(&self, epoch16: u16, roster_ver16: u16) -> Vec<SyncKind> {
        let mut out = Vec::new();
        if peer_behind(self.epoch, epoch16) && self.rekey.is_some() {
            out.push(SyncKind::Rekey);
        }
        if self.roster.is_some() && peer_behind(self.roster_version(), roster_ver16) {
            out.push(SyncKind::Roster);
        }
        out
    }

    /// Processes a received (flooded or SYNC) body of `kind`. JOIN_REQ is the leader's
    /// business ([`crate::join::JoinBook`]) and is refused here.
    pub fn on_body(
        &mut self,
        kind: SyncKind,
        body: &[u8],
        me: &GroupIdentity,
    ) -> Result<Vec<ControlEffect>> {
        match kind {
            SyncKind::Roster => self.on_roster(body),
            SyncKind::Rekey => self.on_rekey(body, me),
            SyncKind::GroupCtl => self.on_group_ctl(body),
            SyncKind::JoinReq => Err(CryptoError::Invalid("JOIN_REQ is not stored control state")),
        }
    }

    /// ROSTER acceptance (§4.7) and effects.
    pub fn on_roster(&mut self, body: &[u8]) -> Result<Vec<ControlEffect>> {
        self.check_open()?;
        let r = Roster::decode(body)?;
        let stored = self.roster.as_ref().map(|(r, _)| r.roster_version);
        super::roster(&r, &self.group_id, &self.leader_ed_pk, stored)?;
        let newly_denied = self.deny.merge_roster(&r);
        let mut effects = vec![ControlEffect::RosterAccepted {
            roster_version: r.roster_version,
            newly_denied,
        }];
        self.roster = Some((r, body.to_vec()));
        if let Some(e) = self.complete_transfer() {
            effects.push(e);
        }
        Ok(effects)
    }

    /// REKEY acceptance (§4.7): opens my entry and advances the epoch.
    pub fn on_rekey(&mut self, body: &[u8], me: &GroupIdentity) -> Result<Vec<ControlEffect>> {
        self.check_open()?;
        let rk = Rekey::decode(body)?;
        let secret = super::rekey(&rk, &self.group_id, &self.leader_ed_pk, self.epoch, me)?;
        self.epoch = rk.new_epoch;
        self.rekey = Some((rk.new_epoch, body.to_vec()));
        Ok(vec![ControlEffect::EpochAdvanced {
            epoch: rk.new_epoch,
            secret,
        }])
    }

    /// The leader stores its own REKEY (it already knows the new secret and need not
    /// seal one to itself). The signature and monotonic rule still apply.
    pub fn record_own_rekey(&mut self, body: &[u8]) -> Result<u32> {
        self.check_open()?;
        let rk = Rekey::decode(body)?;
        crate::rekey::verify(&rk, &self.group_id, &self.leader_ed_pk)?;
        if rk.new_epoch <= self.epoch {
            return Err(CryptoError::Stale(
                "new_epoch not greater than current epoch",
            ));
        }
        self.epoch = rk.new_epoch;
        self.rekey = Some((rk.new_epoch, body.to_vec()));
        Ok(rk.new_epoch)
    }

    /// GROUP_CTL acceptance (§4.7) and effects.
    pub fn on_group_ctl(&mut self, body: &[u8]) -> Result<Vec<ControlEffect>> {
        self.check_open()?;
        let c = GroupCtl::decode(body)?;
        let stored = self.group_ctl.as_ref().map(|(c, _)| c.ctl_version);
        super::group_ctl(&c, &self.group_id, &self.leader_ed_pk, stored, self.epoch)?;
        self.group_ctl = Some((c, body.to_vec()));
        let mut effects = Vec::new();
        match (c.op, c.new_leader) {
            (GroupCtlOp::Close, _) => {
                self.closed = true;
                effects.push(ControlEffect::GroupClosed);
            }
            (GroupCtlOp::TransferLeader, Some((pk, node))) => {
                self.pending_transfer = Some((pk, node));
                effects.push(ControlEffect::TransferPending {
                    new_leader_ed_pk: pk,
                    new_leader_node: node,
                });
                if let Some(e) = self.complete_transfer() {
                    effects.push(e);
                }
            }
            (GroupCtlOp::TransferLeader, None) => {
                return Err(CryptoError::Invalid("transfer-leader without a key"));
            }
        }
        Ok(effects)
    }

    /// The anchor switches once both the transfer GROUP_CTL and a ROSTER (signed by the
    /// old key) listing the new key as leader have verified, in either order.
    fn complete_transfer(&mut self) -> Option<ControlEffect> {
        let (pk, node) = self.pending_transfer?;
        let (r, _) = self.roster.as_ref()?;
        let lists_new_leader = r
            .entries
            .iter()
            .any(|e| e.role == RosterRole::Leader && e.ed_pk == pk && e.node_id == node);
        if !lists_new_leader {
            return None;
        }
        self.leader_ed_pk = pk;
        self.pending_transfer = None;
        Some(ControlEffect::LeaderChanged {
            new_leader_ed_pk: pk,
        })
    }

    fn check_open(&self) -> Result<()> {
        if self.closed {
            Err(CryptoError::Stale("group is closed"))
        } else {
            Ok(())
        }
    }
}
