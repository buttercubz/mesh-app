//! The 1 Hz diagnostics tick: the core's [`CoreSample`] (cumulative counters; the
//! runtime turns them into per-second deltas) and the events derived from changes of
//! the state the UI shows (heard/lost, reachability, partitions, mic state, route,
//! leader). Runs from `Engine::handle_timeout` while diagnostics are on.

use std::collections::{BTreeMap, BTreeSet};

use mesh_api::{MemberRole, MemberStatus, Snapshot, TalkState};
use mesh_lab::ridelog::{CoreAudio, CoreLink, CoreSample, DiagEvent, MeshCounters, QueueDepth};

use super::SAMPLE_US;
use crate::Instant;
use crate::engine::{Ctx, Engine, EngineMode};
use crate::links::{LinkState, Prio};

/// What the previous tick saw (change detection).
#[derive(Debug, Clone, Default)]
struct Prev {
    /// `(node, role)` written in the last `session` event.
    session: Option<(u16, &'static str)>,
    /// Member → lost.
    lost: BTreeMap<u16, bool>,
    reach: BTreeSet<u16>,
    islands: u8,
    mic: Option<(&'static str, Option<&'static str>)>,
    route: Option<&'static str>,
    capture_ok: Option<bool>,
    leader: Option<u16>,
}

/// Diagnostics state of the engine (the 1 Hz grid and change detection).
#[derive(Debug, Clone, Default)]
pub struct DiagTracker {
    next: Option<Instant>,
    prev: Prev,
}

impl DiagTracker {
    /// The next tick (`None` until the first one ran).
    pub fn next(&self) -> Option<Instant> {
        self.next
    }
}

/// Runs the tick when due.
pub(crate) fn tick(e: &mut Engine) {
    if !e.ctx.fx.diag_on() || e.ctx.mode == EngineMode::Idle {
        e.ctx.diag = DiagTracker::default();
        return;
    }
    let now = e.ctx.now;
    let next = match e.ctx.diag.next {
        Some(t) if now < t => return,
        // Keep the 1 s grid when a tick runs late.
        Some(t) => t.plus_us((now.since(t) / SAMPLE_US + 1) * SAMPLE_US),
        None => now.plus_us(SAMPLE_US),
    };
    e.ctx.diag.next = Some(next);
    let snap = crate::ui::build_snapshot(&e.ctx, &e.lab, &e.modules);
    diff_snapshot(&mut e.ctx, &snap);
    let mut s = CoreSample {
        links: links(&e.ctx),
        mesh: snap.session.as_ref().map(|_| MeshCounters {
            nbrs: u16::from(snap.mesh.neighbors),
            reach: u16::from(snap.mesh.reachable),
            islands: u16::from(snap.mesh.islands),
            ..MeshCounters::default()
        }),
        floor: snap.session.as_ref().map(|x| {
            let me = match x.talk {
                TalkState::Talking => "talking",
                TalkState::Blocked => "refused",
                TalkState::Idle | TalkState::Muted => "idle",
            };
            (u8::try_from(snap.talkers.len()).unwrap_or(u8::MAX), me)
        }),
        audio: snap.session.as_ref().map(|x| CoreAudio {
            route: snap.audio.route.as_str(),
            ladder: x.ladder.as_str(),
            vad: false,
            ptt: false,
            muted: x.talk == TalkState::Muted || snap.audio.tx_muted_no_headset,
            tx_frames: 0,
            cap_dbfs: None,
            ren_dbfs: None,
        }),
        jb: Vec::new(),
        jb_total: None,
        dist_m: None,
        dropped_diag: e.ctx.fx.dropped_diag(),
    };
    let ctx = &mut e.ctx;
    e.modules.each(|m| m.fill_diag(ctx, &mut s));
    e.ctx.fx.diag_sample(s);
}

/// Every link as the core sees it now.
fn links(ctx: &Ctx) -> Vec<CoreLink> {
    ctx.links
        .iter()
        .map(|l| {
            let srtt = l.srtt_us();
            let rttvar = l.rtt.rttvar_us();
            let c = &l.counters;
            let loss = (c.pings_sent >= 4).then(|| {
                let lost = c.pings_sent.saturating_sub(c.pongs_received);
                (lost as f32 / c.pings_sent as f32).clamp(0.0, 1.0)
            });
            CoreLink {
                id: l.id.0,
                peer: l.peer_node.map(|n| n.0),
                kind: l.info.kind.lab(),
                role: Some(super::role(l.info.role)),
                state: match l.state {
                    LinkState::Handshake | LinkState::IdProof => "handshake",
                    LinkState::Up => "up",
                    LinkState::Closing => "closing",
                },
                rssi: l.rssi,
                mtu: l.att_mtu,
                rtt_p50_ms: srtt.map(|r| r as f32 / 1_000.0),
                // srtt + 2·rttvar approximates the 95th percentile of a normal spread.
                rtt_p95_ms: srtt.map(|r| (r + 2 * rttvar) as f32 / 1_000.0),
                loss,
                grant_kbps: l.pace.rate_bps.map(|r| r as f32 / 1_000.0),
                q: QueueDepth {
                    voice: sat16(l.queues_len(Prio::P1)),
                    ctrl: sat16(l.queues_len(Prio::P0)),
                    bulk: sat16(l.queues_len(Prio::P2) + l.queues_len(Prio::P3)),
                    units: sat16(l.units.len()),
                },
                would_block: l.diag.would_block,
                tx: l.diag.tx,
                rx: l.diag.rx,
            }
        })
        .collect()
}

fn sat16(v: usize) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

/// The ride banner's mic state: `paused` when live without a working capture.
fn mic_state(s: &Snapshot) -> Option<(&'static str, Option<&'static str>)> {
    let sess = s.session.as_ref()?;
    if !sess.live {
        return Some(("off", None));
    }
    if !s.audio.capture_ok {
        return Some(("paused", Some("captureLost")));
    }
    if s.audio.tx_muted_no_headset {
        return Some(("muted", Some("noHeadset")));
    }
    let state = match sess.talk {
        TalkState::Idle => "idle",
        TalkState::Talking => "talking",
        TalkState::Blocked => "blocked",
        TalkState::Muted => "muted",
    };
    Some((state, None))
}

/// Events from what changed in the UI-visible state since the previous tick.
fn diff_snapshot(ctx: &mut Ctx, s: &Snapshot) {
    let mut prev = std::mem::take(&mut ctx.diag.prev);
    if let Some(sess) = s.session.as_ref() {
        let role = match sess.role {
            MemberRole::Leader => "leader",
            MemberRole::Member => "member",
        };
        let me = sess.my_node_id;
        if prev.session != Some((me, role)) {
            prev.session = Some((me, role));
            ctx.fx.diag(DiagEvent::Session {
                ride_key: None,
                node: Some(me),
                role: Some(role),
            });
        }
        members(ctx, s, me, &mut prev);
        let mic = mic_state(s);
        if mic != prev.mic {
            if let Some((state, cause)) = mic {
                ctx.fx.diag(DiagEvent::MicState { state, cause });
            }
            prev.mic = mic;
        }
        if sess.live && prev.capture_ok == Some(true) && !s.audio.capture_ok {
            ctx.fx.diag(DiagEvent::CaptureLost {
                reason: "captureNotOk",
                detail: None,
            });
        }
        prev.capture_ok = Some(s.audio.capture_ok);
    }
    let route = s.audio.route.as_str();
    if prev.route != Some(route) {
        ctx.fx.diag(DiagEvent::Route {
            route,
            prev: prev.route,
            reason: None,
            hfp_wb: None,
        });
        prev.route = Some(route);
    }
    ctx.diag.prev = prev;
}

/// Heard/lost flips, reachability, partitions and leader changes from the roster.
fn members(ctx: &mut Ctx, s: &Snapshot, me: u16, prev: &mut Prev) {
    let mut reach = BTreeSet::new();
    let mut unreachable = Vec::new();
    let mut leader = None;
    for m in s.members.iter().filter(|m| m.node_id != me) {
        let lost = m.status == MemberStatus::Lost;
        if prev.lost.get(&m.node_id).is_some_and(|l| *l != lost)
            || (lost && !prev.lost.contains_key(&m.node_id))
        {
            ctx.fx.diag(DiagEvent::Heard {
                peer: m.node_id,
                last_heard_s: m.last_heard_ms / 1_000,
                lost,
            });
        }
        prev.lost.insert(m.node_id, lost);
        match m.status {
            MemberStatus::Direct | MemberStatus::Relayed | MemberStatus::OneWay => {
                reach.insert(m.node_id);
            }
            _ => unreachable.push(m.node_id),
        }
        if m.role == MemberRole::Leader {
            leader = Some(m.node_id);
        }
    }
    if s.session
        .as_ref()
        .is_some_and(|x| x.role == MemberRole::Leader)
    {
        leader = Some(me);
    }
    let gone: Vec<u16> = prev
        .lost
        .keys()
        .copied()
        .filter(|n| !s.members.iter().any(|m| m.node_id == *n))
        .collect();
    for n in gone {
        prev.lost.remove(&n);
        ctx.fx.diag(DiagEvent::MemberRemoved { peer: n, epoch: 0 });
    }
    if reach != prev.reach {
        ctx.fx.diag(DiagEvent::Reach {
            nodes: u16::try_from(reach.len()).unwrap_or(u16::MAX),
            added: reach.difference(&prev.reach).copied().collect(),
            removed: prev.reach.difference(&reach).copied().collect(),
        });
        prev.reach = reach.clone();
    }
    let islands = s.mesh.islands;
    if islands != prev.islands {
        ctx.fx.diag(DiagEvent::Partition {
            islands: u16::from(islands),
            mine: u16::try_from(reach.len() + 1).unwrap_or(u16::MAX),
            others: unreachable,
        });
        prev.islands = islands;
    }
    if leader.is_some() && leader != prev.leader {
        if prev.leader.is_some() {
            ctx.fx.diag(DiagEvent::LeaderChange {
                peer: leader.unwrap_or(0),
                epoch: 0,
            });
        }
        prev.leader = leader;
    }
}
