//! S3 cluster15 (DESIGN §13.2, §5.2-§5.6): fifteen iPhones within 30 m, two of them
//! talking at once (PTT, both admitted at cap 2) for a minute. Asserted: voice
//! transmissions per originated frame ≤ 2.25 × 14 (degree bound; voice is never pruned);
//! the pruned control flood (PRESENCE) costs less than an unpruned flood of the same
//! graph (metric); every link direction carries ≤ 60 % of its modeled capacity; every
//! node's airtime stays ≤ `NODE_BLE_BUDGET`; cap 2 on every voice link at steady state.

use std::collections::BTreeSet;

use mesh_api::TalkMode;

use super::Outcome;
use super::common::{form_ride, go_live_all, group_config};
use crate::config::{NodeSpec, WorldConfig};
use crate::presets::{CLUSTER_DIAMETER_M, cluster};
use crate::tap::unpack_frame;
use crate::world::World;

const N: usize = 15;
const TALKERS: [usize; 2] = [3, 10];
const TALK_US: u64 = 60_000_000;
/// Steady state starts this long after the talkers start.
const SETTLE_US: u64 = 5_000_000;

pub fn run(seed: u64) -> Outcome {
    let mut o = Outcome::new("S3", seed);
    let nodes = (0..N).map(|_| NodeSpec::iphone()).collect();
    let cfg = group_config(WorldConfig::new(
        seed,
        nodes,
        cluster(seed, N, CLUSTER_DIAMETER_M / 2.0),
    ));
    let mut w = World::new(cfg);
    let (g, formed) = form_ride(&mut w, 0);
    if !o.check(formed.is_some(), || {
        "the cluster never formed a ride".into()
    }) {
        o.absorb(&w);
        return o;
    }
    o.report("form.s", formed.map_or(0.0, |t| t.0 as f64 / 1e6));
    w.run_s(20);
    o.check(go_live_all(&mut w, &g, TalkMode::Ptt, false), || {
        "go live refused".into()
    });
    // Control-flood cost over a quiet lobby-to-ride period (no voice yet).
    let ctrl0 = presence_totals(&w);
    w.run_s(20);
    let ctrl1 = presence_totals(&w);
    let degree_sum = 2 * w.up_links().len() as u64;
    let unpruned = degree_sum.saturating_sub(N as u64 - 1) as f64;
    let originated = (ctrl1.1 - ctrl0.1).max(1) as f64;
    let pruned = (ctrl1.0 - ctrl0.0) as f64 / originated;
    o.report("ctrl.copies.per.packet.unpruned", unpruned);
    o.metric(
        "ctrl.copies.per.packet",
        pruned,
        pruned < unpruned,
        "< unpruned flood",
    );

    let t0 = w.now.0 + 300_000;
    for &i in &TALKERS {
        w.steady_voice(i);
        w.script_talk(i, t0, t0 + TALK_US, true);
    }
    w.run_until(mesh_core::Instant(t0 + SETTLE_US));
    let snap_links = link_snapshot(&w);
    let air0: Vec<u64> = w.nodes.iter().map(|n| n.air_used_us).collect();
    let t1 = w.now.0;
    w.run_until(mesh_core::Instant(t0 + TALK_US));
    let t2 = w.now.0;
    let dt = (t2 - t1) as f64 / 1e6;
    // Voice transmissions per originated frame.
    let ids: BTreeSet<u16> = TALKERS
        .iter()
        .filter_map(|i| w.node_id(*i, &g.id))
        .collect();
    let mut copies = 0u64;
    let mut frames: BTreeSet<u64> = BTreeSet::new();
    for n in &w.nodes {
        for (k, f) in &n.tap.borrow().frames {
            if ids.contains(&unpack_frame(*k).0) && f.t >= t0 {
                copies += u64::from(f.copies);
                frames.insert(*k);
            }
        }
    }
    let per_frame = copies as f64 / frames.len().max(1) as f64;
    o.check(!frames.is_empty(), || {
        "no voice frame was originated".into()
    });
    o.metric(
        "voice.tx.per.frame",
        per_frame,
        per_frame <= 2.25 * (N as f64 - 1.0),
        "≤ 2.25 × 14",
    );
    // Per-link load and node airtime over the steady state.
    let cap_bps = w.cfg.radio.ble_link_bps as f64;
    let mut worst_load = 0.0f64;
    for (li, b0) in snap_links {
        let l = &w.links[li];
        for (dir, base) in l.dirs.iter().zip(b0.iter()) {
            let bytes = dir.bytes.saturating_sub(*base);
            let load = bytes as f64 * 8.0 / dt / cap_bps;
            worst_load = worst_load.max(load);
        }
    }
    o.metric(
        "link.load.max",
        worst_load,
        worst_load <= 0.6,
        "≤ 60 % of capacity",
    );
    let mut worst_air = 0.0f64;
    for (i, n) in w.nodes.iter().enumerate() {
        let air = (n.air_used_us - air0[i]) as f64 / (t2 - t1) as f64;
        worst_air = worst_air.max(air);
    }
    o.metric(
        "node.airtime.max",
        worst_air,
        worst_air <= 1.0,
        "≤ NODE_BLE_BUDGET",
    );
    // Cap 2 at steady state on every rider.
    for (i, n) in w.nodes.iter().enumerate() {
        let before = n
            .cap_hist
            .iter()
            .take_while(|(t, _)| *t < t1)
            .last()
            .map(|x| x.1);
        let during = n
            .cap_hist
            .iter()
            .filter(|(t, _)| *t >= t1 && *t <= t2)
            .map(|x| x.1);
        let low = during.chain(before).min().unwrap_or(2);
        o.check(low >= 2, || {
            format!("rider {i}: BLE cap {low} at steady state")
        });
    }
    o.absorb(&w);
    o
}

/// `(PRESENCE copies received, distinct PRESENCE packets)` over all riders.
fn presence_totals(w: &World) -> (u64, u64) {
    let mut copies = 0;
    let mut keys = BTreeSet::new();
    for n in &w.nodes {
        let t = n.tap.borrow();
        copies += t.presence_copies;
        keys.extend(t.presence_keys.iter().copied());
    }
    (copies, keys.len() as u64)
}

/// Bytes delivered so far per direction of every Up link.
fn link_snapshot(w: &World) -> Vec<(usize, [u64; 2])> {
    w.up_links()
        .into_iter()
        .map(|li| (li, [w.links[li].dirs[0].bytes, w.links[li].dirs[1].bytes]))
        .collect()
}
