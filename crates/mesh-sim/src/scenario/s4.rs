//! S4 convoy15-highway (DESIGN §13.2, §5.8, §9.5): fifteen iPhones on bike / dash
//! mounts form the ride parked, then ride a curved highway at 90-110 km/h with
//! log-normal gaps (median 40 m, 20-500 m) for 30 minutes, talking in VOX. The convoy
//! splits and merges by itself as gaps open and close.
//!
//! Asserted:
//! - a partition is detected (every rider of each side shows every rider of the other
//!   side as lost) ≤ 6.5 s after the physical split, i.e. after the last unit crossed
//!   the cut (sustained splits only);
//! - riders rejoin (connected again) ≤ 10 s after their sides are back in range
//!   (some cross pair at PDR ≥ the discovery threshold, continuously);
//! - ≥ 95 % of frames reach listeners that stayed within the voice TTL of the talker
//!   for the whole talk span (at ≤ 3 hops: 4-hop listeners get frames only while the
//!   age cap allows, §5.5; reported);
//! - p95 modeled mouth-to-ear excluding headset legs ≤ 250 ms at ≤ 2 hops (= 300 ms with
//!   the legs); 3-hop p95 reported, not asserted.

use std::collections::{BTreeMap, BTreeSet};

use mesh_api::{TalkMode, UiCommand};

use super::Outcome;
use super::common::{
    Scope, component_of, form_ride, go_live_all, group_config, hops_from, last_heard_at,
    sees_reachable, voice_stats,
};
use crate::config::{NodeSpec, WorldConfig};
use crate::mobility::{Convoy, ConvoyParams, Mobility};
use crate::radio::Mount;
use crate::rng::SimRng;
use crate::talk::{TalkSpan, turns};
use crate::world::World;

const N: usize = 15;
/// Ride length of the suite run.
pub const RIDE_US: u64 = 30 * 60 * 1_000_000;
const STEP_MS: u64 = 100;
/// A requested snapshot arrives within the UI rate limit (100 ms) plus a step.
const SNAPSHOT_SLACK_US: u64 = 500_000;
/// §13.2 bounds.
const DETECT_BOUND_US: u64 = 6_500_000;
const REJOIN_BOUND_US: u64 = 10_000_000;
const DELIVERY_MIN: f64 = 0.95;
const M2E_P95_MS: f64 = 250.0;
/// Headset-leg budget added for the with-legs report (§7.9).
const LEG_BUDGET_MS: f64 = 50.0;

pub fn run(seed: u64) -> Outcome {
    run_for(seed, RIDE_US)
}

/// One node pair's partition bookkeeping.
#[derive(Debug, Clone, Copy, Default)]
struct Pair {
    connected: bool,
    /// A sustained split being timed: the physical split instant.
    split_at: Option<u64>,
    /// Split detected (or abandoned because the sides reconnected first).
    detected: bool,
    /// Since when the two sides are continuously back in range.
    back_since: Option<u64>,
}

/// A formed, departed S4 convoy with its scripted talk.
pub struct Setup {
    pub w: World,
    pub g: crate::ride::RideGroup,
    pub spans: Vec<TalkSpan>,
    /// End of the scripted ride (µs).
    pub end: u64,
}

/// Forms the ride parked, goes live in VOX, departs and scripts `ride_us` of talk.
/// `Err` carries the failed outcome when the ride never formed.
pub fn setup(seed: u64, ride_us: u64) -> Result<(Setup, Outcome), Box<Outcome>> {
    let mut o = Outcome::new("S4", seed);
    let nodes = (0..N)
        .map(|_| NodeSpec::iphone().with_mount(Mount::Mount))
        .collect();
    let mut convoy = Convoy::new(N, ConvoyParams::default(), SimRng::new(seed, 0x54_0C04));
    convoy.park(8.0);
    let cfg = group_config(WorldConfig::new(
        seed,
        nodes,
        Mobility::Convoy(Box::new(convoy)),
    ));
    let mut w = World::new(cfg);
    let (g, formed) = form_ride(&mut w, 0);
    if !o.check(formed.is_some(), || "the parked convoy never formed".into()) {
        o.absorb(&w);
        return Err(Box::new(o));
    }
    o.check(go_live_all(&mut w, &g, TalkMode::Vox, true), || {
        "go live refused".into()
    });
    if let Mobility::Convoy(c) = &mut w.mobility {
        c.depart();
    }
    let start = w.now.0 + 1_000_000;
    let end = start + ride_us;
    let mut rng = SimRng::new(seed, 0x54_7A1C);
    let all: Vec<usize> = (0..N).collect();
    let spans = turns(&mut rng, &all, start, end, (2_000, 5_000), (4_000, 20_000));
    for sp in &spans {
        w.script_talk(sp.node, sp.from_us, sp.until_us, false);
    }
    Ok((Setup { w, g, spans, end }, o))
}

/// Runs S4 with a ride of `ride_us` (the tests use a shorter ride; the suite the full
/// 30 minutes).
pub fn run_for(seed: u64, ride_us: u64) -> Outcome {
    run_observed(seed, ride_us, &mut |_| {})
}

/// [`run_for`] with `observe` called on the world after every 100 ms step (traces and
/// diagnostics see exactly the asserted run, snapshot requests included).
pub fn run_observed(seed: u64, ride_us: u64, observe: &mut dyn FnMut(&mut World)) -> Outcome {
    let (
        Setup {
            mut w,
            g,
            spans,
            end,
        },
        mut o,
    ) = match setup(seed, ride_us) {
        Ok(x) => x,
        Err(o) => return *o,
    };
    let ids: Vec<u16> = (0..N).map(|i| w.node_id(i, &g.id).unwrap_or(0)).collect();
    let ttl = w.cfg.limits_for(0).routing.voice_ttl;
    let lost_us = mesh_core::mesh::REACHABLE_US;
    // Listeners that stayed within TTL of the talker for the whole span.
    let mut eligible: Vec<[bool; N]> = vec![[true; N]; spans.len()];
    let mut pairs: BTreeMap<(usize, usize), Pair> = BTreeMap::new();
    let comp0 = component_of(&w);
    let mut prev = comp0.clone();
    for a in 0..N {
        for b in a + 1..N {
            pairs.insert(
                (a, b),
                Pair {
                    connected: comp0[a] == comp0[b],
                    detected: true,
                    ..Pair::default()
                },
            );
        }
    }
    let (mut detect_max, mut detect_n) = (0u64, 0u64);
    let (mut rejoin_max, mut rejoin_n) = (0u64, 0u64);
    let (mut late_detect, mut late_rejoin) = (Vec::new(), Vec::new());
    let mut split_events = 0u64;
    while w.now.0 < end + 2_000_000 {
        // Riders with a split being timed render a fresh snapshot (as a UI on screen
        // would): detection is the engine's view, not the snapshot's change trigger.
        let watching: BTreeSet<usize> = pairs
            .iter()
            .filter(|(_, p)| !p.detected && p.split_at.is_some())
            .flat_map(|(k, _)| [k.0, k.1])
            .collect();
        for i in watching {
            w.command(i, UiCommand::RequestSnapshot {});
        }
        w.run_ms(STEP_MS);
        observe(&mut w);
        let now = w.now.0;
        let comp = component_of(&w);
        let prev_comp = std::mem::replace(&mut prev, comp.clone());
        let in_range = sides_in_range(&w, &comp);
        for (&(a, b), p) in pairs.iter_mut() {
            let conn = comp[a] == comp[b];
            if p.connected && !conn {
                // A new split: the physical split is the last unit across the cut.
                let t = last_crossing(&w, &prev_comp, &comp, a, b).min(now);
                p.split_at = Some(t);
                p.detected = false;
                p.back_since = None;
                split_events += 1;
            }
            if !p.connected && conn {
                if let Some(back) = p.back_since {
                    let d = now - back;
                    rejoin_max = rejoin_max.max(d);
                    rejoin_n += 1;
                    if d > REJOIN_BOUND_US + STEP_MS * 1_000 {
                        late_rejoin.push((a, b, back, d));
                    }
                }
                p.back_since = None;
                // Reconnected before the split was detected: not a sustained split.
                p.detected = true;
            }
            p.connected = conn;
            if conn {
                continue;
            }
            // Detection of a sustained split: both sides show each other lost. The
            // engine marks a member lost exactly `MEMBER_LOST` after it last heard it,
            // so the detection instant is computed from the snapshot's `lastHeardMs`
            // (exact whatever the snapshot cadence).
            if !p.detected
                && let Some(t) = p.split_at
            {
                let lost = !sees_reachable(&w, a, ids[b]) && !sees_reachable(&w, b, ids[a]);
                let heard = last_heard_at(&w, a, ids[b]).zip(last_heard_at(&w, b, ids[a]));
                if lost && let Some((ha, hb)) = heard {
                    let at = ha.max(hb) + lost_us;
                    let d = at.saturating_sub(t);
                    detect_max = detect_max.max(d);
                    detect_n += 1;
                    p.detected = true;
                    if d > DETECT_BOUND_US {
                        late_detect.push((a, b, t, d));
                    }
                } else if now - t > DETECT_BOUND_US + SNAPSHOT_SLACK_US {
                    // Still not lost in a snapshot long after the bound.
                    late_detect.push((a, b, t, now - t));
                    p.detected = true;
                }
            }
            // Back in range, continuously, until they connect.
            let r = in_range.get(&(comp[a].min(comp[b]), comp[a].max(comp[b])));
            if r.is_some_and(|x| *x) {
                let since = *p.back_since.get_or_insert(now);
                if now - since > REJOIN_BOUND_US + STEP_MS * 1_000 {
                    late_rejoin.push((a, b, since, now - since));
                    // Report once per episode: restart the clock.
                    p.back_since = Some(now);
                }
            } else {
                p.back_since = None;
            }
        }
        track_eligibility(&w, &spans, &mut eligible, ttl, now);
    }
    o.report("splits", split_events as f64);
    o.report("partition.pairs.detected", detect_n as f64);
    o.report("rejoins", rejoin_n as f64);
    o.report("partition.detect.late", late_detect.len() as f64);
    o.report("rejoin.late", late_rejoin.len() as f64);
    o.check(detect_n > 0, || "no sustained partition happened".into());
    o.check(rejoin_n > 0, || "no rejoin happened".into());
    o.metric(
        "partition.detect.max.s",
        detect_max as f64 / 1e6,
        late_detect.is_empty(),
        "≤ 6.5 s after the physical split",
    );
    for (a, b, t, d) in late_detect.iter().take(4) {
        o.failures.push(format!(
            "riders {a}/{b}: split at {:.1} s detected after {:.1} s",
            *t as f64 / 1e6,
            *d as f64 / 1e6
        ));
    }
    o.metric(
        "rejoin.max.s",
        rejoin_max as f64 / 1e6,
        late_rejoin.is_empty(),
        "≤ 10 s after back in range",
    );
    for (a, b, t, d) in late_rejoin.iter().take(4) {
        o.failures.push(format!(
            "riders {a}/{b}: back in range at {:.1} s, still apart after {:.1} s",
            *t as f64 / 1e6,
            *d as f64 / 1e6
        ));
    }
    voice_metrics(&mut o, &w, &g, &spans, &eligible);
    o.absorb(&w);
    o
}

/// Time of the last unit delivered on any link between the components of `a` and `b`.
/// The physical split of `a` and `b`: the last unit across the cut that separated
/// them. The cut is made of the links of their previous common component (`prev`)
/// whose ends are now apart (`comp`). A unit from one side reaches the other only
/// while both sides still have such a link, so the split is the earlier of the two
/// sides' last crossings: in a three-way split (`a`'s side, a middle rider, `b`'s side)
/// the links between `a`'s and `b`'s sides are only the long-dead direct ones, and
/// what separated them is the later death of the middle rider's links.
fn last_crossing(w: &World, prev: &[usize], comp: &[usize], a: usize, b: usize) -> u64 {
    let side = |me: usize| {
        w.links
            .iter()
            .filter(|l| {
                let (x, y) = (l.ends[0].node, l.ends[1].node);
                prev[x] == prev[a]
                    && prev[y] == prev[a]
                    && (comp[x] == comp[me]) != (comp[y] == comp[me])
            })
            .map(|l| l.dirs[0].last_delivery_us.max(l.dirs[1].last_delivery_us))
            .max()
    };
    match (side(a), side(b)) {
        (Some(x), Some(y)) => x.min(y),
        _ => w.now.0,
    }
}

/// Component pairs with at least one cross node pair in discovery range.
fn sides_in_range(w: &World, comp: &[usize]) -> BTreeMap<(usize, usize), bool> {
    let min = w.cfg.discovery.min_pdr;
    let mut m = BTreeMap::new();
    for x in 0..N {
        for y in x + 1..N {
            let (cx, cy) = (comp[x], comp[y]);
            if cx == cy {
                continue;
            }
            let k = (cx.min(cy), cx.max(cy));
            if m.get(&k) == Some(&true) {
                continue;
            }
            m.insert(k, w.ble_pdr(x, y) >= min);
        }
    }
    m
}

/// Clears the eligibility of listeners further than `ttl` hops from an active talker.
fn track_eligibility(w: &World, spans: &[TalkSpan], eligible: &mut [[bool; N]], ttl: u8, now: u64) {
    for (k, sp) in spans.iter().enumerate() {
        // The span plus the 1 s attribution tail of `voice_stats`.
        if now < sp.from_us || now > sp.until_us + 1_000_000 {
            continue;
        }
        let d = hops_from(w, sp.node);
        for (l, e) in eligible[k].iter_mut().enumerate() {
            if d[l] > ttl {
                *e = false;
            }
        }
    }
}

fn voice_metrics(
    o: &mut Outcome,
    w: &World,
    g: &crate::ride::RideGroup,
    spans: &[TalkSpan],
    eligible: &[[bool; N]],
) {
    let idx = |sp: &TalkSpan| {
        spans
            .iter()
            .position(|s| s.node == sp.node && s.from_us == sp.from_us)
    };
    let within = |sp: &TalkSpan, l: usize| idx(sp).is_some_and(|k| eligible[k][l]);
    let st = voice_stats(
        w,
        g,
        w.talk_spans(),
        &Scope {
            listener: &within,
            solo_only: true,
        },
    );
    let delivered = 1.0 - st.loss(3);
    o.metric(
        "delivered.le3hops",
        delivered,
        delivered >= DELIVERY_MIN,
        "≥ 95 % within TTL",
    );
    o.report("delivered.le4hops", 1.0 - st.loss(4));
    for h in 1..=4u8 {
        o.report(&format!("m2e.samples.{h}hop"), st.samples(h) as f64);
    }
    let p95 = st.m2e_range_ms(1, 2, 95.0);
    o.check(st.samples(1) + st.samples(2) >= 50, || {
        format!(
            "only {} latency samples at ≤ 2 hops",
            st.samples(1) + st.samples(2)
        )
    });
    o.metric(
        "m2e.p95.le2hops.ms",
        p95.unwrap_or(f64::INFINITY),
        p95.is_some_and(|p| p <= M2E_P95_MS),
        "≤ 250 ms excl. legs",
    );
    if let Some(p) = p95 {
        o.report("m2e.p95.le2hops.withLegs.ms", p + LEG_BUDGET_MS);
    }
    if let Some(p) = st.m2e_ms(3, 95.0) {
        o.report("m2e.p95.3hop.ms", p);
    }
}
