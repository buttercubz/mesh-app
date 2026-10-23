//! Reachability view and snapshot (DESIGN §3.7, §5.8): session info, the member list with
//! status (me / direct / relayed / oneWay / lost / unconfirmed), voice-reach class, who
//! hears me, ready checks, flags, distance; the mesh reachability counts; and the
//! member notices (`memberJoined`, `memberLeft`, `memberLost`, `memberBack`, `alone`,
//! `partition`) aggregated over 5 s.

use mesh_api::{
    LinkKind as UiLinkKind, Member, MemberFlag, MemberReady, MemberRole, MemberStatus, NoticeKey,
    Params, PeerPlatform, SessionInfo, Snapshot, TalkState, VoiceReach as UiReach,
};
use mesh_routing::{ReachEvent, VoiceReach};
use mesh_wire::mesh::RosterRole;

use super::Session;
use super::cue::{self, Cue};
use super::presence::route_kind;
use crate::engine::Ctx;
use crate::group::state::Group;
use crate::links::{LinkKind, LinkState};

fn ui_reach(v: VoiceReach) -> UiReach {
    match v {
        VoiceReach::Kpi => UiReach::Kpi,
        VoiceReach::BestEffort => UiReach::BestEffort,
        VoiceReach::No => UiReach::No,
    }
}

fn platform_of(b: u8) -> PeerPlatform {
    if b == 1 {
        PeerPlatform::Android
    } else {
        PeerPlatform::Ios
    }
}

fn names(g: &Group, nodes: &[mesh_routing::NodeId]) -> String {
    nodes
        .iter()
        .map(|n| g.name_of(*n).unwrap_or_else(|| format!("#{n}")))
        .collect::<Vec<_>>()
        .join(", ")
}

fn notice_n(ctx: &mut Ctx, key: NoticeKey, n: usize, names: String) {
    let mut p = Params::new();
    p.insert("n".into(), (n as u32).into());
    p.insert("names".into(), names.into());
    ctx.notice(key, p);
}

/// The roster changed: feed the reachability indices, aggregate `memberJoined`.
pub fn on_roster(s: &mut Session, ctx: &mut Ctx, g: &Group) {
    let idx: Vec<(mesh_routing::NodeId, u8)> = g.roster_indices().into_iter().collect();
    s.reach.set_roster(&idx);
    for (n, _) in g.roster_indices() {
        if n != s.me && s.known.insert(n) {
            s.joined.push(ctx.now, g.name_of(n).unwrap_or_default());
        }
    }
    s.reach
        .set_my_versions(g.control.roster_version(), g.epoch());
    ctx.mark_dirty();
}

/// Reachability timers and aggregated notices. Returns direct neighbors that are behind
/// (anti-entropy push, §8.9 rule 3) — reported through `PeerBehind` on ingest instead.
pub fn tick(s: &mut Session, ctx: &mut Ctx, g: &Group) {
    for ev in s.reach.tick(ctx.now.routing()) {
        match ev {
            ReachEvent::MemberLost(n) => {
                let who = names(g, &n);
                // One rider: "Ana left"; several at once: "3 riders out of range".
                let c = if n.len() == 1 {
                    Cue::MemberLeft(who.clone())
                } else {
                    Cue::OutOfRange(n.len())
                };
                cue::request(ctx, &c);
                notice_n(ctx, NoticeKey::MemberLost, n.len(), who);
            }
            ReachEvent::MemberBack(n) => {
                let who = names(g, &n);
                cue::request(ctx, &Cue::MemberJoined(who.clone()));
                notice_n(ctx, NoticeKey::MemberBack, n.len(), who);
            }
            ReachEvent::Alone => {
                cue::request(ctx, &Cue::Alone);
                ctx.notice(NoticeKey::Alone, Params::new());
            }
            ReachEvent::Partition(c) => {
                let mut p = Params::new();
                p.insert("islands".into(), (c as u32).into());
                ctx.notice(NoticeKey::Partition, p);
            }
            ReachEvent::PeerBehind(_) => {}
        }
        ctx.mark_dirty();
    }
    // The names gathered over 5 s go to the cue's `{{name}}` too ("Ana, Bob").
    if let Some(n) = s.joined.due(ctx.now) {
        let who = n.join(", ");
        cue::request(ctx, &Cue::MemberJoined(who.clone()));
        notice_n(ctx, NoticeKey::MemberJoined, n.len(), who);
    }
    if let Some(n) = s.left.due(ctx.now) {
        let who = n.join(", ");
        cue::request(ctx, &Cue::MemberLeft(who.clone()));
        notice_n(ctx, NoticeKey::MemberLeft, n.len(), who);
    }
}

fn direct_kinds(ctx: &Ctx, node: u16) -> Vec<UiLinkKind> {
    let mut v: Vec<UiLinkKind> = ctx
        .links
        .iter()
        .filter(|l| l.state == LinkState::Up && l.peer_node.map(|p| p.0) == Some(node))
        .map(|l| match l.info.kind {
            LinkKind::BleL2cap | LinkKind::BleGatt => UiLinkKind::Ble,
            LinkKind::WifiAware => UiLinkKind::Aware,
            LinkKind::Tunnel => UiLinkKind::Tunnel,
        })
        .collect();
    v.dedup();
    v
}

/// Fills the session part of the snapshot.
pub fn fill(s: &Session, ctx: &Ctx, g: &Group, snap: &mut Snapshot) {
    let now = ctx.now.routing();
    let talk = if s.talk_muted() {
        TalkState::Muted
    } else {
        TalkState::Idle
    };
    snap.session = Some(SessionInfo {
        group_id: crate::group::store::hex(&g.id),
        group_name: g.name.clone(),
        role: g.role(),
        my_node_id: s.me,
        live: s.live,
        mode: s.effective_mode(),
        talk,
        ride: s.ride,
        ladder: s.ladder,
        share_distance: s.share,
    });
    let hears_me = s.reach.who_hears_me(now);
    let one_way = s.reach.one_way(now);
    let my_fix = ctx
        .device
        .sample
        .as_ref()
        .and_then(|x| x.gps)
        .map(|f| mesh_routing::Fix {
            lat_deg: f.lat,
            lon_deg: f.lon,
            heading_deg: f.heading_deg,
            at: now,
        });
    let mut members = Vec::new();
    let mut my_flags = Vec::new();
    if s.on_call {
        my_flags.push(MemberFlag::OnCall);
    }
    if matches!(
        s.ladder,
        mesh_api::LadderLevel::L2 | mesh_api::LadderLevel::L3
    ) {
        my_flags.push(MemberFlag::MicLost);
    }
    if s.radio_off_recently(ctx.now) {
        my_flags.push(MemberFlag::RadioOff);
    }
    members.push(Member {
        node_id: s.me,
        name: g.display_name.clone(),
        role: g.role(),
        platform: platform_of(ctx.cfg.platform.caps_byte()),
        status: MemberStatus::Me,
        hops: 0,
        quality: 100,
        link_kinds: Vec::new(),
        hears_me: true,
        voice: UiReach::Kpi,
        last_heard_ms: 0,
        battery: ctx.device.sample.as_ref().and_then(|x| x.battery),
        ready: MemberReady {
            live: s.live,
            mic: s.live && ctx.device.capture_ok(),
            route: Some(ctx.device.route_kind()),
        },
        flags: my_flags,
        distance_m: None,
        bearing_deg: None,
    });
    let roster: Vec<(u16, MemberRole, String)> = g
        .roster()
        .map(|r| {
            r.entries
                .iter()
                .filter(|e| e.role != RosterRole::Denied && e.node_id != s.me)
                .map(|e| {
                    let role = if e.role == RosterRole::Leader {
                        MemberRole::Leader
                    } else {
                        MemberRole::Member
                    };
                    (e.node_id, role, e.name.clone())
                })
                .collect()
        })
        .unwrap_or_default();
    let unconfirmed: Vec<(u16, MemberRole, String)> = s
        .reach
        .members()
        .filter(|m| m.roster_index.is_none() && !roster.iter().any(|(n, _, _)| *n == m.node_id))
        .map(|m| {
            let n = m.node_id;
            (n, MemberRole::Member, g.name_of(n).unwrap_or_default())
        })
        .collect();
    for (node, role, name, listed) in roster
        .into_iter()
        .map(|(n, r, nm)| (n, r, nm, true))
        .chain(unconfirmed.into_iter().map(|(n, r, nm)| (n, r, nm, false)))
    {
        let id = node;
        let reach = s.reach.member(id);
        let reachable = s.reach.reachable(now, id) && !s.gone.contains(&node);
        let kinds = direct_kinds(ctx, node);
        let status = if !listed {
            MemberStatus::Unconfirmed
        } else if !kinds.is_empty() {
            MemberStatus::Direct
        } else if !reachable {
            MemberStatus::Lost
        } else if one_way.contains(&id) {
            MemberStatus::OneWay
        } else {
            MemberStatus::Relayed
        };
        let p = reach.map(|m| m.presence);
        let mut flags = Vec::new();
        if let Some(p) = p {
            if p.state.on_call() {
                flags.push(MemberFlag::OnCall);
            }
            if p.audio.ladder() >= 2 {
                flags.push(MemberFlag::MicLost);
            }
            if p.audio.tx_muted_no_headset() {
                flags.push(MemberFlag::NoHeadset);
            }
            if p.caps.radio_off_recently() {
                flags.push(MemberFlag::RadioOff);
            }
        }
        let rel = match (my_fix, s.positions.get(&node)) {
            (Some(me), Some(them)) => mesh_routing::position::relative(now, &me, them),
            _ => None,
        };
        let platform = p
            .map(|p| p.caps.platform())
            .or_else(|| {
                g.book
                    .values()
                    .find(|j| j.identity.node_id() == node)
                    .map(|j| j.platform)
            })
            .unwrap_or(0);
        members.push(Member {
            node_id: node,
            name,
            role,
            platform: platform_of(platform),
            status,
            hops: reach.map_or(0, |m| m.traversals.saturating_sub(1)),
            quality: match status {
                MemberStatus::Direct => 100,
                MemberStatus::Relayed | MemberStatus::OneWay | MemberStatus::Unconfirmed
                    if reachable =>
                {
                    60
                }
                _ => 0,
            },
            link_kinds: kinds,
            hears_me: hears_me.contains(&id),
            voice: ui_reach(s.reach.voice_reach(now, id)),
            last_heard_ms: reach.map_or(u32::MAX, |m| {
                (now.since(m.last_heard) / 1_000).min(u64::from(u32::MAX)) as u32
            }),
            battery: p
                .filter(|p| p.battery != mesh_wire::mesh::Presence::BATTERY_UNKNOWN)
                .map(|p| f64::from(p.battery.min(100)) / 100.0),
            ready: MemberReady {
                live: p.is_some_and(|p| p.state.live()),
                mic: p.is_some_and(|p| p.state.capture_ok()),
                route: p.and_then(|p| route_kind(p.audio.route())),
            },
            flags,
            distance_m: rel.map(|r| r.distance_m.round()),
            bearing_deg: rel.map(|r| r.bearing_deg.round()),
        });
    }
    let total = members
        .iter()
        .filter(|m| m.status != MemberStatus::Unconfirmed)
        .count();
    let reachable = members
        .iter()
        .filter(|m| {
            matches!(
                m.status,
                MemberStatus::Me
                    | MemberStatus::Direct
                    | MemberStatus::Relayed
                    | MemberStatus::OneWay
            )
        })
        .count();
    snap.mesh.total = total.min(255) as u8;
    snap.mesh.reachable = reachable.min(255) as u8;
    snap.mesh.islands = s.reach.islands(now).count().min(255) as u8;
    snap.members = members;
}
