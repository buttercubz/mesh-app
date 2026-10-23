//! S9 adversarial neighbor (DESIGN §13.2, §5.2, §8.1, §8.4). A ride of four iPhones
//! where the victim `V` is the only radio neighbor of a malicious member `M` (a real
//! group member running a modified app) and of a listener `X`; a fifth phone, a
//! stranger `S` from another group, is near `V`.
//!
//! - **Replays:** units `V` already received from `M` are replayed on air.
//! - **Corrupted frames:** units from `M` are bit-flipped in flight.
//! - **Malformed records** and **forged origin at hops 0:** `M` sends MESH records with
//!   garbage bodies, then TALK packets claiming the leader's origin at `hops = 0`.
//! - **Stranger msg1:** `S` connects to `V` anyway (and a recorded msg1 is replayed).
//!
//! Asserted: everything is dropped (no record accepted from a replay or a corrupted unit,
//! no malformed or forged packet reaches anyone past `V`), no panic, `M` is disconnected
//! once its penalty points reach the threshold and every relink inside the 60 s ban is
//! refused, `V` never puts its static key on air to the stranger, and the ride keeps
//! working through `V` (voice from the leader still reaches `X`).

use std::cell::RefCell;
use std::rc::Rc;

use mesh_api::TalkMode;
use mesh_core::links::Prio;
use mesh_core::{CoreModule, Ctx, Instant, LinkKind};
use mesh_wire::mesh::{
    EncoderProfile, MeshHeader, MeshPayload, PacketType, Talk, TalkMode as WireTalkMode, TalkPrio,
    TalkState, encode_packet,
};
use mesh_wire::record::Record;

use super::Outcome;
use super::common::group_config;
use crate::config::{NodeSpec, WorldConfig};
use crate::mobility::Mobility;
use crate::rng::SimRng;
use crate::world::World;

const LEADER: usize = 0;
const VICTIM: usize = 1;
const EVIL: usize = 2;
const LISTENER: usize = 3;
const STRANGER: usize = 4;
/// Ban after `PENALTY_LIMIT_PER_MIN` points (§5.2: 20 per minute, 60 s ban).
const BAN_US: u64 = 60_000_000;

/// Records the malicious member's app puts on its links at given instants.
type Plan = Rc<RefCell<Vec<(u64, Vec<u8>)>>>;

/// The modified app of the malicious member: raw MESH records written straight to every
/// Up link, bypassing the mesh glue (no origin rewrite, no admission).
struct Evil {
    plan: Plan,
}

impl CoreModule for Evil {
    fn name(&self) -> &'static str {
        "sim-evil"
    }

    fn on_timer(&mut self, ctx: &mut Ctx) {
        let now = ctx.now.0;
        let due: Vec<Vec<u8>> = {
            let mut p = self.plan.borrow_mut();
            let (due, keep): (Vec<_>, Vec<_>) = p.drain(..).partition(|(t, _)| *t <= now);
            *p = keep;
            due.into_iter().map(|(_, b)| b).collect()
        };
        for body in due {
            for l in ctx.links.up_ids() {
                ctx.links
                    .send_record(l, Prio::P0, &Record::Mesh(&body), None);
            }
        }
    }

    fn next_timer(&self) -> Option<Instant> {
        self.plan.borrow().iter().map(|(t, _)| Instant(*t)).min()
    }
}

/// A forged TALK START claiming `origin` at hops 0.
fn forged_talk(origin: u16, sess: u8, seq: u16, burst: u8) -> Vec<u8> {
    let h = MeshHeader::new(PacketType::Talk, origin, sess, seq);
    let t = Talk {
        burst,
        state: TalkState::Start,
        prio: TalkPrio::Leader,
        mode: WireTalkMode::Ptt,
        profile: EncoderProfile::Ble16,
        elapsed_10ms: 0,
    };
    encode_packet(&h, &MeshPayload::Talk(t)).expect("TALK packet")
}

/// A MESH record that authenticates and parses as a record but whose packet does not
/// decode: a valid header from the sender's own origin over a truncated payload.
fn garbage(rng: &mut SimRng, origin: u16, sess: u8, seq: u16) -> Vec<u8> {
    let ptypes = [PacketType::Voice, PacketType::Talk, PacketType::Presence];
    let p = ptypes[(rng.next_u32() % 3) as usize];
    let mut b = MeshHeader::new(p, origin, sess, seq)
        .encode()
        .expect("header")
        .to_vec();
    let mut tail = vec![0u8; 1 + (rng.next_u32() % 2) as usize];
    rng.fill(&mut tail);
    b.extend(tail);
    b
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

pub fn run(seed: u64) -> Outcome {
    run_world(seed).0
}

/// Runs S9 and returns the outcome and the final world (diagnostics).
pub fn run_world(seed: u64) -> (Outcome, World) {
    let mut o = Outcome::new("S9", seed);
    let nodes = (0..5).map(|_| NodeSpec::iphone()).collect();
    // V at the centre; M, X and S around it, the leader on the far side.
    let pos = vec![
        (-12.0, 0.0),
        (0.0, 0.0),
        (10.0, 4.0),
        (0.0, 12.0),
        (3.0, -9.0),
    ];
    let cfg0 = group_config(WorldConfig::new(seed, nodes, Mobility::Static(pos)));
    let mut w = World::new(cfg0);
    // Radio shielding: M and X hear only V; the stranger hears only V.
    for (a, b) in [
        (EVIL, LEADER),
        (EVIL, LISTENER),
        (LISTENER, LEADER),
        (STRANGER, LEADER),
        (STRANGER, EVIL),
        (STRANGER, LISTENER),
    ] {
        w.block(a, b, true);
    }
    // The ride of four; the stranger rides its own group.
    let members: Vec<usize> = vec![VICTIM, EVIL, LISTENER];
    let g = w.form_group(LEADER, &members);
    let sg = w.form_group(STRANGER, &[]);
    w.start_group_all(&g.id, &[LEADER, VICTIM, EVIL, LISTENER]);
    w.start_group(STRANGER, &sg.id);
    let ride = [LEADER, VICTIM, EVIL, LISTENER];
    let formed = w.run_until_pred(super::common::FORM_LIMIT_MS, 250, |w| {
        super::common::all_reachable(w, &ride)
    });
    if !o.check(formed.is_some(), || "the ride never formed".into()) {
        o.absorb(&w);
        return (o, w);
    }
    o.check(go_live_all_of(&mut w, &g, &ride), || {
        "go live refused".into()
    });
    let ids: Vec<u16> = (0..4).map(|i| w.node_id(i, &g.id).unwrap_or(0)).collect();
    let link_mv = w.links_between(EVIL, VICTIM);
    let Some(&li_mv) = link_mv.first() else {
        o.check(false, || "M and V never linked".into());
        o.absorb(&w);
        return (o, w);
    };
    let to_v = usize::from(w.links[li_mv].ends[0].node == EVIL);
    let from_m = 1 - to_v;

    // 1. Replays and corrupted units from M to V.
    w.replay_recent(li_mv, to_v, 8, 200_000);
    w.corrupt_next(li_mv, from_m, 3);
    w.run_s(3);
    o.metric(
        "replay.records.accepted",
        w.faults.replay_records as f64,
        w.faults.replay_records == 0 && w.faults.replayed > 0,
        "= 0 (replays dropped)",
    );
    let v_link = w.links[li_mv].ends[to_v].link;
    let bad = w.nodes[VICTIM]
        .engine
        .ctx()
        .links
        .get(v_link)
        .map_or(0, |l| l.counters.bad_frames);
    o.check(w.links[li_mv].up, || {
        "replays or corrupted units broke the M-V link".into()
    });
    o.report("victim.bad_frames", bad as f64);
    o.check(w.faults.corrupted == 3, || {
        format!("{} units corrupted", w.faults.corrupted)
    });

    // 2. Malformed records, then forged origin at hops 0.
    let plan: Plan = Rc::default();
    w.with_engine(EVIL, |e, _| {
        e.register_module(Box::new(Evil { plan: plan.clone() }))
    });
    let mut rng = SimRng::new(seed, 0x59_0001);
    let t2 = w.now.0 + 200_000;
    let leader_sess = w.nodes[LEADER].engine.ctx().sess;
    let forged_sess = leader_sess ^ 0x5A | 1;
    {
        let mut p = plan.borrow_mut();
        let m_sess = w.nodes[EVIL].engine.ctx().sess;
        for k in 0..8u16 {
            p.push((
                t2 + u64::from(k) * 50_000,
                garbage(&mut rng, ids[EVIL], m_sess, 0x7000 + k),
            ));
        }
        for k in 0..3u16 {
            p.push((
                t2 + 1_000_000 + u64::from(k) * 100_000,
                forged_talk(ids[LEADER], forged_sess, 100 + k, 7),
            ));
        }
    }
    w.run_until(Instant(t2 + 3_000_000));
    // The ban: V closed its link to M.
    let banned_at = w
        .links
        .iter()
        .filter(|l| l.joins(EVIL, VICTIM) && l.down_at.is_some_and(|d| d >= t2))
        .filter(|l| l.closing_by.is_some_and(|c| l.ends[c].node == VICTIM))
        .filter_map(|l| l.down_at)
        .min();
    o.check(banned_at.is_some(), || "V never disconnected M".into());
    // Forged and malformed packets stop at V.
    let forged_seen_at_v = w.nodes[VICTIM]
        .tap
        .borrow()
        .talk
        .iter()
        .filter(|t| t.origin == ids[LEADER] && t.sess == forged_sess)
        .count();
    o.check(forged_seen_at_v > 0, || {
        "the forged packets never reached V".into()
    });
    // Ride on: the leader talks, the listener behind V hears it, while M keeps trying.
    let t3 = w.now.0 + 500_000;
    w.script_talk(LEADER, t3, t3 + 3_000_000, true);
    let ban_end = banned_at.unwrap_or(t2) + BAN_US;
    w.run_until(Instant(ban_end - 2_000_000));
    for &n in &[LEADER, LISTENER] {
        let tap = w.nodes[n].tap.borrow();
        let forged = tap
            .talk
            .iter()
            .filter(|t| t.origin == ids[LEADER] && t.sess == forged_sess)
            .count();
        o.check(forged == 0 && tap.undecodable == 0, || {
            format!(
                "node {n} received {forged} forged and {} malformed packets past V",
                tap.undecodable
            )
        });
    }
    let heard = w.nodes[LISTENER]
        .tap
        .borrow()
        .frames
        .iter()
        .filter(|(k, f)| crate::tap::unpack_frame(**k).0 == ids[LEADER] && f.t >= t3)
        .count();
    o.metric(
        "listener.leader.frames",
        heard as f64,
        heard >= 50,
        "≥ 50 (ride continues)",
    );
    // Every M-V link opened inside the ban was refused promptly.
    if let Some(b) = banned_at {
        let mut kept = 0u64;
        for l in w
            .links
            .iter()
            .filter(|l| l.joins(EVIL, VICTIM) && l.created_at > b)
        {
            let life = l.down_at.unwrap_or(w.now.0) - l.created_at;
            if l.created_at < b + BAN_US - 3_000_000 && life > 3_000_000 {
                kept += 1;
            }
        }
        o.metric(
            "ban.links.kept",
            kept as f64,
            kept == 0,
            "= 0 during the 60 s ban",
        );
    }

    // 3. The stranger: msg1 to V on a forced link, and a replayed msg1.
    let x_pk = w
        .group_of(VICTIM, &g.id)
        .map(|gr| gr.identity.x_pk())
        .unwrap_or([0; 32]);
    let li_s = w.force_link(STRANGER, VICTIM, LinkKind::BleL2cap);
    w.capture_link(li_s);
    w.run_s(8);
    let li_s2 = w.force_link(STRANGER, VICTIM, LinkKind::BleL2cap);
    w.capture_link(li_s2);
    let s_msg1: Vec<Vec<u8>> = w
        .captured(li_s)
        .into_iter()
        .filter(|(d, _, _)| *d == 0)
        .map(|(_, _, b)| b)
        .take(1)
        .collect();
    for b in s_msg1 {
        let t = w.now.0 + 50_000;
        w.schedule_replay(li_s2, 1, b, t);
    }
    w.run_s(8);
    let mut v_units = 0usize;
    let mut leaked = false;
    for li in [li_s, li_s2] {
        for (d, _, bytes) in w.captured(li) {
            if d == 1 {
                v_units += 1;
                leaked |= contains(&bytes, &x_pk);
            }
        }
        o.check(!w.links[li].up, || "V kept a link with the stranger".into());
    }
    o.check(!leaked, || {
        "V's static key went on air to the stranger".into()
    });
    o.report("stranger.v_units", v_units as f64);
    o.absorb(&w);
    (o, w)
}

/// Go live for the ride members only.
fn go_live_all_of(w: &mut World, g: &crate::ride::RideGroup, nodes: &[usize]) -> bool {
    w.start_ride(g.leader);
    w.run_ms(300);
    let mut ok = true;
    for &i in nodes {
        w.mute_mic(i);
        ok &= w.go_live(i, TalkMode::Ptt);
    }
    w.run_ms(300);
    ok
}
