//! S1 pair-ble (DESIGN §13.2): two iPhones 10 m apart form a ride over the real
//! discovery, Noise and IDPROOF path, go live in PTT and take turns talking for two
//! minutes. Asserted: modeled mouth-to-ear p95 ≤ 190 ms (excluding headset legs), frame
//! loss ≤ 1 %, no frame delivered twice.

use mesh_api::TalkMode;

use super::Outcome;
use super::common::{Scope, everyone, form_ride, go_live_all, group_config, voice_stats};
use crate::config::WorldConfig;
use crate::rng::SimRng;
use crate::talk::turns;
use crate::world::World;

/// Talk time of the scenario (µs).
const TALK_US: u64 = 120_000_000;

pub fn run(seed: u64) -> Outcome {
    let mut o = Outcome::new("S1", seed);
    let cfg = group_config(WorldConfig::iphones_on_line(seed, 2, 10.0));
    let mut w = World::new(cfg);
    let (g, formed) = form_ride(&mut w, 0);
    if !o.check(formed.is_some(), || "the pair never formed a ride".into()) {
        o.absorb(&w);
        return o;
    }
    o.report("form.s", formed.map_or(0.0, |t| t.0 as f64 / 1e6));
    o.check(go_live_all(&mut w, &g, TalkMode::Ptt, true), || {
        "go live refused".into()
    });
    let mut rng = SimRng::new(seed, 0x51_0001);
    let start = w.now.0 + 200_000;
    let spans = turns(
        &mut rng,
        &[0, 1],
        start,
        start + TALK_US,
        (2_000, 4_000),
        (800, 1_500),
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
    let p95 = st.m2e_range_ms(1, 1, 95.0);
    o.check(st.samples(1) >= 100, || {
        format!("only {} latency samples at 1 hop", st.samples(1))
    });
    o.metric(
        "m2e.p95.ms",
        p95.unwrap_or(f64::INFINITY),
        p95.is_some_and(|p| p <= 190.0),
        "≤ 190 ms",
    );
    if let Some(p50) = st.m2e_range_ms(1, 1, 50.0) {
        o.report("m2e.p50.ms", p50);
    }
    let loss = st.loss(15);
    o.metric("loss", loss, loss <= 0.01, "≤ 1 %");
    o.metric(
        "duplicates.delivered",
        st.duplicate_copies as f64,
        st.duplicate_copies == 0,
        "= 0",
    );
    o.report(
        "syllables.matched",
        st.matched as f64 / st.syllables.max(1) as f64,
    );
    o.absorb(&w);
    o
}
