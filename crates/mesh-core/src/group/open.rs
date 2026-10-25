//! Open channels ("open world"): public channels 1-9 riders join without an invite.
//!
//! An open channel is a [`Group`] in a restricted mode, so the secure links, discovery,
//! session, media and mesh glue run unchanged:
//!
//! - keys, `group_id` and discovery tags are derived from the public channel context
//!   (`mesh_crypto::open_channel`): every phone on channel N derives the same PSK and
//!   advertises the same rotating tag, so they find and admit each other;
//! - there is no leader (`OPEN_NO_LEADER`): ROSTER / REKEY / GROUP_CTL are ignored, the
//!   ride is "riding" from the start, nobody can remove, lock or close anyone, and a
//!   peer's `REJECT{5}` never ends my session;
//! - every session uses a fresh identity (no stable identity on air); names come from
//!   the signed JOIN_REQ each member floods (optional, may be empty);
//! - policy: voice TTL 2, talker cap 2, a per-origin packet budget and a bound on the
//!   identities remembered;
//! - nothing is persisted: the group lives in memory and in `cfg.stored_groups` (which
//!   `Engine::start` and the mesh glue read) until the session ends.
//!
//! Mute and block are local to this device and never sent: a muted member's VOICE and
//! TALK are dropped before the media module and the mesh glue see them (so this node
//! does not relay them either); a blocked member is muted and refused as a direct
//! neighbor. They work in private groups too.

use std::collections::{BTreeMap, BTreeSet};

use mesh_api::{CommandResult, ErrorCode, MemberFlag, Snapshot, TalkMode, UiCommand, Vehicle};
use mesh_wire::mesh::{MeshHeader, PacketType};

use super::core::GroupCore;
use super::state::Group;
use super::store;
use crate::config::StoredGroup;
use crate::engine::{Ctx, EngineMode};
use crate::{Instant, LinkId};

/// Concurrent talkers on an open channel (`group_cap_override`, §6.3).
pub const OPEN_TALKER_CAP: u8 = 2;
/// Identities (names) remembered per open-channel session.
pub const OPEN_MAX_KNOWN: usize = 64;
/// MESH records accepted per origin per second on an open channel (before dedup: the
/// copies relayed by several neighbors count; one talker sends ≤ 50 VOICE/s).
pub const OPEN_ORIGIN_MAX_PER_S: u32 = 200;
/// Origins tracked by the per-origin limiter before stale entries are dropped.
const LIMITER_MAX_ORIGINS: usize = 256;
const SECOND_US: u64 = 1_000_000;

/// Local (never sent) mute and block lists of the running session.
#[derive(Debug, Default)]
pub struct LocalFilter {
    pub muted: BTreeSet<u16>,
    pub blocked: BTreeSet<u16>,
    /// `id_hash` of blocked members whose identity is known (refused at IDPROOF).
    pub blocked_ids: BTreeSet<[u8; 16]>,
    /// Per-origin record budget (open channels): `origin → (window start, count)`.
    limiter: BTreeMap<u16, (Instant, u32)>,
}

impl LocalFilter {
    /// `true` when voice of `origin` must not reach media nor be relayed.
    pub fn silenced(&self, origin: u16) -> bool {
        self.muted.contains(&origin) || self.blocked.contains(&origin)
    }

    /// Counts one record of `origin`; `false` once it is over its budget this second.
    pub fn admit(&mut self, now: Instant, origin: u16) -> bool {
        if self.limiter.len() >= LIMITER_MAX_ORIGINS && !self.limiter.contains_key(&origin) {
            self.limiter
                .retain(|_, (start, _)| now.since(*start) < SECOND_US);
            if self.limiter.len() >= LIMITER_MAX_ORIGINS {
                return false;
            }
        }
        let e = self.limiter.entry(origin).or_insert((now, 0));
        if now.since(e.0) >= SECOND_US {
            *e = (now, 0);
        }
        e.1 = e.1.saturating_add(1);
        e.1 <= OPEN_ORIGIN_MAX_PER_S
    }
}

impl GroupCore {
    /// Open-channel and local-filter commands; `None` for anything else.
    pub(crate) fn open_command(&mut self, ctx: &mut Ctx, cmd: &UiCommand) -> Option<CommandResult> {
        match cmd {
            UiCommand::JoinOpenChannel {
                channel,
                display_name,
            } => Some(self.join_open(ctx, *channel, display_name)),
            UiCommand::SetMemberMuted { node_id, muted } => {
                Some(self.set_filter(ctx, *node_id, Some(*muted), None))
            }
            UiCommand::SetMemberBlocked { node_id, blocked } => {
                Some(self.set_filter(ctx, *node_id, None, Some(*blocked)))
            }
            _ => None,
        }
    }

    /// `joinOpenChannel`: prepares the in-memory group of `channel` with a fresh
    /// identity; the UI then starts it with `startEngine({ groupId })`.
    fn join_open(&mut self, ctx: &mut Ctx, channel: u8, display_name: &str) -> CommandResult {
        if !mesh_crypto::open_channel::valid_channel(channel) {
            return CommandResult::error(ErrorCode::InvalidParams);
        }
        if ctx.mode != EngineMode::Idle {
            return CommandResult::error(ErrorCode::WrongMode);
        }
        let name = if display_name.is_empty() {
            self.display_name.clone()
        } else {
            mesh_wire::name::truncate_graphemes(display_name, mesh_wire::name::DISPLAY_NAME_MAX)
                .to_owned()
        };
        let vehicle = self.vehicle.unwrap_or(Vehicle::Moto);
        let platform = ctx.cfg.platform.caps_byte();
        let Some(g) = Group::open_channel(channel, &name, vehicle, platform, &mut ctx.rng) else {
            return CommandResult::error(ErrorCode::InvalidParams);
        };
        // At most one open channel at a time.
        self.drop_open(ctx, None);
        let id = g.id;
        mirror(ctx, &g);
        self.groups.insert(id, g);
        CommandResult::GroupId(mesh_api::OkGroupId {
            ok: mesh_api::OkTrue,
            group_id: store::hex(&id),
        })
    }

    /// `setMemberMuted` / `setMemberBlocked` (local only).
    fn set_filter(
        &mut self,
        ctx: &mut Ctx,
        node: u16,
        mute: Option<bool>,
        block: Option<bool>,
    ) -> CommandResult {
        let Some((g, act)) = self.active_mut() else {
            return CommandResult::error(ErrorCode::NoSession);
        };
        if node == g.my_node_id() {
            return CommandResult::error(ErrorCode::InvalidParams);
        }
        let f = &mut act.filter;
        if let Some(m) = mute {
            if m {
                f.muted.insert(node);
            } else {
                f.muted.remove(&node);
            }
        }
        if let Some(b) = block {
            let ids: Vec<[u8; 16]> = act
                .links
                .values()
                .filter_map(|l| l.peer)
                .chain(g.book.values().map(|j| j.identity))
                .filter(|p| p.node_id() == node)
                .map(|p| p.id_hash())
                .collect();
            let f = &mut act.filter;
            if b {
                f.blocked.insert(node);
                f.blocked_ids.extend(ids);
            } else {
                f.blocked.remove(&node);
                for h in ids {
                    f.blocked_ids.remove(&h);
                }
            }
            if b {
                // Refuse the blocked member as a direct neighbor from now on.
                let links: Vec<LinkId> = act
                    .links
                    .iter()
                    .filter(|(_, l)| l.peer.is_some_and(|p| p.node_id() == node))
                    .map(|(id, _)| *id)
                    .collect();
                for l in links {
                    if let Some(rt) = act.links.get_mut(&l) {
                        rt.up = false;
                    }
                    let now = ctx.now;
                    ctx.links
                        .close(now, l, mesh_wire::frame::RejectReason::Leaving, &mut ctx.fx);
                }
            }
        }
        ctx.mark_dirty();
        CommandResult::done()
    }

    /// Removes every open-channel group except `keep` (session ended, another channel
    /// chosen) and gives the talker cap back.
    pub(crate) fn drop_open(&mut self, ctx: &mut Ctx, keep: Option<[u8; 16]>) {
        let gone: Vec<[u8; 16]> = self
            .groups
            .values()
            .filter(|g| g.open.is_some() && Some(g.id) != keep)
            .map(|g| g.id)
            .collect();
        for id in gone {
            self.groups.remove(&id);
            ctx.cfg.stored_groups.retain(|s| s.group_id != id);
        }
        let keep_open =
            keep.is_some_and(|id| self.groups.get(&id).is_some_and(|g| g.open.is_some()));
        if !keep_open && let Some(saved) = self.saved_cap.take() {
            ctx.cfg.limits.routing.group_cap_override = saved;
        }
    }

    /// Session start of an open channel: riding at once (no leader starts the ride), PTT
    /// by default, talker cap 2.
    pub(crate) fn start_open(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        if g.open.is_none() {
            return;
        }
        act.session.ride = mesh_api::RidePhase::Riding;
        act.session.mode = Some(TalkMode::Ptt);
        let lim = &mut ctx.cfg.limits.routing;
        if self.saved_cap.is_none() {
            self.saved_cap = Some(lim.group_cap_override);
        }
        lim.group_cap_override = Some(
            lim.group_cap_override
                .map_or(OPEN_TALKER_CAP, |c| c.min(OPEN_TALKER_CAP)),
        );
    }

    /// The active session is an open channel.
    pub(crate) fn is_open(&self) -> bool {
        self.act
            .as_ref()
            .and_then(|a| self.groups.get(&a.group_id))
            .is_some_and(|g| g.open.is_some())
    }

    /// Local filtering of a MESH record from an Up link. Returns `true` when the record
    /// is dropped here (claimed: neither media nor the mesh glue see it).
    pub(crate) fn filter_mesh(&mut self, ctx: &Ctx, bytes: &[u8]) -> bool {
        let open = self.is_open();
        let Some(act) = self.act.as_mut() else {
            return false;
        };
        let Ok(h) = MeshHeader::decode(bytes) else {
            return false;
        };
        if open && !act.filter.admit(ctx.now, h.origin) {
            return true;
        }
        act.filter.silenced(h.origin) && matches!(h.ptype, PacketType::Voice | PacketType::Talk)
    }

    /// Open channel number and the local mute/block flags in the snapshot.
    pub(crate) fn fill_open(&self, s: &mut Snapshot) {
        let Some(act) = self.act.as_ref() else { return };
        let open = self.groups.get(&act.group_id).and_then(|g| g.open);
        if let Some(info) = s.session.as_mut() {
            info.open_channel = open;
        }
        for m in &mut s.members {
            if act.filter.muted.contains(&m.node_id) && !m.flags.contains(&MemberFlag::Muted) {
                m.flags.push(MemberFlag::Muted);
            }
            if act.filter.blocked.contains(&m.node_id) && !m.flags.contains(&MemberFlag::Blocked) {
                m.flags.push(MemberFlag::Blocked);
            }
        }
    }
}

/// Mirrors an open group into `cfg.stored_groups` (what `Engine::start` and the mesh
/// glue read) without any `Persist` effect.
pub(crate) fn mirror(ctx: &mut Ctx, g: &Group) {
    ctx.cfg.stored_groups.retain(|s| s.group_id != g.id);
    ctx.cfg.stored_groups.push(StoredGroup {
        group_id: g.id,
        blob: store::encode(g),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_origin_budget_resets_every_second() {
        let mut f = LocalFilter::default();
        let t = Instant::from_ms(10);
        for _ in 0..OPEN_ORIGIN_MAX_PER_S {
            assert!(f.admit(t, 7));
        }
        assert!(!f.admit(t, 7), "over budget");
        assert!(f.admit(t, 8), "other origins keep their own budget");
        assert!(f.admit(t.plus_ms(1_000), 7), "new window");
    }

    #[test]
    fn muted_and_blocked_are_silenced() {
        let mut f = LocalFilter::default();
        f.muted.insert(1);
        f.blocked.insert(2);
        assert!(f.silenced(1) && f.silenced(2) && !f.silenced(3));
    }

    #[test]
    fn limiter_map_is_bounded() {
        let mut f = LocalFilter::default();
        let t = Instant::from_ms(0);
        for o in 0..LIMITER_MAX_ORIGINS as u16 {
            assert!(f.admit(t, o));
        }
        assert!(!f.admit(t, 9_999), "full within the window");
        assert!(f.admit(t.plus_ms(1_001), 9_999), "stale origins dropped");
        assert!(f.limiter.len() <= LIMITER_MAX_ORIGINS);
    }
}
