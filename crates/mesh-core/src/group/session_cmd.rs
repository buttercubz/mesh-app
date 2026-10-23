//! Session and leader commands of a running group session, and the platform events the
//! session reacts to (audio IO, radio state).

use mesh_api::{CommandResult, ErrorCode, MemberRole, NodeRef, OkAccessLoss, OkTrue, UiCommand};

use super::core::GroupCore;
use super::leader;
use crate::PlatformEvent;
use crate::engine::{Ctx, EngineMode};
use crate::session::{self, ride::Signer};

/// Hours → unix minutes of a new invite expiry.
fn expiry(unix_s: u64, hours: Option<u32>) -> Option<u32> {
    hours.map(|h| ((unix_s / 60) + u64::from(h) * 60).min(u64::from(u32::MAX)) as u32)
}

impl GroupCore {
    pub(crate) fn session_command(
        &mut self,
        ctx: &mut Ctx,
        cmd: &UiCommand,
    ) -> Option<CommandResult> {
        if !matches!(ctx.mode, EngineMode::Session { .. }) {
            return None;
        }
        let closed = self.act.is_some() && self.active_mut().is_none();
        if closed {
            return Some(CommandResult::error(ErrorCode::GroupClosed));
        }
        // Removed (denied, or refused as removed): nothing may go live again.
        let stopped = self
            .active_mut()
            .is_some_and(|(g, act)| !super::core::runs(g, act));
        if stopped && matches!(cmd, UiCommand::GoLive {} | UiCommand::ResumeMic {}) {
            return Some(CommandResult::error(ErrorCode::NoSession));
        }
        let check = &mut self.check;
        let act = self.act.as_mut()?;
        let s = &mut act.session;
        let r = match cmd {
            // Gesture commands: validated and recorded here, then passed on: the audio
            // part is the media module's (see `session::audio`).
            UiCommand::GoLive {} => return session::audio::go_live(s, check, ctx).answer(),
            UiCommand::ResumeMic {} => return session::audio::resume_mic(s, ctx).answer(),
            // Answered here: the media module gets the gated mode on the bus.
            UiCommand::SetTalkMode { mode } => session::ride::set_talk_mode(s, ctx, *mode),
            UiCommand::SetShareDistance { enabled } => session::ride::set_share(s, ctx, *enabled),
            UiCommand::RideControl { op } => {
                let (g, act) = self.active_mut()?;
                let signer = Signer {
                    identity: &g.identity,
                    group_id: &g.id,
                    epoch: g.epoch(),
                    leader: g.is_leader(),
                };
                let (r, out) = session::ride::ride_control(&mut act.session, ctx, *op, &signer);
                if let Some(o) = out {
                    self.originate(ctx, &o);
                }
                r
            }
            UiCommand::SetInviteExpiry { hours } => {
                let unix = super::radio::unix_s(ctx);
                let (g, act) = self.active_mut()?;
                if !g.is_leader() {
                    return Some(CommandResult::error(ErrorCode::NotLeader));
                }
                g.expires_at = expiry(unix, *hours);
                act.dirty_store = true;
                CommandResult::done()
            }
            UiCommand::PreviewAccessLoss {} => self.preview_access_loss(ctx),
            UiCommand::RemoveMember { node_id } => {
                self.leader_op(ctx, |g, rng| leader::remove(g, *node_id, rng))
            }
            UiCommand::LockGroup {} => {
                let r = self.leader_op(ctx, leader::lock);
                if r.is_ok() {
                    ctx.notice(mesh_api::NoticeKey::GroupLocked, mesh_api::Params::new());
                }
                r
            }
            UiCommand::CloseGroup {} => self.leader_op(ctx, |g, _| leader::close(g)),
            UiCommand::TransferLeader { node_id } => {
                if self.active_mut().is_some_and(|(g, _)| !g.is_leader()) {
                    return Some(CommandResult::error(ErrorCode::NotLeader));
                }
                let present = self.act.as_ref().is_some_and(|a| {
                    a.link_to(*node_id).is_some()
                        || a.session.reach.reachable(ctx.now.routing(), *node_id)
                });
                if !present {
                    return Some(CommandResult::error(ErrorCode::UnknownMember));
                }
                self.leader_op(ctx, |g, _| leader::transfer(g, *node_id))
            }
            _ => return None,
        };
        Some(r)
    }

    /// Runs a leader operation and floods what it issued.
    fn leader_op(
        &mut self,
        ctx: &mut Ctx,
        op: impl FnOnce(
            &mut super::state::Group,
            &mut rand_chacha::ChaCha20Rng,
        ) -> Result<leader::Issued, mesh_crypto::CryptoError>,
    ) -> CommandResult {
        let Some((g, act)) = self.active_mut() else {
            return CommandResult::error(ErrorCode::NoSession);
        };
        if !g.is_leader() {
            return CommandResult::error(ErrorCode::NotLeader);
        }
        match op(g, &mut ctx.rng) {
            Ok(issued) => {
                act.dirty_store = true;
                self.publish(ctx, issued);
                CommandResult::done()
            }
            Err(mesh_crypto::CryptoError::Invalid(_)) => {
                CommandResult::error(ErrorCode::UnknownMember)
            }
            Err(_) => CommandResult::error(ErrorCode::Internal),
        }
    }

    /// `previewAccessLoss` (§8.6 step 5): members seen in PRESENCE but not in the roster
    /// (no sealed REKEY) lose access on lock/remove.
    fn preview_access_loss(&mut self, ctx: &mut Ctx) -> CommandResult {
        let Some((g, act)) = self.active_mut() else {
            return CommandResult::error(ErrorCode::NoSession);
        };
        if g.role() != MemberRole::Leader {
            return CommandResult::error(ErrorCode::NotLeader);
        }
        let listed: Vec<u16> = g.roster_indices().into_iter().map(|(n, _)| n).collect();
        let now = ctx.now.routing();
        let mut unconfirmed: Vec<NodeRef> = act
            .session
            .reach
            .members()
            .filter(|m| !listed.contains(&m.node_id))
            .filter(|m| act.session.reach.reachable(now, m.node_id))
            .map(|m| NodeRef {
                node_id: m.node_id,
                name: g.name_of(m.node_id).unwrap_or_default(),
            })
            .collect();
        // Direct neighbors admitted with the current PSK but not listed yet.
        for l in act.links.values().filter(|l| l.up) {
            if let Some(p) = l.peer
                && !listed.contains(&p.node_id())
                && !unconfirmed.iter().any(|u| u.node_id == p.node_id())
            {
                unconfirmed.push(NodeRef {
                    node_id: p.node_id(),
                    name: g.name_of(p.node_id()).unwrap_or_default(),
                });
            }
        }
        CommandResult::AccessLoss(OkAccessLoss {
            ok: OkTrue,
            unconfirmed,
        })
    }

    /// Platform events the session reacts to.
    pub(crate) fn event(&mut self, ctx: &mut Ctx, ev: &PlatformEvent<'_>) {
        let Some(act) = self.act.as_mut() else { return };
        let s = &mut act.session;
        match ev {
            PlatformEvent::AudioIo(io) => {
                session::audio::on_audio_io(s, ctx, *io);
            }
            PlatformEvent::RadioState { ble, .. } => {
                if session::audio::on_radio_state(s, ctx, *ble)
                    && *ble == crate::links::RadioAvail::On
                {
                    // Back on: rebuild HELLO/discovery so the links re-form (§9.5).
                    self.retarget_radio(ctx);
                }
            }
            PlatformEvent::AudioRoute(r) => {
                session::audio::on_route(s, ctx, r);
                ctx.mark_dirty();
            }
            PlatformEvent::Platform(p) => {
                session::audio::on_battery(s, ctx, p.battery, p.charging);
                ctx.mark_dirty();
            }
            _ => {}
        }
    }
}
