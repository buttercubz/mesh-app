//! PRESENCE origination and ingest (DESIGN §4.6.1, §5.8): 14 bytes every 2 s with the
//! anti-entropy version fields, ride/talk state, ladder and route, battery, link counts,
//! platform caps and the `heard` bitmap by roster index.

use mesh_api::{AudioRouteKind, LadderLevel, RidePhase, TalkMode};
use mesh_routing::ReachEvent;
use mesh_wire::mesh::{
    PacketType, Presence, PresenceAudio, PresenceCaps, PresenceLinks, PresenceState,
};

use super::{Out, Session};
use crate::LinkKind;
use crate::engine::Ctx;

/// PRESENCE `audio.route` code of a route kind (§4.6.1).
pub fn route_code(k: AudioRouteKind) -> u8 {
    match k {
        AudioRouteKind::None => PresenceAudio::ROUTE_NONE,
        AudioRouteKind::BuiltinSpeaker | AudioRouteKind::BuiltinReceiver => {
            PresenceAudio::ROUTE_BUILTIN
        }
        AudioRouteKind::HfpNarrowband => PresenceAudio::ROUTE_HFP_NB,
        AudioRouteKind::HfpWideband => PresenceAudio::ROUTE_HFP_WB,
        AudioRouteKind::LeAudio => PresenceAudio::ROUTE_LE_AUDIO,
        AudioRouteKind::CarBluetooth => PresenceAudio::ROUTE_CAR_HFP,
        AudioRouteKind::Wired => PresenceAudio::ROUTE_WIRED,
    }
}

/// Route kind of a PRESENCE code (`None` for "none").
pub fn route_kind(code: u8) -> Option<AudioRouteKind> {
    Some(match code {
        PresenceAudio::ROUTE_BUILTIN => AudioRouteKind::BuiltinSpeaker,
        PresenceAudio::ROUTE_HFP_NB => AudioRouteKind::HfpNarrowband,
        PresenceAudio::ROUTE_HFP_WB => AudioRouteKind::HfpWideband,
        PresenceAudio::ROUTE_LE_AUDIO => AudioRouteKind::LeAudio,
        PresenceAudio::ROUTE_CAR_HFP => AudioRouteKind::CarBluetooth,
        PresenceAudio::ROUTE_WIRED => AudioRouteKind::Wired,
        _ => return None,
    })
}

/// Ladder level as the 2-bit PRESENCE value.
pub fn ladder_bits(l: LadderLevel) -> u8 {
    match l {
        LadderLevel::L0 => 0,
        LadderLevel::L1 => 1,
        LadderLevel::L2 => 2,
        LadderLevel::L3 => 3,
    }
}

/// Ladder level of a 2-bit PRESENCE value.
pub fn ladder_of(b: u8) -> LadderLevel {
    match b & 3 {
        0 => LadderLevel::L0,
        1 => LadderLevel::L1,
        2 => LadderLevel::L2,
        _ => LadderLevel::L3,
    }
}

/// Inputs from the group that the session does not own.
#[derive(Debug, Clone, Copy)]
pub struct GroupView {
    pub leader: bool,
    pub roster_version: u32,
    pub epoch: u32,
}

/// Builds my PRESENCE now.
pub fn build(s: &Session, ctx: &Ctx, g: GroupView) -> Presence {
    let now = ctx.now;
    let ride = match s.ride {
        RidePhase::Lobby => PresenceState::RIDE_LOBBY,
        RidePhase::Riding => PresenceState::RIDE_RIDING,
    };
    let talk = if s.talk_muted() || s.effective_mode() == TalkMode::Muted {
        PresenceState::TALK_MUTED
    } else {
        PresenceState::TALK_IDLE
    };
    let capture_ok = s.live && ctx.device.capture_ok();
    let state = PresenceState::new(ride, talk, g.leader, s.on_call, capture_ok, s.live);
    let route = ctx.device.route_kind();
    let headset = ctx.device.route.headset_present;
    let tx_muted_no_headset = s.live
        && s.vehicle == mesh_api::Vehicle::Moto
        && !headset
        && matches!(
            route,
            AudioRouteKind::BuiltinSpeaker | AudioRouteKind::BuiltinReceiver
        );
    let audio = PresenceAudio::new(
        ladder_bits(s.ladder),
        route_code(route),
        tx_muted_no_headset,
        ctx.device.render_ok(),
        s.share,
    );
    let battery = ctx
        .device
        .sample
        .as_ref()
        .and_then(|x| x.battery)
        .map_or(Presence::BATTERY_UNKNOWN, |b| {
            (b.clamp(0.0, 1.0) * 100.0).round() as u8
        });
    let ble = s.link_kinds.values().filter(|k| k.is_ble()).count().min(15) as u8;
    let aware = s
        .link_kinds
        .values()
        .filter(|k| **k == LinkKind::WifiAware)
        .count()
        .min(15) as u8;
    let caps = PresenceCaps::new(
        ctx.cfg.platform.caps_byte(),
        false,
        s.radio_off_recently(now),
    );
    Presence {
        roster_ver16: 0,
        epoch16: 0,
        state,
        audio,
        battery,
        links: PresenceLinks::new(ble, aware),
        caps,
        interval: (super::PRESENCE_INTERVAL_US / 100_000) as u8,
        heard: s.reach.heard_bitmap(now.routing()),
    }
    .with_versions(g.roster_version, g.epoch)
}

/// PRESENCE due now: returns the packet to originate.
pub fn tick(s: &mut Session, ctx: &Ctx, g: GroupView) -> Option<Out> {
    if ctx.now < s.next_presence {
        return None;
    }
    s.next_presence = ctx.now.plus_us(super::PRESENCE_INTERVAL_US);
    Some(Out {
        ptype: PacketType::Presence,
        payload: build(s, ctx, g).encode(),
    })
}

/// An authenticated, deduplicated PRESENCE from `origin`, received over a link whose
/// one-way delay estimate is `link_owd_us`. Returns the reachability events (incl.
/// `PeerBehind` for direct neighbors that are behind me).
#[allow(clippy::too_many_arguments)]
pub fn ingest(
    s: &mut Session,
    ctx: &Ctx,
    origin: u16,
    sess: u8,
    hops: u8,
    age_us: u64,
    link_owd_us: u64,
    p: &Presence,
) -> Vec<ReachEvent> {
    s.gone.remove(&origin);
    // Heard as of when the origin sent it: the copy's age on arrival (its path age
    // plus the incoming link's one-way estimate, §4.5 `age_now`) is taken off the
    // receive time, so a member is lost `MEMBER_LOST` after its last PRESENCE left it,
    // however long that copy took to cross the mesh (§5.8: a partition is detected
    // ≤ 6.5 s after the physical split, also across congested or fading relays).
    let sent = crate::Instant(ctx.now.0.saturating_sub(age_us + link_owd_us));
    s.reach
        .on_presence(sent.routing(), origin, sess, hops, age_us, p, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EngineConfig, Platform};
    use crate::engine::Engine;

    #[test]
    fn a_presence_is_heard_as_of_when_its_origin_sent_it() {
        let now = crate::Instant(50_000_000);
        let e = Engine::new(EngineConfig::new(Platform::Ios, [7; 32]), now);
        let mut s = Session::new(1, mesh_api::Vehicle::Moto, crate::Instant::ZERO, 4);
        // 300 ms of path age plus a 40 ms incoming link: sent 340 ms ago.
        ingest(
            &mut s,
            e.ctx(),
            2,
            9,
            2,
            300_000,
            40_000,
            &Presence::default(),
        );
        let m = s.reach.member(2).expect("member tracked");
        assert_eq!(m.last_heard, crate::Instant(49_660_000).routing());
        // Lost exactly MEMBER_LOST after that, not after the copy arrived.
        let lost = crate::Instant(49_660_000 + mesh_routing::consts::MEMBER_LOST_US);
        assert!(s.reach.reachable(crate::Instant(lost.0 - 1).routing(), 2));
        assert!(!s.reach.reachable(lost.routing(), 2));
    }
}
