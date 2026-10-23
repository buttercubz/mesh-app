//! S5 floor-contention (DESIGN §13.2, §6): six iPhones in a small cluster ride in VOX.
//! Four of them, the leader among them, start talking within 120 ms of each other; a
//! sixth rider is shielded during the starts and links up mid-burst. Asserted:
//!
//! - ≤ 2 streams per BLE link at all times (global stream-cap assertion);
//! - every loser is busy (BLOCKED) within 300 ms of its talk attempt;
//! - the leader is always heard by every listener;
//! - no equal-priority eviction after `T_STICKY`: the admitted member keeps the floor
//!   for its whole span;
//! - every node, the late one included, shows the admitted set within 300 ms of hearing
//!   a TALK KEEP or VOICE of it (plus one 100 ms snapshot period, the UI throttle).

use std::collections::BTreeSet;

use mesh_api::TalkMode;
use mesh_api::TalkState;

use super::Outcome;
use super::common::{form_ride, go_live_all, group_config};
use crate::config::{NodeSpec, WorldConfig};
use crate::presets::cluster;
use crate::rng::SimRng;
use crate::world::World;

const N: usize = 6;
/// Lobby time between the ride forming and the start.
const LOBBY_S: u64 = 40;
const LATE: usize = 5;
const TALKERS: [usize; 4] = [0, 1, 2, 3];
const SPAN_US: u64 = 14_000_000;
/// Snapshot emission period (≤ 10 Hz, §3.4): the UI view lags the floor table by up to
/// one period.
const SNAPSHOT_US: u64 = 100_000;
const AGREE_US: u64 = 300_000;
const BUSY_US: u64 = 300_000;

/// The snapshot talker set of node `i` at time `t`.
fn talkers_at(w: &World, i: usize, t: u64) -> Vec<u16> {
    w.nodes[i]
        .talkers_hist
        .iter()
        .take_while(|(at, _)| *at <= t)
        .last()
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// First snapshot change of node `i` to Talking or Blocked at or after `from`, and the
/// first Blocked at or after `from`.
fn attempt_and_busy(w: &World, i: usize, from: u64, until: u64) -> (Option<u64>, Option<u64>) {
    let h = &w.nodes[i].talk_hist;
    let attempt = h
        .iter()
        .find(|(t, s)| {
            *t >= from && *t < until && matches!(s, TalkState::Talking | TalkState::Blocked)
        })
        .map(|x| x.0);
    let busy = h
        .iter()
        .find(|(t, s)| *t >= from && *t < until && *s == TalkState::Blocked)
        .map(|x| x.0);
    let denied = w.nodes[i]
        .denied
        .iter()
        .find(|(t, _)| *t >= from && *t < until)
        .map(|x| x.0);
    let busy = match (busy, denied) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    (attempt, busy)
}

/// A finished S5 world and its script.
pub struct S5Run {
    pub w: World,
    pub g: Option<crate::ride::RideGroup>,
    pub t_c: u64,
    pub starts: Vec<(usize, u64)>,
    pub unshield: u64,
    pub end: u64,
    pub early: Vec<String>,
}

/// Runs the S5 script.
pub fn simulate(seed: u64) -> S5Run {
    let mut early = Vec::new();
    let nodes = (0..N).map(|_| NodeSpec::iphone()).collect();
    let cfg = group_config(WorldConfig::new(seed, nodes, cluster(seed, N, 8.0)));
    let mut w = World::new(cfg);
    let (g, formed) = form_ride(&mut w, 0);
    if formed.is_none() {
        early.push("the cluster never formed a ride".into());
        return S5Run {
            w,
            g: None,
            t_c: 0,
            starts: Vec::new(),
            unshield: 0,
            end: 0,
            early,
        };
    }
    // Lobby: riders gather before the ride starts (link pacers settle, §5.6).
    w.run_s(LOBBY_S);
    if !go_live_all(&mut w, &g, TalkMode::Vox, false) {
        early.push("go live refused".into());
    }
    // The late rider is shielded from everyone and its links drop.
    for b in 0..N {
        if b != LATE {
            w.block(LATE, b, true);
        }
    }
    w.run_s(8);
    if !w
        .up_links()
        .iter()
        .all(|l| w.links[*l].peer(LATE).is_none())
    {
        early.push("the shielded rider kept a link".into());
    }
    let mut rng = SimRng::new(seed, 0x55_0001);
    let t_c = w.now.0 + 500_000;
    let mut starts = Vec::new();
    for &i in &TALKERS {
        w.steady_voice(i);
        let s = t_c + rng.range_u64(0, 120) * 1_000;
        w.script_talk(i, s, s + SPAN_US, false);
        starts.push((i, s.div_ceil(10_000) * 10_000));
    }
    let unshield = t_c + 2_000_000;
    w.run_until(mesh_core::Instant(unshield));
    for b in 0..N {
        w.block(LATE, b, false);
    }
    let end = t_c + SPAN_US;
    w.run_until(mesh_core::Instant(end + 1_500_000));
    S5Run {
        w,
        g: Some(g),
        t_c,
        starts,
        unshield,
        end,
        early,
    }
}

pub fn run(seed: u64) -> Outcome {
    let mut o = Outcome::new("S5", seed);
    let S5Run {
        w,
        g,
        t_c,
        starts,
        unshield,
        end,
        early,
    } = simulate(seed);
    o.failures.extend(early);
    let Some(g) = g else {
        o.absorb(&w);
        return o;
    };
    let ids: Vec<u16> = (0..N).map(|i| w.node_id(i, &g.id).unwrap_or(0)).collect();
    let leader_id = ids[0];

    // The admitted set: origins whose frames reach the silent listener (node 4) in the
    // steady state of the contention.
    let (ss_lo, ss_hi) = (t_c + 1_500_000, end - 500_000);
    let admitted: BTreeSet<u16> = {
        let tap = w.nodes[4].tap.borrow();
        let mut per: std::collections::BTreeMap<u16, u32> = Default::default();
        for (k, f) in &tap.frames {
            if f.t >= ss_lo && f.t < ss_hi {
                *per.entry(crate::tap::unpack_frame(*k).0).or_default() += 1;
            }
        }
        // Steady streams carry ≥ 25 frames/s; a few stray frames are not a stream.
        let min = ((ss_hi - ss_lo) / 1_000_000 * 15) as u32;
        per.into_iter()
            .filter(|(_, c)| *c >= min)
            .map(|(o, _)| o)
            .collect()
    };
    o.metric(
        "admitted.count",
        admitted.len() as f64,
        admitted.len() == 2,
        "= 2 (cap)",
    );
    o.check(admitted.contains(&leader_id), || {
        format!("the leader {leader_id} is not admitted: {admitted:?}")
    });

    // Losers are busy within 300 ms of their attempt.
    let mut worst_busy = 0u64;
    for &(i, s) in &starts {
        if admitted.contains(&ids[i]) {
            continue;
        }
        let (attempt, busy) = attempt_and_busy(&w, i, s, end);
        match (attempt, busy) {
            (Some(a), Some(b)) => worst_busy = worst_busy.max(b.saturating_sub(a)),
            _ => {
                o.check(false, || {
                    format!("loser {i} never showed busy (attempt {attempt:?})")
                });
            }
        }
    }
    o.metric(
        "loser.busy.ms",
        worst_busy as f64 / 1_000.0,
        worst_busy <= BUSY_US,
        "≤ 300 ms",
    );

    // The leader is heard by every listener over its whole span (the late rider from
    // its first frame on).
    for l in 1..N {
        let tap = w.nodes[l].tap.borrow();
        let mut ts: Vec<u64> = tap
            .frames
            .iter()
            .filter(|(k, f)| {
                crate::tap::unpack_frame(**k).0 == leader_id && f.t >= t_c && f.t < end + 500_000
            })
            .map(|(_, f)| f.t)
            .collect();
        ts.sort_unstable();
        let gap = ts.windows(2).map(|p| p[1] - p[0]).max().unwrap_or(u64::MAX);
        let from = if l == LATE {
            ts.first().copied().unwrap_or(end)
        } else {
            t_c + 400_000
        };
        let covered = ts.first().is_some_and(|f| *f <= from)
            && ts.last().is_some_and(|x| *x >= end - 400_000);
        o.check(covered && gap <= 400_000, || {
            format!(
                "listener {l}: leader not heard throughout (frames {}, max gap {gap} µs)",
                ts.len()
            )
        });
    }

    // No equal-priority eviction after T_STICKY: the admitted member's frames reach
    // the silent listener continuously until its span ends.
    if let Some(&member) = admitted.iter().find(|x| **x != leader_id) {
        let tap = w.nodes[4].tap.borrow();
        let mut ts: Vec<u64> = tap
            .frames
            .iter()
            .filter(|(k, f)| crate::tap::unpack_frame(**k).0 == member && f.t >= t_c)
            .map(|(_, f)| f.t)
            .collect();
        ts.sort_unstable();
        let first = ts.first().copied().unwrap_or(end);
        let sticky_from = first + 300_000 + 200_000;
        let gap = ts
            .windows(2)
            .filter(|p| p[0] >= sticky_from && p[1] < end - 200_000)
            .map(|p| p[1] - p[0])
            .max()
            .unwrap_or(0);
        let last = ts.last().copied().unwrap_or(0);
        o.check(gap <= 400_000 && last >= end - 400_000, || {
            format!("admitted member {member} was cut after T_STICKY (gap {gap} µs, last {last})")
        });
    }

    // Agreement on the admitted set.
    let mut worst_agree = 0u64;
    for v in 0..N {
        // `talkers[]` lists me first while I talk, then the live bursts by rank.
        let expect: Vec<u16> = admitted.iter().copied().collect();
        let first = {
            let tap = w.nodes[v].tap.borrow();
            let lo = if v == LATE { unshield } else { t_c };
            let f = tap
                .frames
                .iter()
                .filter(|(k, f)| admitted.contains(&crate::tap::unpack_frame(**k).0) && f.t >= lo)
                .map(|(_, f)| f.t)
                .min();
            let k = tap
                .talk
                .iter()
                .filter(|t| {
                    t.t >= lo
                        && admitted.contains(&t.origin)
                        && t.state == Some(mesh_wire::mesh::TalkState::Keep)
                })
                .map(|t| t.t)
                .min();
            match (f, k) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        };
        let Some(first) = first else {
            o.check(false, || {
                format!("node {v} never heard the admitted bursts")
            });
            continue;
        };
        let deadline = first + AGREE_US + SNAPSHOT_US;
        let agreed_at = w.nodes[v]
            .talkers_hist
            .iter()
            .filter(|(t, ids)| *t >= first && *ids == expect)
            .map(|x| x.0)
            .next();
        let ok_at_deadline = talkers_at(&w, v, deadline) == expect;
        let lag = agreed_at.map_or(u64::MAX, |a| a.saturating_sub(first));
        if !ok_at_deadline {
            o.check(false, || {
                format!(
                    "node {v}: talkers {:?} ≠ admitted {expect:?} {} ms after first hearing them",
                    talkers_at(&w, v, deadline),
                    (deadline - first) / 1_000
                )
            });
        }
        if lag != u64::MAX {
            worst_agree = worst_agree.max(lag);
        }
    }
    o.metric(
        "agree.ms",
        worst_agree as f64 / 1_000.0,
        worst_agree <= AGREE_US + SNAPSHOT_US,
        "≤ 300 ms (+100 ms snapshot period)",
    );
    o.absorb(&w);
    o
}
