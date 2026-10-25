//! Processing of signed control bodies, flooded or received as SYNC (DESIGN §4.7, §8.5,
//! §8.9 rules 6-7): monotonic acceptance, the first-ROSTER leader-key cross-check, the
//! leader's JOIN_REQ admission, and the effects of newly accepted state (close links to
//! denied peers, switch new handshakes to a new epoch, close the group, move the trust
//! anchor).

use mesh_api::{NoticeKey, Params};
use mesh_crypto::accept::ControlEffect;
use mesh_crypto::{CryptoError, JoinReq, KeySchedule, Roster};
use mesh_wire::frame::RejectReason;
use mesh_wire::mesh::PacketType;
use mesh_wire::record::SyncKind;

use super::core::GroupCore;
use super::leader::{self, Issued};
use super::state::Joiner;
use crate::engine::Ctx;
use crate::links::Prio;
use crate::session::{self, Cue, Out, view};

impl GroupCore {
    /// A complete ROSTER / REKEY / GROUP_CTL / JOIN_REQ body, flooded or received as SYNC.
    pub(crate) fn apply_body(&mut self, ctx: &mut Ctx, kind: SyncKind, body: &[u8]) {
        // Open channels have no leader: only the JOIN_REQ name announcements count.
        if kind != SyncKind::JoinReq && self.is_open() {
            return;
        }
        match kind {
            SyncKind::JoinReq => self.apply_join(ctx, body),
            SyncKind::Roster | SyncKind::Rekey | SyncKind::GroupCtl => {
                let Some((g, _)) = self.active_mut() else {
                    return;
                };
                let first_roster = g.roster().is_none();
                let res = match kind {
                    SyncKind::Roster => g.control.on_roster(body),
                    SyncKind::Rekey => {
                        let me = g.identity.clone();
                        g.control.on_rekey(body, &me)
                    }
                    _ => g.control.on_group_ctl(body),
                };
                match res {
                    Ok(effects) => self.on_effects(ctx, effects),
                    Err(CryptoError::BadSignature) if kind == SyncKind::Roster && first_roster => {
                        self.check_invite_leader(ctx, body);
                    }
                    Err(_) => {}
                }
            }
        }
    }

    /// The first ROSTER does not verify under the invite's leader key: if it is
    /// self-consistent under another key, the invite was not the leader's (§8.1).
    fn check_invite_leader(&mut self, ctx: &mut Ctx, body: &[u8]) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        let Ok(r) = Roster::decode(body) else { return };
        let Some(pk) = mesh_crypto::roster::self_signed_leader(&r) else {
            return;
        };
        if pk != g.invite_leader && !g.unverified_leader {
            g.unverified_leader = true;
            act.dirty_store = true;
            let mut p = Params::new();
            p.insert("group".into(), g.name.clone().into());
            ctx.notice(NoticeKey::InviteLeaderMismatch, p);
            ctx.mark_dirty();
        }
    }

    fn apply_join(&mut self, ctx: &mut Ctx, body: &[u8]) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        let Ok(j) = JoinReq::decode(body) else { return };
        let Ok(id) = mesh_crypto::join::verify(&j, &g.id) else {
            return;
        };
        let h = id.id_hash();
        if g.control.deny().is_denied(&h) || h == g.my_id_hash() {
            return;
        }
        // Open channels remember a bounded number of (ephemeral) identities.
        if g.open.is_some()
            && g.book.len() >= super::open::OPEN_MAX_KNOWN
            && !g.book.contains_key(&h)
        {
            return;
        }
        let fresh = g.remember(id, j.platform, &j.name);
        if fresh {
            act.dirty_store = true;
        }
        let joiner = Joiner {
            identity: id,
            platform: j.platform,
            name: j.name.clone(),
        };
        if g.is_leader() {
            if let Ok(Some(issued)) = leader::admit(g, &joiner) {
                act.dirty_store = true;
                self.publish(ctx, issued);
            }
            // Every member learns the joiner's keys (a future leader seals REKEYs to
            // its X25519 key after a handover); duplicates are idempotent everywhere.
            if fresh {
                let out = Out {
                    ptype: PacketType::JoinReq,
                    payload: body.to_vec(),
                };
                self.originate(ctx, &out);
            }
        } else {
            // Carry JOIN_REQs onward toward the leader: a joiner's (it arrived as SYNC
            // from a neighbor, or by flood from a joiner whose path to the leader may be
            // missing) and a member's re-announcement after a handover (the new leader
            // seals REKEYs with the X25519 key it carries). At most once per 10 s per
            // identity, well under the joiner's own 30 s retransmission period (§8.9
            // rule 5); the leader treats duplicates idempotently (by `id_hash`).
            let now = ctx.now;
            let due = act
                .relayed_joins
                .get(&h)
                .is_none_or(|t| now.since(*t) >= super::active::JOIN_RELAY_INTERVAL_US);
            if !due {
                return;
            }
            act.relayed_joins.insert(h, now);
            let out = Out {
                ptype: PacketType::JoinReq,
                payload: body.to_vec(),
            };
            self.originate(ctx, &out);
        }
    }

    /// Floods the bodies a leader operation issued (REKEY → ROSTER → GROUP_CTL; a
    /// transfer sends GROUP_CTL first), then applies their local effects (a removed
    /// member still receives the roster that denies it before its link closes).
    pub(crate) fn publish(&mut self, ctx: &mut Ctx, issued: Issued) {
        let Issued {
            rekey,
            roster,
            group_ctl,
            effects,
        } = issued;
        let transfer = group_ctl.is_some() && roster.is_some();
        let out = |ptype, b: Vec<u8>| Out { ptype, payload: b };
        let mut outs = Vec::new();
        if transfer && let Some(b) = group_ctl.clone() {
            outs.push(out(PacketType::GroupCtl, b));
        }
        if let Some(b) = rekey {
            outs.push(out(PacketType::Rekey, b));
        }
        if let Some(b) = roster {
            outs.push(out(PacketType::Roster, b));
        }
        if !transfer && let Some(b) = group_ctl {
            outs.push(out(PacketType::GroupCtl, b));
        }
        // Applying the effects closes links right away when a member is denied (its
        // link closes behind the REKEY and the roster that deny it) or the group closes
        // (behind the GROUP_CTL): those bodies must be on the link queues first.
        let closes = effects.iter().any(|e| match e {
            ControlEffect::RosterAccepted { newly_denied, .. } => !newly_denied.is_empty(),
            ControlEffect::GroupClosed => true,
            _ => false,
        });
        for o in &outs {
            if closes {
                self.originate_before_close(ctx, o);
            } else {
                self.originate(ctx, o);
            }
        }
        self.on_effects(ctx, effects);
    }

    /// §8.9 rule 7: effects of newly accepted control state.
    pub(crate) fn on_effects(&mut self, ctx: &mut Ctx, effects: Vec<ControlEffect>) {
        for e in effects {
            let Some((g, act)) = self.active_mut() else {
                return;
            };
            act.dirty_store = true;
            match e {
                ControlEffect::RosterAccepted { newly_denied, .. } => {
                    let me = g.my_id_hash();
                    if let Some(r) = g.roster() {
                        for en in r.entries.iter().filter(|x| x.id_hash != me) {
                            act.registry.observe(&en.id_hash, &me);
                        }
                    }
                    view::on_roster(&mut act.session, ctx, g);
                    if newly_denied.contains(&me) {
                        self.on_removed(ctx);
                        continue;
                    }
                    let denied: Vec<crate::LinkId> = act
                        .links
                        .iter()
                        .filter(|(_, l)| {
                            l.peer.is_some_and(|p| newly_denied.contains(&p.id_hash()))
                        })
                        .map(|(id, _)| *id)
                        .collect();
                    // CLOSE{5} right behind what is queued for them (the REKEY and the
                    // roster that denies them, P2): ordered by the link queue, not by a
                    // timer, and the link takes nothing new meanwhile.
                    for l in denied {
                        if let Some(rt) = act.links.get_mut(&l) {
                            rt.up = false;
                        }
                        let now = ctx.now;
                        ctx.links.close_graceful(
                            now,
                            l,
                            RejectReason::MemberRemovedOrDenied,
                            Prio::P2,
                            &mut ctx.fx,
                        );
                    }
                    for h in &newly_denied {
                        act.registry.forget(h);
                    }
                    self.refresh_caps(ctx);
                }
                ControlEffect::EpochAdvanced { epoch, secret } => {
                    g.keyring.insert(KeySchedule::new(g.id, epoch, secret));
                    act.newest_holders.clear();
                    act.session
                        .reach
                        .set_my_versions(g.control.roster_version(), g.epoch());
                    self.retarget_radio(ctx);
                    self.refresh_caps(ctx);
                }
                ControlEffect::GroupClosed => self.close_group(ctx),
                ControlEffect::TransferPending {
                    new_leader_ed_pk, ..
                } => {
                    // The new leader seals future REKEYs to every member's X25519 key,
                    // which it only knows from JOIN_REQs and direct IDPROOFs: re-announce
                    // mine once (idempotent at the leader).
                    if new_leader_ed_pk != g.identity.ed_pk() {
                        let out = Out {
                            ptype: PacketType::JoinReq,
                            payload: super::run::join_body(g, ctx.cfg.platform.caps_byte()),
                        };
                        self.originate(ctx, &out);
                    }
                }
                ControlEffect::LeaderChanged { new_leader_ed_pk } => {
                    let name = g
                        .roster()
                        .and_then(|r| r.entries.iter().find(|x| x.ed_pk == new_leader_ed_pk))
                        .map(|x| x.name.clone())
                        .unwrap_or_default();
                    session::cue::request(ctx, &Cue::LeaderChanged(name.clone()));
                    let mut p = Params::new();
                    p.insert("name".into(), name.into());
                    ctx.notice(NoticeKey::LeaderChanged, p);
                    let mine = new_leader_ed_pk == g.identity.ed_pk();
                    self.refresh_caps(ctx);
                    if mine && let Some((g, _)) = self.active_mut() {
                        // Publish a roster carrying my own invite_sig.
                        if let Ok(issued) = leader::reissue(g) {
                            self.publish(ctx, issued);
                        }
                    }
                }
            }
            ctx.mark_dirty();
        }
    }

    /// GROUP_CTL{close} accepted: mark closed, delete the secrets, stop the session's
    /// radio and audio, close the links once the GROUP_CTL left.
    fn close_group(&mut self, ctx: &mut Ctx) {
        let Some(act) = self.act.as_mut() else { return };
        let id = act.group_id;
        session::audio::end_session(&mut act.session, ctx);
        if let Some(g) = self.groups.remove(&id) {
            let mut p = Params::new();
            p.insert("group".into(), g.name.clone().into());
            ctx.notice(NoticeKey::GroupClosed, p);
            self.closed.insert(id, g.closed());
        }
        if let Some(act) = self.act.as_mut() {
            act.dirty_store = false;
        }
        self.persist(ctx, id);
        ctx.radio.stop(&mut ctx.fx);
        // The GROUP_CTL (P0) leaves first, then CLOSE{leaving}.
        self.close_all(ctx, RejectReason::Leaving, Prio::P0);
    }
}
