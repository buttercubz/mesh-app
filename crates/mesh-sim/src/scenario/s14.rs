//! S14 fading-pair (field test 2026-10-24): an iPhone and an Android tablet take turns
//! talking in PTT over one BLE link, (a) at 10.5 m inside a ground-reflection fading
//! null ([`crate::radio::FadeNull`]: every BLE attempt succeeds with p ≈ 0.6, so the
//! reliable link layer pays it in retransmissions, i.e. RTT and `WouldBlock`), (b) at
//! 21 m past the null, and (c) at the edge of range (no null, PDR ≈ 0.65 from distance).
//!
//! The field symptom was a "Network busy" banner at ~10-11 m while twice the distance
//! worked: the delay-based pacer read the retransmission delay of a weak link as
//! congestion, cut `R` below the cost of one stream, `cap_pacing` fell to 0 and even a
//! lone talker was refused with `FLOOR_REJECT{CONGESTION}`. Asserted in every case: no
//! `talkDenied{congestion}` for a lone talker and the listener gets most of the speech; in the clean case
//! frame loss ≤ 1 %.

use mesh_api::{TalkDeniedReason, TalkMode};

use super::Outcome;
use super::common::{Scope, everyone, form_ride, go_live_all, group_config, voice_stats};
use crate::config::{NodeSpec, WorldConfig};
use crate::mobility::Mobility;
use crate::radio::FadeNull;
use crate::rng::SimRng;
use crate::talk::{TalkSpan, turns};
use crate::world::World;

/// Talk time per case (µs).
const TALK_US: u64 = 60_000_000;

/// The null of case (a): deepest at 10.5 m, PDR × 0.6 there.
pub const FIELD_NULL: FadeNull = FadeNull {
    center_m: 10.5,
    half_width_m: 3.0,
    depth_pdr: 0.6,
};

/// One case of the scenario.
#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub distance_m: f64,
    pub null: Option<FadeNull>,
    /// Minimum share of the scripted 40 ms frames the listener must receive.
    pub min_delivered: f64,
    /// Maximum frame loss over the bursts that went out (`None` = report only).
    pub max_loss: Option<f64>,
}

/// The three cases of S14.
pub const CASES: [Case; 3] = [
    Case {
        name: "null10m",
        distance_m: 10.5,
        null: Some(FIELD_NULL),
        min_delivered: 0.80,
        max_loss: None,
    },
    Case {
        name: "clear21m",
        distance_m: 21.0,
        null: Some(FIELD_NULL),
        min_delivered: 0.90,
        max_loss: Some(0.01),
    },
    Case {
        name: "edge",
        distance_m: 48.8,
        null: None,
        min_delivered: 0.75,
        max_loss: None,
    },
];

/// The world of one case: node 0 an iPhone (central), node 1 an Android tablet.
pub fn world(seed: u64, case: &Case) -> World {
    world_with(seed, case, &|_| {})
}

/// [`world`] with a tweak of the configuration (radio model) applied last.
pub fn world_with(seed: u64, case: &Case, tweak: &dyn Fn(&mut WorldConfig)) -> World {
    let mut tablet = NodeSpec::android();
    tablet.model = "SM-X710".into();
    let mut iphone = NodeSpec::iphone();
    iphone.model = "iPhone15,5".into();
    let mut cfg = group_config(WorldConfig::new(
        seed,
        vec![iphone, tablet],
        Mobility::line(2, case.distance_m),
    ));
    cfg.radio.fade_null = case.null;
    tweak(&mut cfg);
    World::new(cfg)
}

/// What one case measured.
#[derive(Debug, Clone, Default)]
pub struct CaseResult {
    pub formed: bool,
    pub denied_congestion: usize,
    pub denied_other: usize,
    pub delivered: f64,
    pub loss: f64,
    /// Modeled mouth-to-ear p95 (ms), when measured.
    pub m2e_p95_ms: Option<f64>,
    /// Lowest pacer rate any BLE link of the pair was paced at while talking (bps).
    pub min_rate_bps: u32,
    /// Lowest member-row quality (0-100) either phone showed for the other one.
    pub min_quality: u8,
}

/// Runs one case: form, go live, alternate PTT turns, measure.
pub fn run_case(seed: u64, case: &Case, o: &mut Outcome) -> CaseResult {
    run_case_with(seed, case, TALK_US, &|_| {}, o)
}

/// [`run_case`] with `talk_us` of turns and a radio-model tweak.
pub fn run_case_with(
    seed: u64,
    case: &Case,
    talk_us: u64,
    tweak: &dyn Fn(&mut WorldConfig),
    o: &mut Outcome,
) -> CaseResult {
    let mut r = CaseResult {
        min_rate_bps: u32::MAX,
        min_quality: u8::MAX,
        ..CaseResult::default()
    };
    let mut w = world_with(seed, case, tweak);
    let (g, formed) = form_ride(&mut w, 0);
    r.formed = formed.is_some();
    if !o.check(r.formed, || {
        format!("{}: the pair never formed a ride", case.name)
    }) {
        o.absorb(&w);
        return r;
    }
    o.check(go_live_all(&mut w, &g, TalkMode::Ptt, true), || {
        format!("{}: go live refused", case.name)
    });
    let mut rng = SimRng::new(seed, 0x51_4001);
    let start = w.now.0 + 1_000_000;
    let spans = turns(
        &mut rng,
        &[0, 1],
        start,
        start + talk_us,
        (3_000, 6_000),
        (800, 1_500),
    );
    for sp in &spans {
        w.script_talk(sp.node, sp.from_us, sp.until_us, true);
    }
    let end = spans.last().map_or(start, |s| s.until_us) + 2_000_000;
    while w.now.0 < end {
        let t = (w.now.0 + 250_000).min(end);
        w.run_until(mesh_core::Instant(t));
        for n in &w.nodes {
            if let Some(snap) = n.snapshot.as_ref() {
                for m in snap
                    .members
                    .iter()
                    .filter(|m| m.status == mesh_api::MemberStatus::Direct)
                {
                    r.min_quality = r.min_quality.min(m.quality);
                }
            }
            for l in n.engine.ctx().links.iter() {
                if l.info.kind.is_ble()
                    && let Some(rate) = l.pace.rate_bps
                {
                    r.min_rate_bps = r.min_rate_bps.min(rate);
                }
            }
        }
    }
    for n in &w.nodes {
        for (t, reason) in &n.denied {
            if *t < start {
                continue;
            }
            if *reason == TalkDeniedReason::Congestion {
                r.denied_congestion += 1;
            } else {
                r.denied_other += 1;
            }
        }
    }
    let st = voice_stats(
        &w,
        &g,
        w.talk_spans(),
        &Scope {
            listener: &everyone,
            solo_only: true,
        },
    );
    r.loss = st.loss(15);
    r.m2e_p95_ms = st.m2e_range_ms(1, 1, 95.0);
    r.delivered = delivered_share(&w, &g, w.talk_spans());
    o.absorb(&w);
    r
}

/// Share of the scripted speech (40 ms frames over every span) that reached the other
/// node: refused bursts count as missing, unlike [`super::common::VoiceStats::loss`].
pub fn delivered_share(w: &World, g: &crate::ride::RideGroup, spans: &[TalkSpan]) -> f64 {
    let mut expected = 0u64;
    let mut got = 0u64;
    for sp in spans {
        let Some(origin) = w.node_id(sp.node, &g.id) else {
            continue;
        };
        expected += (sp.until_us - sp.from_us) / 40_000;
        let listener = 1 - sp.node;
        let tap = w.nodes[listener].tap.borrow();
        got += tap
            .frames
            .iter()
            .filter(|(k, f)| {
                let (o, ..) = crate::tap::unpack_frame(**k);
                o == origin && f.t >= sp.from_us && f.t < sp.until_us + 500_000
            })
            .map(|(_, f)| u64::from(f.ticks.max(1)) / 4)
            .sum::<u64>();
    }
    if expected == 0 {
        0.0
    } else {
        (got as f64 / expected as f64).min(1.0)
    }
}

pub fn run(seed: u64) -> Outcome {
    let mut o = Outcome::new("S14", seed);
    for case in &CASES {
        let r = run_case(seed, case, &mut o);
        if !r.formed {
            continue;
        }
        let n = case.name;
        o.metric(
            &format!("{n}.denied.congestion"),
            r.denied_congestion as f64,
            r.denied_congestion == 0,
            "= 0 (a lone talker is never refused for congestion)",
        );
        o.report(&format!("{n}.denied.other"), r.denied_other as f64);
        o.report(
            &format!("{n}.pace.min.kbps"),
            f64::from(r.min_rate_bps) / 1e3,
        );
        o.report(&format!("{n}.quality.min"), f64::from(r.min_quality));
        if let Some(p95) = r.m2e_p95_ms {
            o.report(&format!("{n}.m2e.p95.ms"), p95);
        }
        o.metric(
            &format!("{n}.delivered"),
            r.delivered,
            r.delivered >= case.min_delivered,
            &format!("≥ {:.2}", case.min_delivered),
        );
        match case.max_loss {
            Some(max) => {
                o.metric(
                    &format!("{n}.loss"),
                    r.loss,
                    r.loss <= max,
                    &format!("≤ {max}"),
                );
            }
            None => o.report(&format!("{n}.loss"), r.loss),
        }
    }
    o
}
