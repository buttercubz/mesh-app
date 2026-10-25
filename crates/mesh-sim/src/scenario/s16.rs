//! S16 hybrid (Wi-Fi backbone, BLE edge): four Android phones on Wi-Fi Aware with each
//! other (a ring: every Aware link is an upgrade of a BLE link, so the ring is what the
//! BLE ranges allow) and two iPhones on BLE only, each BLE-linked to one Android relay.
//!
//! ```text
//!   P0 ── A0 ═══ A1          ═══ Aware (plus the BLE fallback link)
//!         ║      ║           ─── BLE only
//!         A3 ═══ A2 ── P1
//! ```
//!
//! Phase 1: the four Androids talk at once (starts 400 ms apart, so rank = start
//! order). The Aware island must carry all four (S_MAX_AWARE), never throttled to the
//! BLE cap because some link of the group is BLE; each relay's BLE edge carries only
//! the top `cap(L)` = 2 streams (shed on that edge, no FLOOR_REJECT). Phase 2: each
//! iPhone talks (P1 next to one Android talker) and is heard by everyone.
//!
//! Asserted: no `talkDenied` for the island talkers and ≥ 95 % frame delivery among
//! the Androids; each iPhone receives ≤ 2 streams at once and ≥ 95 % of the frames of
//! the two top-ranked streams; each iPhone talker reaches every other node (≥ 95 %);
//! no voice on a BLE link between two nodes that hold a working Aware link (after the
//! 1 s bundle-switch overlap); no `talkDenied{congestion}` at all.
//!
//! Variants (same assertions):
//! - **S16b** ([`run_ble60`]): BLE links near the DESIGN per-link budget (64 kbps
//!   physical; the budget is 60 kbps, §4.8, and a physical rate of exactly 60 kbps
//!   leaves the pacer no headroom for `cap_pacing = 2`). The leader A0 stays silent and
//!   A1, A3, A2 talk in that order, so the two top-ranked streams come from origins
//!   whose only voice egress is Aware (their BLE links are fallbacks to Aware peers).
//!   Their streams still cross the BLE edges to the iPhones, so they must be BLE-profile
//!   streams (§7.4): two Wi-Fi-profile streams (20 ms frames, 2 × 34.4 + 3 = 71.8 kbps)
//!   overload an edge admitted for two BLE16 streams (50.2 kbps).
//! - **S16c** ([`run_star`]): the Aware island is a star on A1 (the leaves A0, A2 and
//!   A3 hold one Aware link plus its BLE fallback to A1; A1 creates the group), talk
//!   order A1, A0, A2, A3. A leaf's BLE fallback link to its only Aware neighbor carries
//!   no voice, so it must not bound the leaf's `cap_local`: the leaves A2 and A3 are the
//!   3rd and 4th talkers and all four island talkers are admitted. The hub needs three
//!   BLE links (above the per-node target of 2), so formation gets a second
//!   `FORM_LIMIT_MS` window here; formation is not what this variant checks.

use mesh_api::{TalkDeniedReason, TalkMode};

use super::Outcome;
use super::common::{form_ride, group_config};
use crate::config::{NodeSpec, WorldConfig};
use crate::mobility::Mobility;
use crate::ride::RideGroup;
use crate::talk::TalkSpan;
use crate::world::World;

/// Android nodes (the Aware island) and iPhones (BLE only).
pub const ANDROIDS: [usize; 4] = [0, 1, 2, 3];
pub const IPHONES: [usize; 2] = [4, 5];
/// The Android relay each iPhone is BLE-linked to.
pub const RELAY_OF: [(usize, usize); 2] = [(4, 0), (5, 2)];

/// Island phase length and start stagger (µs).
const ISLAND_US: u64 = 15_000_000;
const STAGGER_US: u64 = 400_000;
/// iPhone turn length (µs).
const TURN_US: u64 = 6_000_000;
/// Bundle switch overlap: both links carry voice this long after a switch (§9.7).
const OVERLAP_US: u64 = 1_000_000;
/// Frames must arrive within this long after the span end to count.
const LATE_US: u64 = 500_000;

/// One S16 variant.
#[derive(Debug, Clone, Copy)]
struct Variant {
    id: &'static str,
    /// Star island on A1 instead of the ring.
    star: bool,
    /// BLE link rate override (bps).
    ble_link_bps: Option<u64>,
    /// Island talkers in start order (rank = start order).
    order: &'static [usize],
    /// The node that creates the group (and leads the ride).
    leader: usize,
}

const RING: Variant = Variant {
    id: "S16",
    star: false,
    ble_link_bps: None,
    order: &ANDROIDS,
    leader: 0,
};
const BLE60: Variant = Variant {
    id: "S16b",
    star: false,
    ble_link_bps: Some(64_000),
    order: &[1, 3, 2],
    leader: 0,
};
const STAR: Variant = Variant {
    id: "S16c",
    star: true,
    ble_link_bps: None,
    order: &[1, 0, 2, 3],
    // The hub creates the group, so every node is at most two BLE hops from it.
    leader: 1,
};

/// Positions (m): a 40 m Android square (sides inside BLE range, diagonals at 56.6 m
/// outside it at the pocket R50 of 50 m) with an iPhone 30 m off two opposite corners.
/// Star: A0, A2 and A3 within BLE range of A1 only (40 m; 56.6 m or more between
/// leaves), P0 30 m off A0 and P1 30 m off A3.
fn positions(star: bool) -> Vec<(f64, f64)> {
    if star {
        return vec![
            (0.0, 0.0),
            (40.0, 0.0),
            (80.0, 0.0),
            (40.0, 40.0),
            (-30.0, 0.0),
            (40.0, 70.0),
        ];
    }
    vec![
        (0.0, 0.0),
        (40.0, 0.0),
        (40.0, 40.0),
        (0.0, 40.0),
        (-21.0, -21.0),
        (61.0, 61.0),
    ]
}

/// The S16 world.
pub fn world(seed: u64) -> World {
    variant_world(seed, RING)
}

fn variant_world(seed: u64, v: Variant) -> World {
    let mut nodes: Vec<NodeSpec> = ANDROIDS
        .iter()
        .map(|i| {
            let mut s = NodeSpec::android();
            s.model = format!("Pixel {}", 7 + i);
            s
        })
        .collect();
    for i in 0..IPHONES.len() {
        let mut s = NodeSpec::iphone();
        s.model = format!("iPhone15,{}", 2 + i);
        nodes.push(s);
    }
    let mut cfg = group_config(WorldConfig::new(
        seed,
        nodes,
        Mobility::Static(positions(v.star)),
    ));
    if let Some(bps) = v.ble_link_bps {
        cfg.radio.ble_link_bps = bps;
    }
    World::new(cfg)
}

/// Up Aware links between Androids.
fn aware_links(w: &World) -> usize {
    w.links
        .iter()
        .filter(|l| l.up && l.kind == mesh_core::LinkKind::WifiAware)
        .count()
}

/// Share of the 10 ms ticks of `sp` that `listener` received (first copies within the
/// span, plus [`LATE_US`]).
pub fn delivered(w: &World, g: &RideGroup, sp: &TalkSpan, listener: usize) -> f64 {
    let Some(origin) = w.node_id(sp.node, &g.id) else {
        return 0.0;
    };
    let expected = (sp.until_us - sp.from_us) / 10_000;
    let tap = w.nodes[listener].tap.borrow();
    let got: u64 = tap
        .frames
        .iter()
        .filter(|(k, f)| {
            let (o, ..) = crate::tap::unpack_frame(**k);
            o == origin && f.t >= sp.from_us && f.t < sp.until_us + LATE_US
        })
        .map(|(_, f)| u64::from(f.ticks.max(1)))
        .sum();
    if expected == 0 {
        0.0
    } else {
        (got as f64 / expected as f64).min(1.0)
    }
}

/// Voice burst spans received on a BLE link between two nodes that held an Up Aware
/// link at the time (outside the bundle-switch overlap): `(count, total ms)`.
pub fn ble_voice_beside_aware(w: &World, after_us: u64) -> (usize, u64) {
    let mut n = 0usize;
    let mut ms = 0u64;
    for (i, node) in w.nodes.iter().enumerate() {
        let tap = node.tap.borrow();
        for ((link, _), segs) in &tap.spans {
            for &(a, b) in segs {
                if b < after_us {
                    continue;
                }
                let live = |l: &crate::link::SimLink, t: u64| {
                    l.created_at <= t && l.down_at.is_none_or(|d| d > t)
                };
                let Some(ble) = w.links.iter().find(|l| {
                    l.kind.is_ble()
                        && l.ends.iter().any(|e| e.node == i && e.link.0 == *link)
                        && live(l, a)
                }) else {
                    continue;
                };
                let Some(peer) = ble.peer(i) else { continue };
                let aware_since = w
                    .links
                    .iter()
                    .filter(|l| l.kind == mesh_core::LinkKind::WifiAware && l.joins(i, peer))
                    .filter(|l| live(l, a) && live(l, b))
                    .map(|l| l.created_at)
                    .min();
                if let Some(t) = aware_since
                    && a >= t + OVERLAP_US
                {
                    n += 1;
                    ms += (b - a) / 1_000;
                }
            }
        }
    }
    (n, ms)
}

pub fn run(seed: u64) -> Outcome {
    run_variant(seed, RING)
}

/// S16b: BLE at 60 kbps, two Aware-only origins on top.
pub fn run_ble60(seed: u64) -> Outcome {
    run_variant(seed, BLE60)
}

/// S16c: star-shaped Aware island (leaves with a single Aware neighbor).
pub fn run_star(seed: u64) -> Outcome {
    run_variant(seed, STAR)
}

fn run_variant(seed: u64, v: Variant) -> Outcome {
    let mut o = Outcome::new(v.id, seed);
    let mut w = variant_world(seed, v);
    let (g, mut formed) = form_ride(&mut w, v.leader);
    if formed.is_none() && v.star {
        let all: Vec<usize> = (0..w.nodes.len()).collect();
        formed = w.run_until_pred(super::common::FORM_LIMIT_MS, 250, |w| {
            super::common::all_reachable(w, &all)
        });
    }
    if !o.check(formed.is_some(), || "the hybrid ride never formed".into()) {
        o.absorb(&w);
        return o;
    }
    for &i in &ANDROIDS {
        w.set_aware_radio(i, true);
    }
    // Every BLE link of the Android ring (star) upgrades (lower node_id offers, one per
    // tick).
    let need = if v.star { 3 } else { 4 };
    let up = w.run_until_pred(60_000, 250, |w| aware_links(w) >= need);
    o.metric(
        "aware.links",
        aware_links(&w) as f64,
        up.is_some(),
        "≥ 4 ring / 3 star (the Android island on Aware)",
    );
    w.run_ms(3_000);
    w.start_ride(g.leader);
    w.run_ms(300);
    let mut ok = true;
    for i in 0..w.nodes.len() {
        w.mute_mic(i);
        w.track_onsets(i);
        ok &= w.go_live(i, TalkMode::Ptt);
    }
    w.run_ms(300);
    o.check(ok, || "go live refused".into());

    // Phase 1: four island talkers at once.
    let t0 = w.now.0 + 1_000_000;
    for (k, &a) in v.order.iter().enumerate() {
        w.script_talk(a, t0 + k as u64 * STAGGER_US, t0 + ISLAND_US, true);
    }
    // Phase 2: P0 alone, then P1 next to one Android talker (A1).
    let t1 = t0 + ISLAND_US + 2_000_000;
    let t2 = t1 + TURN_US + 2_000_000;
    w.script_talk(IPHONES[0], t1, t1 + TURN_US, true);
    w.script_talk(IPHONES[1], t2, t2 + TURN_US, true);
    w.script_talk(1, t2 + 300_000, t2 + TURN_US, true);
    let end = t2 + TURN_US + 2_000_000;
    w.run_until(mesh_core::Instant(end));

    let spans: Vec<TalkSpan> = w.talk_spans().to_vec();
    // talkDenied: island talkers during phase 1, congestion anywhere.
    let mut island_denied = 0usize;
    let mut congestion = 0usize;
    for (i, n) in w.nodes.iter().enumerate() {
        for (t, reason) in &n.denied {
            if *t < t0 {
                continue;
            }
            if *reason == TalkDeniedReason::Congestion {
                congestion += 1;
            }
            if ANDROIDS.contains(&i) && *t < t0 + ISLAND_US {
                island_denied += 1;
            }
        }
    }
    o.metric(
        "island.denied",
        island_denied as f64,
        island_denied == 0,
        "= 0 (4 talkers fit S_MAX_AWARE)",
    );
    o.metric(
        "denied.congestion",
        congestion as f64,
        congestion == 0,
        "= 0",
    );

    let island: Vec<&TalkSpan> = spans
        .iter()
        .filter(|s| ANDROIDS.contains(&s.node) && s.from_us < t0 + ISLAND_US)
        .collect();
    o.metric(
        "island.talkers",
        island.len() as f64,
        island.len() == v.order.len(),
        "= every scripted island talker",
    );
    let mut worst = 1.0f64;
    for sp in &island {
        for j in (0..w.nodes.len()).filter(|j| *j != sp.node) {
            if ANDROIDS.contains(&j) {
                worst = worst.min(delivered(&w, &g, sp, j));
            }
        }
    }
    o.metric(
        "island.delivery.min",
        worst,
        worst >= 0.95,
        "≥ 0.95 (every Android hears every island talker)",
    );

    // iPhones: ≤ 2 streams at once, ≥ 95 % of the two top-ranked (first) streams.
    let streams = w.link_streams();
    for &p in &IPHONES {
        let max = streams
            .iter()
            .filter(|s| s.node == p)
            .map(|s| s.max)
            .max()
            .unwrap_or(0);
        o.metric(
            &format!("iphone{p}.streams.max"),
            max as f64,
            max <= 2,
            "≤ 2 (BLE edge cap)",
        );
        let mut top = 1.0f64;
        for sp in island.iter().filter(|s| v.order[..2].contains(&s.node)) {
            top = top.min(delivered(&w, &g, sp, p));
        }
        o.metric(
            &format!("iphone{p}.top2.delivery.min"),
            top,
            top >= 0.95,
            "≥ 0.95",
        );
        let mut low = 0.0f64;
        for sp in island.iter().filter(|s| !v.order[..2].contains(&s.node)) {
            low = low.max(delivered(&w, &g, sp, p));
        }
        o.report(&format!("iphone{p}.shed.delivery.max"), low);
    }
    for &(p, relay) in &RELAY_OF {
        o.report(&format!("iphone{p}.relay"), relay as f64);
    }

    // iPhone talkers reach everyone.
    for &p in &IPHONES {
        let mut heard = 1.0f64;
        let mut any = false;
        for sp in spans.iter().filter(|s| s.node == p) {
            any = true;
            for j in (0..w.nodes.len()).filter(|j| *j != p) {
                heard = heard.min(delivered(&w, &g, sp, j));
            }
        }
        o.metric(
            &format!("iphone{p}.heard.min"),
            if any { heard } else { 0.0 },
            any && heard >= 0.95,
            "≥ 0.95 (admitted and heard by every other node)",
        );
    }

    let (dups, dup_ms) = ble_voice_beside_aware(&w, t0);
    o.metric(
        "ble.voice.beside.aware",
        dups as f64,
        dups == 0,
        "= 0 (voice to a neighbor on Aware goes over Aware only)",
    );
    o.report("ble.voice.beside.aware.ms", dup_ms as f64);
    o.absorb(&w);
    o
}
