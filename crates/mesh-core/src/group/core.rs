//! The group module registered with the engine (`wire/group.rs`): one [`CoreModule`]
//! that owns the stored groups and, while a group session runs, its secure links, the
//! signed control state and the ride session. Secure links, groups and the session share
//! identities, keys and roster state that one module cannot reach in another, so the
//! three sub-modules of WP-10B (`secure/`, `group/`, `session/`) are driven from here.

use std::collections::BTreeMap;

use mesh_api::{CommandResult, Snapshot, UiCommand, Vehicle};
use mesh_wire::mesh::RosterRole;
use mesh_wire::record::{Caps, Record};

use super::active::Active;
use super::state::{ClosedGroup, Group};
use super::store::{self, Stored};
use crate::config::StoredGroup;
use crate::effects::Effect;
use crate::engine::{CoreModule, Ctx, EngineMode};
use crate::links::{LinkDownReason, LinkInfo, LinkSecurity};
use crate::media::{MediaIdentity, MediaRequest};
use crate::session::AudioCheck;
use crate::{Instant, LinkId, NodeId, PlatformEvent};

/// The group module.
#[derive(Debug, Default)]
pub struct GroupCore {
    loaded: bool,
    /// Time of the call being handled (for `next_timer`).
    pub(crate) now: Instant,
    pub(crate) groups: BTreeMap<[u8; 16], Group>,
    pub(crate) closed: BTreeMap<[u8; 16], ClosedGroup>,
    pub(crate) act: Option<Active>,
    pub(crate) check: AudioCheck,
    /// Display name / vehicle set without a group (used at create/join).
    pub(crate) display_name: String,
    pub(crate) vehicle: Option<Vehicle>,
}

impl GroupCore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the groups the runtime handed over in `EngineConfig::stored_groups`.
    pub(crate) fn ensure_loaded(&mut self, ctx: &mut Ctx) {
        self.now = ctx.now;
        if self.loaded {
            return;
        }
        self.loaded = true;
        for sg in &ctx.cfg.stored_groups {
            match store::decode(&sg.blob) {
                Ok(Stored::Live(g)) if g.id == sg.group_id => {
                    self.groups.insert(g.id, *g);
                }
                Ok(Stored::Closed(c)) if c.id == sg.group_id => {
                    self.closed.insert(c.id, c);
                }
                _ => ctx.fx.log(
                    crate::effects::LogLevel::Warn,
                    format!("group/{}: unreadable blob", store::hex(&sg.group_id)),
                ),
            }
        }
    }

    /// Writes `group/<hex>` and `groups/index`, and mirrors the blob into
    /// `cfg.stored_groups` so `Engine::start` finds the group.
    pub(crate) fn persist(&mut self, ctx: &mut Ctx, id: [u8; 16]) {
        let blob = if let Some(g) = self.groups.get(&id) {
            Some(store::encode(g))
        } else {
            self.closed.get(&id).map(store::encode_closed)
        };
        ctx.fx.push(Effect::Persist {
            key: store::group_key(&id),
            value: blob.clone(),
        });
        ctx.cfg.stored_groups.retain(|s| s.group_id != id);
        if let Some(blob) = blob {
            ctx.cfg
                .stored_groups
                .push(StoredGroup { group_id: id, blob });
        }
        let ids: Vec<[u8; 16]> = self
            .groups
            .keys()
            .chain(self.closed.keys())
            .copied()
            .collect();
        ctx.fx.push(Effect::Persist {
            key: store::INDEX_KEY.to_owned(),
            value: Some(store::encode_index(ids.iter())),
        });
    }

    /// Persists the active group if something changed during this call.
    pub(crate) fn flush(&mut self, ctx: &mut Ctx) {
        let Some(act) = self.act.as_mut() else { return };
        if act.dirty_store {
            act.dirty_store = false;
            let id = act.group_id;
            self.persist(ctx, id);
        }
    }

    /// End of every hook: persist, then bring the media module up to date.
    fn after_call(&mut self, ctx: &mut Ctx) {
        self.flush(ctx);
        self.sync_media(ctx);
    }

    /// The session → media seam (`MediaRequest` on the inter-module bus), posted only
    /// on change: the group identity (session start, new epoch, leader handover,
    /// regenerated identity), the members' display names (roster and name changes) and
    /// the gated talk mode while live. Nothing once the session stopped running.
    pub(crate) fn sync_media(&mut self, ctx: &mut Ctx) {
        let Some((g, act)) = self.active_mut() else {
            return;
        };
        if !runs(g, act) {
            return;
        }
        let id = MediaIdentity {
            node: act.session.me,
            leader: g.is_leader(),
            epoch: g.epoch(),
            leader_priority: g.policy.leader_priority(),
        };
        if act.media.identity != Some(id) {
            act.media.identity = Some(id);
            ctx.post_media(MediaRequest::Identity(id));
        }
        let fresh = act.media.names_key.as_ref().is_none_or(|(v, n, me)| {
            *v != g.control.roster_version() || *n != g.book.len() || *me != g.display_name
        });
        if fresh {
            act.media.names_key = Some((
                g.control.roster_version(),
                g.book.len(),
                g.display_name.clone(),
            ));
            let names = member_names(g, act.session.me);
            if names != act.media.names {
                act.media.names = names.clone();
                ctx.post_media(MediaRequest::MemberNames(names));
            }
        }
        crate::session::audio::sync_talk_mode(&mut act.session, ctx);
    }

    /// The active group and its runtime.
    pub(crate) fn active_mut(&mut self) -> Option<(&mut Group, &mut Active)> {
        let act = self.act.as_mut()?;
        let g = self.groups.get_mut(&act.group_id)?;
        Some((g, act))
    }

    /// The CAPS this node sends on new group links: the anti-entropy summary and role.
    pub(crate) fn refresh_caps(&mut self, ctx: &mut Ctx) {
        let Some((g, _)) = self.active_mut() else {
            return;
        };
        let summary = g.control.summary();
        let caps: Caps = crate::links::own_caps(
            ctx.cfg.platform,
            ctx.cfg.app_version,
            &ctx.cfg.limits,
            g.is_leader(),
            (g.epoch(), summary.1, summary.2),
        );
        ctx.links.set_own_caps(caps);
        ctx.radio.set_leader(g.is_leader());
    }
}

/// The group session runs: the group is open, I am not denied, and no member refused
/// me as removed during this session.
pub(crate) fn runs(g: &Group, act: &Active) -> bool {
    g.session_runs() && !act.refused
}

/// Display names the media module renders and shows (talkers, `memberJoined{name}`):
/// non-denied roster entries, then verified identities not listed yet, and me.
fn member_names(g: &Group, me: u16) -> Vec<(u16, String)> {
    let mut names: BTreeMap<u16, String> = BTreeMap::new();
    for j in g.book.values() {
        let h = j.identity.id_hash();
        if !j.name.is_empty() && !g.control.deny().is_denied(&h) {
            names.insert(j.identity.node_id(), j.name.clone());
        }
    }
    if let Some(r) = g.roster() {
        for e in r.entries.iter().filter(|e| e.role != RosterRole::Denied) {
            names.insert(e.node_id, e.name.clone());
        }
    }
    names.insert(me, g.display_name.clone());
    names.into_iter().collect()
}

impl CoreModule for GroupCore {
    fn name(&self) -> &'static str {
        "group"
    }

    fn on_stopping(&mut self, ctx: &mut Ctx) {
        self.ensure_loaded(ctx);
        self.leave(ctx);
        self.flush(ctx);
    }

    fn on_mode(&mut self, ctx: &mut Ctx, mode: EngineMode) {
        self.ensure_loaded(ctx);
        self.stop_session(ctx);
        if let EngineMode::Session { group_id } = mode {
            self.start_session(ctx, group_id);
        }
        self.after_call(ctx);
    }

    fn security_for(
        &mut self,
        ctx: &mut Ctx,
        link: LinkId,
        info: &LinkInfo,
    ) -> Option<Box<dyn LinkSecurity>> {
        self.ensure_loaded(ctx);
        crate::secure::admit::security_for(self, ctx, link, info)
    }

    fn on_link_established(&mut self, ctx: &mut Ctx, link: LinkId) {
        self.ensure_loaded(ctx);
        crate::secure::admit::send_idproof(self, ctx, link);
    }

    fn on_link_up(&mut self, ctx: &mut Ctx, link: LinkId) {
        self.ensure_loaded(ctx);
        self.link_up(ctx, link);
        self.after_call(ctx);
    }

    fn on_link_down(
        &mut self,
        ctx: &mut Ctx,
        link: LinkId,
        reason: LinkDownReason,
        peer: Option<NodeId>,
    ) {
        self.ensure_loaded(ctx);
        self.link_down(ctx, link, reason, peer);
        self.after_call(ctx);
    }

    fn on_caps(&mut self, ctx: &mut Ctx, link: LinkId, caps: &Caps) {
        self.ensure_loaded(ctx);
        self.peer_caps(ctx, link, caps);
        self.after_call(ctx);
    }

    fn on_record(&mut self, ctx: &mut Ctx, link: LinkId, rec: &Record<'_>) -> bool {
        self.ensure_loaded(ctx);
        let claimed = self.record(ctx, link, rec);
        self.after_call(ctx);
        claimed
    }

    fn on_timer(&mut self, ctx: &mut Ctx) {
        self.ensure_loaded(ctx);
        self.timer(ctx);
        self.after_call(ctx);
    }

    fn next_timer(&self) -> Option<Instant> {
        let act = self.act.as_ref()?;
        // A closed group or a removed member runs nothing (its links close on their
        // own once drained).
        let g = self.groups.get(&act.group_id)?;
        if !runs(g, act) {
            return None;
        }
        let mut t = act.session.next_deadline(self.now);
        if !g.in_roster() {
            t = t.min(act.join_next.max(self.now.plus_us(1)));
        }
        Some(t)
    }

    fn on_command(&mut self, ctx: &mut Ctx, cmd: &UiCommand) -> Option<CommandResult> {
        self.ensure_loaded(ctx);
        let r = self.command(ctx, cmd);
        self.after_call(ctx);
        r
    }

    fn on_event(&mut self, ctx: &mut Ctx, ev: &PlatformEvent<'_>) {
        self.ensure_loaded(ctx);
        self.event(ctx, ev);
        self.after_call(ctx);
    }

    fn on_hello_read(&mut self, ctx: &mut Ctx, disc_id: u32, bytes: &[u8]) -> bool {
        self.ensure_loaded(ctx);
        self.hello_read(ctx, disc_id, bytes)
    }

    fn fill_snapshot(&self, ctx: &Ctx, s: &mut Snapshot) {
        let Some(act) = self.act.as_ref() else { return };
        if let Some(g) = self.groups.get(&act.group_id) {
            crate::session::view::fill(&act.session, ctx, g, s);
        }
    }
}
