//! S2 chain5-ble (DESIGN §13.2, §5.5, §7.9): five iPhones on a road, 45 m apart (pocket
//! R50 50 m: neighbors link, two-apart do not). The two ends take turns in PTT for two
//! minutes. Asserted: listeners at 1 / 2 hops p95 ≤ 210 / 250 ms modeled mouth-to-ear
//! excluding headset legs (≤ 260 / 300 ms with the 50 ms T5a leg budget, KPI3); 3-hop
//! p95 reported (best effort); loss ≤ 3 % at ≤ 3 hops; the 4-hop listener gets frames
//! only while their age is ≤ `VOICE_MAX_AGE` (150 ms); cap 2 on every link (snapshot
//! cap and the global stream-cap assertion).

use mesh_api::TalkMode;

use super::Outcome;
use super::common::{Scope, everyone, form_ride, go_live_all, group_config, voice_stats};
use crate::config::WorldConfig;
use crate::presets::CHAIN_SPACING_M;
use crate::rng::SimRng;
use crate::talk::turns;
use crate::world::World;

const N: usize = 5;
const TALK_US: u64 = 120_000_000;
/// The T5a headset-leg budget added to reach KPI3 (§7.9).
pub const LEG_BUDGET_MS: f64 = 50.0;

pub fn run(seed: u64) -> Outcome {
    run_world(seed).0
}

/// Runs S2 and returns the outcome and the final world (diagnostics).
pub fn run_world(seed: u64) -> (Outcome, World) {
    let mut o = Outcome::new("S2", seed);
    let cfg = group_config(WorldConfig::iphones_on_line(seed, N, CHAIN_SPACING_M));
    let mut w = World::new(cfg);
    let (g, formed) = form_ride(&mut w, N / 2);
    if !o.check(formed.is_some(), || "the chain never formed a ride".into()) {
        o.absorb(&w);
        return (o, w);
    }
    o.report("form.s", formed.map_or(0.0, |t| t.0 as f64 / 1e6));
    w.run_s(20);
    o.check(go_live_all(&mut w, &g, TalkMode::Ptt, true), || {
        "go live refused".into()
    });
    let mut rng = SimRng::new(seed, 0x52_0001);
    let start = w.now.0 + 300_000;
    let spans = turns(
        &mut rng,
        &[0, N - 1],
        start,
        start + TALK_US,
        (2_000, 4_000),
        (1_000, 2_000),
    );
    for sp in &spans {
        w.script_talk(sp.node, sp.from_us, sp.until_us, true);
    }
    let end = spans.last().map_or(start, |s| s.until_us) + 2_000_000;
    w.run_until(mesh_core::Instant(end));
    let st = voice_stats(
        &w,
        &g,
        w.talk_spans(),
        &Scope {
            listener: &everyone,
            solo_only: true,
        },
    );
    for (h, bound) in [(1u8, 210.0), (2, 250.0)] {
        let p95 = st.m2e_ms(h, 95.0);
        o.check(st.samples(h) >= 50, || {
            format!("only {} latency samples at {h} hops", st.samples(h))
        });
        o.metric(
            &format!("m2e.p95.{h}hop.ms"),
            p95.unwrap_or(f64::INFINITY),
            p95.is_some_and(|p| p <= bound),
            &format!("≤ {bound} ms excl. legs"),
        );
        if let Some(p) = p95 {
            o.report(&format!("m2e.p95.{h}hop.withLegs.ms"), p + LEG_BUDGET_MS);
        }
    }
    if let Some(p) = st.m2e_ms(3, 95.0) {
        o.report("m2e.p95.3hop.ms", p);
    }
    let loss = st.loss(3);
    o.metric("loss.le3hops", loss, loss <= 0.03, "≤ 3 %");
    // The 4-hop listener only gets frames within the age cap.
    let max_age = st_max_age(&w, 4);
    let cap_us = w.cfg.limits_for(0).routing.voice_max_age_us;
    o.metric(
        "age.max.4hop.ms",
        max_age as f64 / 1_000.0,
        max_age <= cap_us + 2_000,
        "≤ VOICE_MAX_AGE (150 ms)",
    );
    // Cap 2 on every link: the snapshot cap of every rider stays 2 while voice flows.
    for (i, n) in w.nodes.iter().enumerate() {
        let low = n
            .cap_hist
            .iter()
            .filter(|(t, _)| *t >= start && *t <= end)
            .map(|x| x.1)
            .min();
        let before = n
            .cap_hist
            .iter()
            .take_while(|(t, _)| *t < start)
            .last()
            .map(|x| x.1);
        let low = low.into_iter().chain(before).min().unwrap_or(2);
        o.check(low >= 2, || format!("rider {i}: BLE cap dropped to {low}"));
    }
    o.absorb(&w);
    (o, w)
}

/// Highest header age of the frames that arrived after `traversals` link traversals.
fn st_max_age(w: &World, traversals: u8) -> u64 {
    w.nodes
        .iter()
        .flat_map(|n| {
            n.tap
                .borrow()
                .frames
                .values()
                .filter(|f| f.traversals == traversals)
                .map(|f| u64::from(f.age_us))
                .collect::<Vec<_>>()
        })
        .max()
        .unwrap_or(0)
}
