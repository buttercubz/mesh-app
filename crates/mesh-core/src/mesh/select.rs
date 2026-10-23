//! Link selection policy, evaluated every 2 s (DESIGN §5.7, §9.5).
//!
//! The radio policy of the link layer (WP-10A) already drives discovery and connects:
//! the discovery mode from the BLE link count against the target, one outstanding
//! connect per transport, the 8 s attempt timeout, per-peer backoff 5 s → 60 s and the
//! `LimitReached` lowering of the BLE max; the group module adds the retained-epoch
//! HELLO check and the speculative link to an epoch-ahead peer. The glue adds what
//! needs the mesh view:
//!
//! - **candidate classes and `Forming`** (§5.7, §5.8): from the reachability view it
//!   hands the radio policy the advertising tokens of unreachable roster members
//!   (connected first, then unknown tokens, then reachable members, RSSI within a
//!   class) and keeps discovery `Forming` while any member is unreachable, even at the
//!   link target;
//! - **over max** (per kind, after `LimitReached` penalties) or **too many distinct
//!   neighbors**: close the link to a neighbor that another neighbor's fresh NBR lists
//!   as `good` (redundant), worst `q` first, never one whose removal cuts off a
//!   reachable roster member (articulation check on the 2-hop graph); `CLOSE{9}`;
//! - **rejoin** after a lost link: the T3 strategy of the Phase 0 memo
//!   ([`super::ReconnectPolicy`]); with the pending reconnect, an iPhone central keeps
//!   an outstanding connect to a lost iPhone peer (`RadioCommand::ArmReconnect`).

use mesh_routing::SelectAction;
use mesh_routing::SelectInput;
use mesh_wire::frame::RejectReason;

use super::cl;
use super::policy::ReconnectPolicy;
use super::state::{LinkMeta, Mesh};
use crate::config::Platform;
use crate::effects::RadioCommand;
use crate::engine::Ctx;
use crate::links::{LinkDownReason, LinkRole, RadioAvail};

impl Mesh {
    /// Runs the selection policy when due (every 2 s).
    pub fn select_tick(&mut self, ctx: &mut Ctx) {
        if ctx.now < self.next_select {
            return;
        }
        self.next_select = ctx.now.plus_us(mesh_routing::consts::SELECT_PERIOD_US);
        let now = ctx.now.routing();
        let (reachable_members, unreachable_members) = self.reachability(ctx.now);
        self.drive_radio(ctx, &reachable_members, &unreachable_members);
        let input = SelectInput {
            // Connect candidates are chosen by the link layer's radio policy.
            candidates: Vec::new(),
            unreachable_members,
            reachable_members,
            aware_data_paths: if ctx.radio.aware == RadioAvail::On {
                self.router.cfg.limits.aware_links_max
            } else {
                0
            },
        };
        for a in self.selector.evaluate(now, &input, &self.router.neighbors) {
            if let SelectAction::CloseRedundant { link } = a
                && ctx.links.get(cl(link)).is_some()
            {
                self.stats.closed_redundant += 1;
                ctx.links
                    .close(ctx.now, cl(link), RejectReason::Redundant, &mut ctx.fx);
                // The link only flushes its CLOSE now: route around it at once.
                self.forget_link(cl(link));
                self.nbr.on_link_down();
            }
        }
    }

    /// The §5.7 hooks of the radio policy from the reachability view: the candidate
    /// classes (tokens of unreachable / reachable members, recomputed when the slot,
    /// the epoch or the view changes), the tokens of BLE neighbors (never connected
    /// again: their advertising token rotates every slot, so the token a link was
    /// opened with no longer names them), the tokens of lost members (the rejoin
    /// rules) and `Forming` while a member is unreachable.
    fn drive_radio(&mut self, ctx: &mut Ctx, reachable: &[u16], unreachable: &[u16]) {
        let slot = crate::group::radio::slot_now(ctx);
        let mut neighbors: Vec<u16> = self
            .links
            .values()
            .filter(|m| m.kind.is_ble())
            .map(|m| m.peer)
            .collect();
        neighbors.sort_unstable();
        neighbors.dedup();
        // Lost: unreachable members heard before (not members still to be met).
        let lost: Vec<u16> = unreachable
            .iter()
            .copied()
            .filter(|m| self.heard.contains_key(m))
            .collect();
        let view = (
            slot,
            self.id.epoch,
            unreachable.to_vec(),
            reachable.to_vec(),
            neighbors,
            lost,
        );
        if self.radio_view.as_ref() != Some(&view) {
            ctx.radio.set_member_tokens(
                self.id.member_tokens(slot, unreachable),
                self.id.member_tokens(slot, reachable),
            );
            ctx.radio
                .set_neighbor_tokens(self.id.member_tokens(slot, &view.4));
            ctx.radio
                .set_lost_tokens(self.id.member_tokens(slot, &view.5));
            self.radio_view = Some(view);
        }
        if ctx.radio.set_force_forming(!unreachable.is_empty()) {
            ctx.refresh_radio();
        }
    }

    /// The rejoin strategy after a lost link (§9.5).
    pub fn arm_reconnect(&mut self, ctx: &mut Ctx, m: &LinkMeta, reason: LinkDownReason) {
        if self.policy.reconnect != ReconnectPolicy::PendingConnect
            || ctx.cfg.platform != Platform::Ios
            || !m.kind.is_ble()
            || m.role != LinkRole::Initiator
            || !matches!(reason, LinkDownReason::Timeout | LinkDownReason::Error(_))
        {
            return;
        }
        // Only toward iPhones: Android peers advertise from their FGS and are found again
        // by normal scanning; a still-linked peer needs nothing.
        let ios_peer = m
            .peer_caps
            .is_none_or(|c| c.platform == mesh_wire::record::Caps::PLATFORM_IOS);
        let still_linked = self
            .links
            .values()
            .any(|x| x.peer == m.peer && x.kind == m.kind);
        let Some(disc_id) = m.disc_id else { return };
        if !ios_peer || still_linked {
            return;
        }
        ctx.fx.diag(mesh_lab::ridelog::DiagEvent::ReconnectArmed {
            peer: m.peer,
            until_s: u32::from(self.policy.reconnect_hold_s),
        });
        ctx.fx.radio(RadioCommand::ArmReconnect {
            peer_ref: disc_id,
            until_s: self.policy.reconnect_hold_s,
        });
    }
}
