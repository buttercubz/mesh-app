//! Link security setup and IDPROOF admission (DESIGN §4.4, §5.7, §8.2, §8.4, §8.6).
//!
//! After the Noise handshake each side sends IDPROOF as its first SEC record. The
//! receiver verifies it against the handshake hash and the Noise-authenticated static
//! key, then runs the admission checks in order: deny list (`REJECT{5}`), own identity,
//! retained-epoch admission (an older epoch only for non-denied roster members not yet
//! known to hold the newest one, `REJECT{2}`), `node_id` collision (`REJECT{8}`; the
//! larger `id_hash` regenerates), and duplicate-link resolution (keep the link whose
//! initiator has the lower `node_id`, `CLOSE{4}` the other). Only then is the link Up.
//!
//! A refusal goes out behind my own IDPROOF (ordered close, never a timer), so the
//! refused side reads both: the reason reaches its group module as
//! `LinkDownReason::PeerRejected`. `{8}` without a direct collision there means a third
//! node's identity holds its `node_id` (it regenerates); `{5}` means it was removed.

use std::cell::Cell;
use std::rc::Rc;

use mesh_crypto::idproof::{self, IdProof};
use mesh_crypto::keyring::admit_epoch;
use mesh_crypto::{CollisionOutcome, CryptoError, PublicIdentity};
use mesh_wire::frame::RejectReason;
use mesh_wire::record::Record;

use super::{EpochChoice, SecureLink, role_of};
use crate::engine::Ctx;
use crate::group::active::LinkRt;
use crate::group::core::GroupCore;
use crate::links::{LinkInfo, LinkRole, LinkSecurity, LinkState, Prio};
use crate::{LinkId, NodeId};

/// Supplies the security of a new group link (`CoreModule::security_for`).
pub fn security_for(
    core: &mut GroupCore,
    ctx: &mut Ctx,
    link: LinkId,
    info: &LinkInfo,
) -> Option<Box<dyn LinkSecurity>> {
    let (g, act) = core.active_mut()?;
    if !crate::group::core::runs(g, act) {
        return None;
    }
    let slot = Rc::new(Cell::new(None));
    let mut seed = [0u8; 32];
    ctx.fill(&mut seed);
    let mut rt = LinkRt::new(slot.clone(), info.role, info.kind);
    let sec = match info.role {
        LinkRole::Initiator => {
            let disc = info.disc_id;
            let spec = disc.and_then(|d| act.spec_pending.remove(&d));
            rt.speculative = spec;
            let choice = match (spec, disc.and_then(|d| act.hellos.get(&d))) {
                (None, Some(h)) => EpochChoice::Hint(h.epoch_lo),
                _ => EpochChoice::Newest,
            };
            SecureLink::initiator(&g.identity, &g.keyring, choice, seed, slot)
        }
        LinkRole::Responder => SecureLink::responder(&g.identity, &g.keyring, ctx.now, seed, slot),
    };
    act.links.insert(link, rt);
    Some(Box::new(sec))
}

/// Keys are established: send my IDPROOF as the first SEC record.
pub fn send_idproof(core: &mut GroupCore, ctx: &mut Ctx, link: LinkId) {
    let Some((g, act)) = core.active_mut() else {
        return;
    };
    let Some(rt) = act.links.get(&link) else {
        return;
    };
    let Some(info) = rt.slot.get() else {
        return;
    };
    let proof = idproof::create(&g.identity, role_of(rt.role), &info.handshake_hash);
    ctx.links
        .send_record(link, Prio::P0, &Record::IdProof(proof), None);
}

/// Refuses `link` with `reason` (`CLOSE` once established, `REJECT` before), after my
/// own IDPROOF (P0) if it is still queued: the peer then sees both, in order.
fn reject(ctx: &mut Ctx, link: LinkId, reason: RejectReason) {
    let now = ctx.now;
    ctx.links
        .close_graceful(now, link, reason, Prio::P0, &mut ctx.fx);
}

/// The peer's IDPROOF on a link in the IdProof state. Returns the admitted identity.
pub fn on_idproof(
    core: &mut GroupCore,
    ctx: &mut Ctx,
    link: LinkId,
    proof: &IdProof,
) -> Option<PublicIdentity> {
    if ctx.links.get(link).map(|l| l.state) != Some(LinkState::IdProof) {
        return None;
    }
    let mut regenerate = false;
    let mut direct = false;
    let outcome = {
        let (g, act) = core.active_mut()?;
        let rt = act.links.get(&link)?;
        let info = rt.slot.get()?;
        let verified = idproof::verify(
            proof,
            role_of(rt.role).peer(),
            &info.handshake_hash,
            &info.remote_static,
        );
        let checked = verified.and_then(|peer| {
            let h = peer.id_hash();
            g.control.deny().check(&h)?;
            if h == g.my_id_hash() {
                return Err(CryptoError::Invalid("peer presents my own identity"));
            }
            // A stranger (old QR, not in the roster) is refused before its node_id is
            // recorded, so it can never cause a collision verdict.
            admit_epoch(
                info.epoch,
                g.epoch(),
                &h,
                g.roster(),
                g.control.deny(),
                &act.newest_holders,
            )
            .map_err(|e| match e {
                CryptoError::Denied => e,
                _ => CryptoError::WrongGroup,
            })?;
            match act.registry.observe(&h, &g.my_id_hash()) {
                CollisionOutcome::None => {}
                CollisionOutcome::RejectSelfRegenerates => {
                    // The leader's key is the trust anchor: it never regenerates.
                    direct = true;
                    regenerate = !g.is_leader();
                    return Err(CryptoError::NodeIdCollision);
                }
                CollisionOutcome::RejectPeerRegenerates if &peer.ed_pk == g.control.leader_pk() => {
                    // Colliding with the leader: I regenerate even with the smaller
                    // `id_hash`, otherwise I could never join (§8.2).
                    direct = true;
                    regenerate = true;
                    return Err(CryptoError::NodeIdCollision);
                }
                CollisionOutcome::RejectPeerRegenerates => {
                    direct = true;
                    return Err(CryptoError::NodeIdCollision);
                }
                // Two other identities collide: the newcomer is refused, and its `{8}`
                // tells it to regenerate (it cannot see the identity it collides with).
                CollisionOutcome::RejectNewcomer => return Err(CryptoError::NodeIdCollision),
            }
            Ok(peer)
        });
        match checked {
            Ok(peer) => {
                if info.epoch == g.epoch() {
                    act.newest_holders.insert(peer.id_hash());
                }
                Ok((peer, info.epoch))
            }
            Err(e) => Err(e.reject_reason().unwrap_or(RejectReason::HandshakeFailure)),
        }
    };
    if direct && let Some(rt) = core.active_mut().and_then(|(_, a)| a.links.get_mut(&link)) {
        rt.collided = true;
    }
    let (peer, _epoch) = match outcome {
        Ok(x) => x,
        Err(RejectReason::NodeIdCollision) if !regenerate => {
            // The peer must see my IDPROOF to learn of the collision (the larger
            // `id_hash` regenerates): `CLOSE{8}` goes out behind it, in order.
            reject(ctx, link, RejectReason::NodeIdCollision);
            return None;
        }
        Err(reason) => {
            reject(ctx, link, reason);
            if regenerate {
                core.regenerate_identity(ctx);
            }
            return None;
        }
    };
    if !resolve_duplicate(core, ctx, link, &peer) {
        return None;
    }
    let (g, act) = core.active_mut()?;
    let platform = act
        .links
        .get(&link)
        .and_then(|_| ctx.links.get(link))
        .and_then(|l| l.info.peer_platform_hint)
        .map_or(0, |p| u8::from(p == mesh_api::PeerPlatform::Android));
    if g.remember_identity(peer, platform) {
        act.dirty_store = true;
    }
    if let Some(rt) = act.links.get_mut(&link) {
        rt.peer = Some(peer);
        rt.up = true;
    }
    ctx.links.confirm_up(ctx.now, link, NodeId(peer.node_id()));
    Some(peer)
}

/// Duplicate links to the same node and transport (§5.7): keep the link whose initiator
/// has the lower `node_id`, close the other with `CLOSE{4}`. Returns `false` when the
/// new link is the one closed.
fn resolve_duplicate(
    core: &mut GroupCore,
    ctx: &mut Ctx,
    link: LinkId,
    peer: &PublicIdentity,
) -> bool {
    let Some((g, act)) = core.active_mut() else {
        return false;
    };
    let me = g.my_node_id();
    let Some(new) = act.links.get(&link) else {
        return false;
    };
    let kind = new.kind.routing();
    let new_init = match new.role {
        LinkRole::Initiator => me,
        LinkRole::Responder => peer.node_id(),
    };
    let existing: Vec<(LinkId, u16)> = act
        .links
        .iter()
        .filter(|(id, l)| {
            **id != link && l.up && l.kind.routing() == kind && l.peer.as_ref() == Some(peer)
        })
        .filter_map(|(id, l)| l.initiator_node(me).map(|n| (*id, n)))
        .collect();
    let mut keep_new = true;
    for (old, old_init) in existing {
        // Lower initiator node_id wins; on a tie the existing link stays.
        if new_init < old_init {
            reject(ctx, old, RejectReason::DuplicateLink);
            if let Some(rt) = act.links.get_mut(&old) {
                rt.up = false;
            }
        } else {
            keep_new = false;
        }
    }
    if !keep_new {
        reject(ctx, link, RejectReason::DuplicateLink);
    }
    keep_new
}
