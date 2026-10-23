//! Ride control and membership messages (DESIGN §1.3 F3/F8, §4.6): signed RIDE_CTL with
//! `ride_id` (leader floods start/end; a member's local start is never flooded), signed
//! LEAVE, talk mode, "Share my distance" and POSITION origination/ingest.

use mesh_api::{CommandResult, ErrorCode, NoticeKey, Params, RideOp, RidePhase, TalkMode};
use mesh_crypto::{GroupIdentity, SignedLeave, SignedRideCtl};
use mesh_routing::Fix;
use mesh_wire::mesh::{LeaveReason, PacketType, Position, RideCtl, RideCtlOp, Roster};

use super::cue::{self, Cue};
use super::{Out, Session};
use crate::engine::Ctx;

/// What the group contributes to a ride command.
pub struct Signer<'a> {
    pub identity: &'a GroupIdentity,
    pub group_id: &'a [u8; 16],
    pub epoch: u32,
    pub leader: bool,
}

fn start_ride(s: &mut Session, ctx: &mut Ctx, ride_id: Option<u32>) {
    let was = s.ride;
    s.ride = RidePhase::Riding;
    s.ride_id = ride_id.or(s.ride_id);
    if was != RidePhase::Riding {
        cue::request(ctx, &Cue::RideStarted);
        let mut p = Params::new();
        p.insert("n".into(), (s.reach.members().count() as u32 + 1).into());
        ctx.notice(NoticeKey::RideStarted, p);
    }
    ctx.mark_dirty();
}

/// `rideControl{op}`. The leader signs and floods RIDE_CTL; a member's Start is local.
pub fn ride_control(
    s: &mut Session,
    ctx: &mut Ctx,
    op: RideOp,
    signer: &Signer<'_>,
) -> (CommandResult, Option<Out>) {
    match op {
        RideOp::Start => {
            if !signer.leader {
                start_ride(s, ctx, None);
                return (CommandResult::done(), None);
            }
            let ride_id = ctx.rand_u32();
            start_ride(s, ctx, Some(ride_id));
            let c = SignedRideCtl::sign(
                signer.identity,
                signer.group_id,
                signer.epoch,
                ride_id,
                RideCtlOp::Start,
            );
            (CommandResult::done(), Some(ride_out(&c.ctl)))
        }
        RideOp::End => {
            if !signer.leader {
                return (CommandResult::error(ErrorCode::NotLeader), None);
            }
            let Some(ride_id) = s.ride_id else {
                s.ride = RidePhase::Lobby;
                ctx.mark_dirty();
                return (CommandResult::done(), None);
            };
            let c = SignedRideCtl::sign(
                signer.identity,
                signer.group_id,
                signer.epoch,
                ride_id,
                RideCtlOp::End,
            );
            end_ride(s, ctx);
            (CommandResult::done(), Some(ride_out(&c.ctl)))
        }
    }
}

fn ride_out(c: &RideCtl) -> Out {
    Out {
        ptype: PacketType::RideCtl,
        payload: c.encode(),
    }
}

fn end_ride(s: &mut Session, ctx: &mut Ctx) {
    s.ride = RidePhase::Lobby;
    s.ride_id = None;
    ctx.notice(NoticeKey::RideEnded, Params::new());
    ctx.mark_dirty();
}

/// A flooded RIDE_CTL from `origin`: verified against the roster key of its origin;
/// `end` applies only to the current ride.
pub fn on_ride_ctl(
    s: &mut Session,
    ctx: &mut Ctx,
    origin: u16,
    c: RideCtl,
    roster: Option<&Roster>,
    epoch: u32,
) {
    let signed = SignedRideCtl { origin, ctl: c };
    let Some(r) = roster else { return };
    if signed.verify_with_roster(r, epoch).is_err() {
        return;
    }
    match c.op {
        RideCtlOp::Start => start_ride(s, ctx, Some(c.ride_id)),
        RideCtlOp::End => {
            if signed.ends_ride(s.ride_id) {
                // Members get a prompt and are never auto-stopped (§1.3 F8).
                end_ride(s, ctx);
            }
        }
    }
}

/// My signed LEAVE.
pub fn leave_out(
    identity: &GroupIdentity,
    group_id: &[u8; 16],
    sess: u8,
    reason: LeaveReason,
) -> Out {
    let l = SignedLeave::sign(identity, group_id, sess, reason);
    Out {
        ptype: PacketType::Leave,
        payload: l.leave.encode(),
    }
}

/// A flooded LEAVE: verified with the origin's roster key; the member is forgotten and
/// a `memberLeft` notice is aggregated.
pub fn on_leave(
    s: &mut Session,
    ctx: &mut Ctx,
    origin: u16,
    sess: u8,
    leave: mesh_wire::mesh::Leave,
    roster: Option<&Roster>,
    name: Option<String>,
) {
    let Some(r) = roster else { return };
    let signed = SignedLeave {
        origin,
        sess,
        leave,
    };
    if signed.verify_with_roster(r).is_err() {
        return;
    }
    if s.gone.insert(origin) {
        s.reach.remove(origin);
        s.positions.remove(&origin);
        s.left.push(ctx.now, name.unwrap_or_default());
        ctx.mark_dirty();
    }
}

/// `setTalkMode`.
pub fn set_talk_mode(s: &mut Session, ctx: &mut Ctx, mode: TalkMode) -> CommandResult {
    s.mode = Some(mode);
    ctx.mark_dirty();
    CommandResult::done()
}

/// `setShareDistance`.
pub fn set_share(s: &mut Session, ctx: &mut Ctx, on: bool) -> CommandResult {
    if s.share != on {
        s.share = on;
        s.next_position = ctx.now;
        if !on {
            s.positions.clear();
        }
        ctx.mark_dirty();
    }
    CommandResult::done()
}

/// POSITION due now (sharing on, valid fix): every 5 s moving, 20 s stopped.
pub fn position_tick(s: &mut Session, ctx: &Ctx) -> Option<Out> {
    if !s.share || ctx.now < s.next_position {
        return None;
    }
    // Without a fix, look again after the moving period (never spin on a past deadline).
    s.next_position = ctx.now.plus_us(super::POSITION_MOVING_US);
    let sample = ctx.device.sample.as_ref()?;
    let fix = sample.gps?;
    let speed = fix.speed_mps.unwrap_or(0.0).max(0.0);
    let moving = speed >= super::MOVING_MPS;
    if !moving {
        s.next_position = ctx.now.plus_us(super::POSITION_STOPPED_US);
    }
    let p = Position {
        lat_e7: (fix.lat * 1e7).round() as i32,
        lon_e7: (fix.lon * 1e7).round() as i32,
        speed_dms: (speed * 10.0).round().min(f64::from(u16::MAX)) as u16,
        heading_2deg: (fix.heading_deg.unwrap_or(0.0).rem_euclid(360.0) / 2.0) as u8,
        acc_m: fix.acc_m.unwrap_or(255.0).clamp(0.0, 255.0) as u8,
        fix_age_100ms: 0,
        rsv: 0,
    };
    Some(Out {
        ptype: PacketType::Position,
        payload: p.encode(),
    })
}

/// A flooded POSITION from `origin`.
pub fn on_position(s: &mut Session, ctx: &mut Ctx, origin: u16, p: &Position) {
    if !s.share {
        return;
    }
    s.positions
        .insert(origin, Fix::from_position(p, ctx.now.routing()));
    ctx.mark_dirty();
}
