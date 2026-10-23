//! Signed control state of one group (DESIGN §4.7, §8.5-§8.9): the leader trust anchor,
//! the current epoch, the latest accepted ROSTER / REKEY / GROUP_CTL bodies stored
//! verbatim (self-authenticating, re-sent as SYNC), the deny list and a pending leader
//! transfer. Acceptance uses exactly the monotonic rules of `mesh_crypto::accept`, so
//! flooding and anti-entropy can never roll state back.
//!
//! This mirrors `mesh_crypto::ControlState` but can be restored from the persisted
//! bodies (the crypto type has no restore constructor) and exposes what the engine needs.

use mesh_crypto::accept::{self, ControlEffect};
use mesh_crypto::{CryptoError, DenyList, GroupCtl, GroupCtlOp, GroupIdentity, Rekey, Roster};
use mesh_wire::mesh::RosterRole;
use mesh_wire::record::SyncKind;
use mesh_wire::serial::peer_behind;

/// Control state of one group.
#[derive(Debug, Clone)]
pub struct Control {
    group_id: [u8; 16],
    leader_pk: [u8; 32],
    epoch: u32,
    roster: Option<(Roster, Vec<u8>)>,
    rekey: Option<(u32, Vec<u8>)>,
    ctl: Option<(GroupCtl, Vec<u8>)>,
    pending_transfer: Option<([u8; 32], u16)>,
    closed: bool,
    deny: DenyList,
}

impl Control {
    /// Fresh state from an invite (or a new group): anchor and epoch only.
    pub fn new(group_id: [u8; 16], leader_pk: [u8; 32], epoch: u32) -> Self {
        Control {
            group_id,
            leader_pk,
            epoch,
            roster: None,
            rekey: None,
            ctl: None,
            pending_transfer: None,
            closed: false,
            deny: DenyList::new(),
        }
    }

    /// Restores persisted state. Bodies were verified before they were stored; a body
    /// that no longer decodes is dropped (anti-entropy brings it back).
    pub fn restore(
        group_id: [u8; 16],
        leader_pk: [u8; 32],
        epoch: u32,
        bodies: [Option<&[u8]>; 3],
        pending_transfer: Option<([u8; 32], u16)>,
    ) -> Self {
        let mut c = Control::new(group_id, leader_pk, epoch);
        if let Some(b) = bodies[0]
            && let Ok(r) = Roster::decode(b)
        {
            c.deny.merge_roster(&r);
            c.roster = Some((r, b.to_vec()));
        }
        if let Some(b) = bodies[1]
            && let Ok(r) = Rekey::decode(b)
        {
            c.rekey = Some((r.new_epoch, b.to_vec()));
        }
        if let Some(b) = bodies[2]
            && let Ok(g) = GroupCtl::decode(b)
        {
            c.closed = g.op == GroupCtlOp::Close;
            c.ctl = Some((g, b.to_vec()));
        }
        c.pending_transfer = pending_transfer;
        c
    }

    pub fn group_id(&self) -> &[u8; 16] {
        &self.group_id
    }

    /// Current trust anchor (leader key).
    pub fn leader_pk(&self) -> &[u8; 32] {
        &self.leader_pk
    }

    /// Current epoch.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn roster(&self) -> Option<&Roster> {
        self.roster.as_ref().map(|(r, _)| r)
    }

    /// Stored roster version (0 before the first).
    pub fn roster_version(&self) -> u32 {
        self.roster.as_ref().map_or(0, |(r, _)| r.roster_version)
    }

    /// Stored GROUP_CTL version (0 before the first).
    pub fn ctl_version(&self) -> u32 {
        self.ctl.as_ref().map_or(0, |(c, _)| c.ctl_version)
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

    /// CAPS anti-entropy summary `(epoch, roster_version, ctl_version)`.
    pub fn summary(&self) -> (u32, u32, u32) {
        (self.epoch, self.roster_version(), self.ctl_version())
    }

    /// Verbatim stored body of `kind`.
    pub fn body(&self, kind: SyncKind) -> Option<&[u8]> {
        match kind {
            SyncKind::Roster => self.roster.as_ref().map(|(_, b)| b.as_slice()),
            SyncKind::Rekey => self.rekey.as_ref().map(|(_, b)| b.as_slice()),
            SyncKind::GroupCtl => self.ctl.as_ref().map(|(_, b)| b.as_slice()),
            SyncKind::JoinReq => None,
        }
    }

    /// Bodies a peer with CAPS summary `peer` is missing, in the order REKEY → ROSTER →
    /// GROUP_CTL (§8.9 rule 2). A stored transfer-leader GROUP_CTL the peer lacks goes
    /// first: it is signed by the key the peer still trusts and authorizes the new
    /// leader's key, which signs the stored REKEY and ROSTER after a handover.
    pub fn to_push(&self, peer: (u32, u32, u32)) -> Vec<SyncKind> {
        let mut out = Vec::new();
        let transfer_first = peer.2 < self.ctl_version()
            && self
                .ctl
                .as_ref()
                .is_some_and(|(c, _)| c.op == GroupCtlOp::TransferLeader);
        if transfer_first {
            out.push(SyncKind::GroupCtl);
        }
        if peer.0 < self.epoch && self.rekey.is_some() {
            out.push(SyncKind::Rekey);
        }
        if peer.1 < self.roster_version() {
            out.push(SyncKind::Roster);
        }
        if peer.2 < self.ctl_version() && !transfer_first {
            out.push(SyncKind::GroupCtl);
        }
        out
    }

    /// Runs `check` under the trust anchor, then — while a verified transfer-leader
    /// GROUP_CTL is pending — under the key it designates. The old leader's signature
    /// on the transfer is what authorizes that key; a node that missed the handover
    /// roster (anti-entropy only keeps the latest bodies, signed by the new leader after
    /// it re-issues) completes the handover with the new leader's own roster.
    fn under_anchor<T>(
        &self,
        check: impl Fn(&[u8; 32]) -> Result<T, CryptoError>,
    ) -> Result<T, CryptoError> {
        match check(&self.leader_pk) {
            Err(CryptoError::BadSignature) => match self.pending_transfer {
                Some((pk, _)) => check(&pk),
                None => Err(CryptoError::BadSignature),
            },
            other => other,
        }
    }

    /// The same decision from a direct neighbor's PRESENCE (16-bit serial fields).
    pub fn to_push_presence(&self, epoch16: u16, roster_ver16: u16) -> Vec<SyncKind> {
        let mut out = Vec::new();
        if self.rekey.is_some() && peer_behind(self.epoch, epoch16) {
            out.push(SyncKind::Rekey);
        }
        if self.roster.is_some() && peer_behind(self.roster_version(), roster_ver16) {
            out.push(SyncKind::Roster);
        }
        out
    }

    /// ROSTER acceptance (§4.7): verifies under the anchor, strictly newer version.
    pub fn on_roster(&mut self, body: &[u8]) -> Result<Vec<ControlEffect>, CryptoError> {
        self.check_open()?;
        let r = Roster::decode(body)?;
        let stored = self.roster.as_ref().map(|(r, _)| r.roster_version);
        let gid = self.group_id;
        self.under_anchor(|pk| {
            accept::roster(&r, &gid, pk, stored)?;
            // Signed by the designated new leader: it must list itself as leader.
            if pk != &self.leader_pk
                && !r
                    .entries
                    .iter()
                    .any(|e| e.role == RosterRole::Leader && &e.ed_pk == pk)
            {
                return Err(CryptoError::Invalid(
                    "new leader's roster without the leader",
                ));
            }
            Ok(())
        })?;
        let newly_denied = self.deny.merge_roster(&r);
        let mut effects = vec![ControlEffect::RosterAccepted {
            roster_version: r.roster_version,
            newly_denied,
        }];
        self.roster = Some((r, body.to_vec()));
        effects.extend(self.complete_transfer());
        Ok(effects)
    }

    /// REKEY acceptance (§4.7): newer epoch and an entry for me that opens.
    pub fn on_rekey(
        &mut self,
        body: &[u8],
        me: &GroupIdentity,
    ) -> Result<Vec<ControlEffect>, CryptoError> {
        self.check_open()?;
        let rk = Rekey::decode(body)?;
        let (gid, epoch) = (self.group_id, self.epoch);
        let secret = self.under_anchor(|pk| accept::rekey(&rk, &gid, pk, epoch, me))?;
        self.epoch = rk.new_epoch;
        self.rekey = Some((rk.new_epoch, body.to_vec()));
        Ok(vec![ControlEffect::EpochAdvanced {
            epoch: rk.new_epoch,
            secret,
        }])
    }

    /// The leader records its own REKEY (it already holds the secret).
    pub fn record_own_rekey(&mut self, body: &[u8]) -> Result<u32, CryptoError> {
        self.check_open()?;
        let rk = Rekey::decode(body)?;
        mesh_crypto::rekey::verify(&rk, &self.group_id, &self.leader_pk)?;
        if rk.new_epoch <= self.epoch {
            return Err(CryptoError::Stale(
                "new_epoch not greater than current epoch",
            ));
        }
        self.epoch = rk.new_epoch;
        self.rekey = Some((rk.new_epoch, body.to_vec()));
        Ok(rk.new_epoch)
    }

    /// GROUP_CTL acceptance (§4.7): strictly newer ctl_version, epoch ≥ current.
    pub fn on_group_ctl(&mut self, body: &[u8]) -> Result<Vec<ControlEffect>, CryptoError> {
        self.check_open()?;
        let c = GroupCtl::decode(body)?;
        let stored = self.ctl.as_ref().map(|(c, _)| c.ctl_version);
        accept::group_ctl(&c, &self.group_id, &self.leader_pk, stored, self.epoch)?;
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
            }
            (GroupCtlOp::TransferLeader, None) => {
                return Err(CryptoError::Invalid("transfer-leader without a key"));
            }
        }
        self.ctl = Some((c, body.to_vec()));
        effects.extend(self.complete_transfer());
        Ok(effects)
    }

    /// The anchor switches once the transfer GROUP_CTL and a ROSTER (signed by the old
    /// key) listing the new key as leader have both verified, in either order.
    fn complete_transfer(&mut self) -> Option<ControlEffect> {
        let (pk, node) = self.pending_transfer?;
        let (r, _) = self.roster.as_ref()?;
        let listed = r
            .entries
            .iter()
            .any(|e| e.role == RosterRole::Leader && e.ed_pk == pk && e.node_id == node);
        if !listed {
            return None;
        }
        self.leader_pk = pk;
        self.pending_transfer = None;
        Some(ControlEffect::LeaderChanged {
            new_leader_ed_pk: pk,
        })
    }

    fn check_open(&self) -> Result<(), CryptoError> {
        if self.closed {
            Err(CryptoError::Stale("group is closed"))
        } else {
            Ok(())
        }
    }
}
