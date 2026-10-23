//! Rotating on-air identifiers and the HELLO check in group mode (DESIGN §8.3, §8.7,
//! §9.3): the advertised `group_tag` / `node_token` of the newest epoch for the current
//! 10-minute slot (`SetHello` / `SetDiscovery` refreshed at every slot rollover and
//! epoch change), and the initiator-side HELLO check against every retained epoch
//! (t10 ± 1), plus the speculative link toward a peer whose `epoch_lo` is 1-4 epochs
//! ahead (at most one per 30 s, 60 s per-peer backoff).

use mesh_crypto::schedule::t10;
use mesh_wire::hello::Hello;

use super::active::{SPECULATIVE_BACKOFF_US, SPECULATIVE_INTERVAL_US};
use super::core::GroupCore;
use crate::engine::Ctx;
use crate::links::RadioTarget;

/// Wall-clock seconds now (anchored by the platform samples).
pub fn unix_s(ctx: &Ctx) -> u64 {
    ctx.device.wall_ms(ctx.now) / 1_000
}

/// Current 10-minute discovery slot.
pub fn slot_now(ctx: &Ctx) -> u32 {
    t10(unix_s(ctx))
}

impl GroupCore {
    /// (Re)starts discovery with the newest epoch's tag and my token for this slot.
    pub(crate) fn retarget_radio(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        if !super::core::runs(g, act) {
            return;
        }
        let slot = slot_now(ctx);
        let sched = g.keyring.newest();
        let tag = sched.group_tag(slot);
        let token = sched.node_token(slot, &g.my_id_hash());
        act.t10 = slot;
        act.radio_epoch = sched.epoch();
        ctx.radio.set_leader(g.is_leader());
        ctx.radio.start(
            RadioTarget::Group {
                group_tag: tag,
                epoch: sched.epoch(),
            },
            token,
            &mut ctx.fx,
        );
        ctx.refresh_radio();
    }

    /// `HelloRead` in group mode. Returns `true` when this module answered (through the
    /// radio policy, so its connect bookkeeping stays consistent).
    pub(crate) fn hello_read(&mut self, ctx: &mut Ctx, disc_id: u32, bytes: &[u8]) -> bool {
        let unix = unix_s(ctx);
        let now = ctx.now;
        let limits = ctx.cfg.limits;
        let Some((g, act)) = self.active_mut() else {
            return false;
        };
        if !super::core::runs(g, act) {
            return false;
        }
        let Ok(h) = Hello::decode(bytes) else {
            return false;
        };
        if h.flags.lab() {
            return false;
        }
        act.hellos.insert(disc_id, h);
        let current = g.keyring.newest().group_tag(act.t10);
        let retained = g.keyring.match_group_tag(&h.group_tag, unix).is_some();
        let speculative = !retained
            && h.speculative_candidate(g.epoch())
            && act
                .spec_last
                .is_none_or(|t| now.since(t) >= SPECULATIVE_INTERVAL_US)
            && act
                .spec_backoff
                .get(&h.node_token)
                .is_none_or(|t| now >= *t);
        if !(retained || speculative) {
            return false;
        }
        if speculative {
            act.spec_last = Some(now);
            act.spec_backoff
                .insert(h.node_token, now.plus_us(SPECULATIVE_BACKOFF_US));
            act.spec_pending.insert(disc_id, h.node_token);
        }
        // The radio policy compares the tag with the current one: hand it the HELLO
        // with the verified tag substituted so it opens the link and keeps its state.
        let mut checked = h;
        checked.group_tag = current;
        let Ok(rewritten) = checked.encode() else {
            return false;
        };
        ctx.radio
            .on_hello_read(now, disc_id, &rewritten, &limits, &mut ctx.fx);
        true
    }
}
