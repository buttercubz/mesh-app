//! UI commands of groups and the session (DESIGN §3.7, §1.3, §8): createGroup,
//! parseInvite, joinGroup, listGroups, forgetGroup, getInvite, setInviteExpiry,
//! setDisplayName, setVehicle, goLive, resumeMic, audioCheck, setTalkMode, rideControl,
//! setShareDistance, previewAccessLoss, removeMember, lockGroup, closeGroup,
//! transferLeader. Session commands without a running group session return `None`, so
//! the dispatcher answers `noSession`.

use mesh_api::{
    AudioCheckOp, CommandResult, ErrorCode, GroupSummary, OkGroupId, OkGroups, OkInvite,
    OkInviteInfo, OkTrue, UiCommand, Vehicle,
};
use mesh_crypto::Invite;
use mesh_wire::mesh::PacketType;

use super::core::GroupCore;
use super::leader;
use super::png::qr_data_url;
use super::run::join_body;
use super::state::Group;
use super::store;
use crate::engine::Ctx;
use crate::session::Out;

/// `getInvite`: the QR hides after 60 s (§1.3 F3).
pub const QR_HIDE_AFTER_MS: u32 = 60_000;
/// Pixels per QR module in the rendered PNG.
pub const QR_SCALE: usize = 4;

fn group_id_result(id: &[u8; 16]) -> CommandResult {
    CommandResult::GroupId(OkGroupId {
        ok: OkTrue,
        group_id: store::hex(id),
    })
}

impl GroupCore {
    pub(crate) fn command(&mut self, ctx: &mut Ctx, cmd: &UiCommand) -> Option<CommandResult> {
        match cmd {
            UiCommand::CreateGroup { name, vehicle } => Some(self.create(ctx, name, *vehicle)),
            UiCommand::ParseInvite { url } => Some(self.parse_invite(ctx, url)),
            UiCommand::JoinGroup { url, display_name } => Some(self.join(ctx, url, display_name)),
            UiCommand::ListGroups {} => Some(self.list()),
            UiCommand::ForgetGroup { group_id } => Some(self.forget(ctx, group_id)),
            UiCommand::GetInvite { group_id } => Some(self.get_invite(group_id)),
            UiCommand::SetDisplayName { name } => Some(self.set_display_name(ctx, name)),
            UiCommand::SetVehicle { vehicle } => Some(self.set_vehicle(ctx, *vehicle)),
            UiCommand::AudioCheck(op) => self.audio_check(op),
            _ => self.session_command(ctx, cmd),
        }
    }

    fn create(&mut self, ctx: &mut Ctx, name: &str, vehicle: Vehicle) -> CommandResult {
        let platform = ctx.cfg.platform.caps_byte();
        let display = self.display_name.clone();
        let mut g = Group::create(name, vehicle, &display, platform, &mut ctx.rng);
        if leader::initial_roster(&mut g).is_err() {
            return CommandResult::error(ErrorCode::Internal);
        }
        let id = g.id;
        self.groups.insert(id, g);
        self.persist(ctx, id);
        group_id_result(&id)
    }

    fn parse_invite(&mut self, ctx: &mut Ctx, url: &str) -> CommandResult {
        let unix = super::radio::unix_s(ctx);
        let info = match Invite::from_url(url) {
            Ok(inv) => OkInviteInfo {
                ok: OkTrue,
                valid: true,
                group_name: Some(inv.name.clone()),
                group_id: Some(store::hex(&inv.group_id)),
                epoch: Some(inv.epoch),
                expired: inv.is_expired(unix),
                already_member: self.groups.contains_key(&inv.group_id),
                leader_sig_ok: inv.verify_leader_sig().is_ok(),
            },
            Err(_) => OkInviteInfo {
                ok: OkTrue,
                valid: false,
                group_name: None,
                group_id: None,
                epoch: None,
                expired: false,
                already_member: false,
                leader_sig_ok: false,
            },
        };
        CommandResult::InviteInfo(info)
    }

    fn join(&mut self, ctx: &mut Ctx, url: &str, display_name: &str) -> CommandResult {
        let Ok(inv) = Invite::from_url(url) else {
            return CommandResult::error(ErrorCode::InviteInvalid);
        };
        if inv.is_expired(super::radio::unix_s(ctx)) {
            return CommandResult::error(ErrorCode::InviteExpired);
        }
        if self.groups.contains_key(&inv.group_id) {
            return CommandResult::error(ErrorCode::AlreadyMember);
        }
        let vehicle = self.vehicle.unwrap_or(Vehicle::Moto);
        let mut g = Group::join(&inv, display_name, vehicle, &mut ctx.rng);
        g.remember(
            *g.identity.public(),
            ctx.cfg.platform.caps_byte(),
            display_name,
        );
        if !display_name.is_empty() {
            self.display_name = display_name.to_owned();
        }
        let id = g.id;
        // A closed tombstone of the same group is replaced by the new membership.
        self.closed.remove(&id);
        self.groups.insert(id, g);
        self.persist(ctx, id);
        group_id_result(&id)
    }

    fn list(&self) -> CommandResult {
        let mut groups: Vec<GroupSummary> = self
            .groups
            .values()
            .map(|g| GroupSummary {
                group_id: store::hex(&g.id),
                name: g.name.clone(),
                role: g.role(),
                vehicle: g.vehicle,
                members: g.member_count(),
                epoch: g.epoch(),
                closed: false,
            })
            .collect();
        groups.extend(self.closed.values().map(|c| GroupSummary {
            group_id: store::hex(&c.id),
            name: c.name.clone(),
            role: c.role,
            vehicle: c.vehicle,
            members: c.members,
            epoch: c.epoch,
            closed: true,
        }));
        CommandResult::Groups(OkGroups { ok: OkTrue, groups })
    }

    fn forget(&mut self, ctx: &mut Ctx, group_id: &str) -> CommandResult {
        let Some(id) = store::parse_hex(group_id) else {
            return CommandResult::error(ErrorCode::UnknownGroup);
        };
        if self.act.as_ref().is_some_and(|a| a.group_id == id) {
            return CommandResult::error(ErrorCode::WrongMode);
        }
        if self.groups.remove(&id).is_none() && self.closed.remove(&id).is_none() {
            return CommandResult::error(ErrorCode::UnknownGroup);
        }
        self.persist(ctx, id);
        CommandResult::done()
    }

    fn get_invite(&self, group_id: &str) -> CommandResult {
        let Some(id) = store::parse_hex(group_id) else {
            return CommandResult::error(ErrorCode::UnknownGroup);
        };
        if self.closed.contains_key(&id) {
            return CommandResult::error(ErrorCode::GroupClosed);
        }
        let Some(g) = self.groups.get(&id) else {
            return CommandResult::error(ErrorCode::UnknownGroup);
        };
        // A member without a verified leader-signed invite of the current epoch cannot
        // re-share one (§8.3): it never forges or alters an invite.
        let Some(inv) = g.invite() else {
            return CommandResult::error(ErrorCode::InviteInvalid);
        };
        match qr_data_url(&inv.to_url(), QR_SCALE) {
            Some(qr_png) => CommandResult::Invite(OkInvite {
                ok: OkTrue,
                qr_png,
                hide_after_ms: QR_HIDE_AFTER_MS,
            }),
            None => CommandResult::error(ErrorCode::Internal),
        }
    }

    fn set_display_name(&mut self, ctx: &mut Ctx, name: &str) -> CommandResult {
        self.display_name =
            mesh_wire::name::truncate_graphemes(name, mesh_wire::name::DISPLAY_NAME_MAX).to_owned();
        let display = self.display_name.clone();
        let platform = ctx.cfg.platform.caps_byte();
        let Some((g, act)) = self.active_mut() else {
            return CommandResult::done();
        };
        g.display_name = display.clone();
        g.remember(*g.identity.public(), platform, &display);
        act.dirty_store = true;
        if g.is_leader() {
            let me = g.book.get(&g.my_id_hash()).cloned();
            if let Some(j) = me
                && let Ok(Some(issued)) = leader::admit(g, &j)
            {
                self.publish(ctx, issued);
            }
        } else if g.in_roster() {
            let out = Out {
                ptype: PacketType::JoinReq,
                payload: join_body(g, platform),
            };
            self.originate(ctx, &out);
        }
        ctx.mark_dirty();
        CommandResult::done()
    }

    fn set_vehicle(&mut self, ctx: &mut Ctx, vehicle: Vehicle) -> CommandResult {
        self.vehicle = Some(vehicle);
        if let Some((g, act)) = self.active_mut() {
            g.vehicle = vehicle;
            act.session.vehicle = vehicle;
            act.dirty_store = true;
            ctx.mark_dirty();
        }
        CommandResult::done()
    }

    /// `audioCheck`: the lifecycle (refused while live) is decided here; the audio check
    /// itself (session, tone, record, echo, meter) is the media module's.
    fn audio_check(&mut self, op: &AudioCheckOp) -> Option<CommandResult> {
        let live = self.act.as_ref().is_some_and(|a| a.session.live);
        match op {
            AudioCheckOp::Start { .. } => self.check.start(live).answer(),
            AudioCheckOp::Stop {} => self.check.stop().answer(),
        }
    }
}
