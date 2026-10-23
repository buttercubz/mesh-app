//! S6 mixed-roles (DESIGN §13.2, §9.2, §9.4): four iPhones and three Android phones
//! within 20 m form the ride in the foreground, then the iPhones go to the background.
//! A shielding episode takes every link down; when the riders are back in range the
//! mesh must reform under the role rules: a backgrounded iPhone advertises only in the
//! overflow area (seen by iPhones, which cannot scan it in the background either), so
//! every new link is iPhone-central toward an Android peripheral or Android ↔ Android.
//!
//! Asserted: connected (one component, every rider sees every member reachable) within
//! 30 s of being back in range and at every sample of the following minute; every link
//! formed while the iPhones were backgrounded is iPhone → Android or Android ↔ Android;
//! no Android-initiated link to an iPhone (also a global assertion of every scenario).

use mesh_api::TalkMode;
use mesh_core::Platform;

use super::Outcome;
use super::common::{all_reachable, form_ride, go_live_all, group_config};
use crate::config::{AppState, NodeSpec, WorldConfig};
use crate::presets::cluster;
use crate::world::World;

const IPHONES: usize = 4;
const ANDROIDS: usize = 3;
const N: usize = IPHONES + ANDROIDS;
/// Shielding episode: long enough for every link to drop and every member to be lost.
const SHIELD_MS: u64 = 15_000;
const RECONNECT_BOUND_MS: u64 = 30_000;
const HOLD_MS: u64 = 60_000;

pub fn run(seed: u64) -> Outcome {
    run_world(seed).0
}

/// Runs S6 and returns the outcome and the final world (diagnostics).
pub fn run_world(seed: u64) -> (Outcome, World) {
    let mut o = Outcome::new("S6", seed);
    let nodes = (0..N)
        .map(|i| {
            if i < IPHONES {
                NodeSpec::iphone()
            } else {
                NodeSpec::android()
            }
        })
        .collect();
    let mut w = World::new(group_config(WorldConfig::new(
        seed,
        nodes,
        cluster(seed, N, 10.0),
    )));
    let (g, formed) = form_ride(&mut w, 0);
    if !o.check(formed.is_some(), || "the mixed fleet never formed".into()) {
        o.absorb(&w);
        return (o, w);
    }
    o.report("form.s", formed.map_or(0.0, |t| t.0 as f64 / 1e6));
    o.check(go_live_all(&mut w, &g, TalkMode::Ptt, false), || {
        "go live refused".into()
    });
    for i in 0..IPHONES {
        w.set_app_state(i, AppState::Background);
    }
    w.run_s(5);
    // Every pair shielded: all links drop, every member is lost.
    set_shield(&mut w, true);
    w.run_ms(SHIELD_MS);
    o.check(w.up_links().is_empty(), || {
        format!("{} links survived the shielding", w.up_links().len())
    });
    let first_new_link = w.links.len();
    set_shield(&mut w, false);
    let back = w.now;
    let all: Vec<usize> = (0..N).collect();
    let joined = w.run_until_pred(RECONNECT_BOUND_MS, 100, |w| {
        w.components().len() == 1 && all_reachable(w, &all)
    });
    o.metric(
        "reconnect.s",
        joined.map_or(f64::INFINITY, |t| (t.0 - back.0) as f64 / 1e6),
        joined.is_some(),
        "≤ 30 s after back in range",
    );
    // Stays connected while the geometry allows it (it always does here).
    let mut apart = 0u32;
    let hold_end = w.now.plus_ms(HOLD_MS);
    while w.now < hold_end {
        w.run_ms(500);
        if w.components().len() != 1 {
            apart += 1;
        }
    }
    o.metric(
        "apart.samples",
        f64::from(apart),
        apart == 0,
        "connected at every sample",
    );
    // Roles of every link formed with the iPhones in the background.
    let (mut i2a, mut a2a) = (0u32, 0u32);
    for l in &w.links[first_new_link..] {
        let (c, p) = (l.ends[0].node, l.ends[1].node);
        let (pc, pp) = (w.nodes[c].spec.platform, w.nodes[p].spec.platform);
        match (pc, pp) {
            (Platform::Ios, Platform::Android) => i2a += 1,
            (Platform::Android, Platform::Android) => a2a += 1,
            _ => o.failures.push(format!(
                "link {c} ({pc:?}) → {p} ({pp:?}) formed with the iPhones backgrounded"
            )),
        }
    }
    o.report("links.iphoneToAndroid", f64::from(i2a));
    o.report("links.androidToAndroid", f64::from(a2a));
    o.check(i2a >= IPHONES as u32, || {
        format!("only {i2a} iPhone → Android links for {IPHONES} backgrounded iPhones")
    });
    o.absorb(&w);
    (o, w)
}

fn set_shield(w: &mut World, on: bool) {
    for a in 0..N {
        for b in a + 1..N {
            w.block(a, b, on);
        }
    }
}
