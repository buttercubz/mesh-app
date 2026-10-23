//! World-level behaviour of the §13.2 models with real engines in Lab mode: link
//! formation, determinism, supervision drop and rediscovery, the node airtime budget
//! read from `Limits`, the §9.2 role rule on mixed fleets, iPhone background
//! visibility and the web presets.

use mesh_api::{LabAnchor, LabAudio, LabBench, LabPattern, LabTransport};
use mesh_core::{Instant, Limits, Platform};
use mesh_sim::scenario::lab;
use mesh_sim::{AppState, NodeSpec, Preset, PresetOptions, World, WorldConfig};

const CODE: u16 = 4_321;

fn lab_world(cfg: WorldConfig) -> World {
    let mut w = World::new(cfg);
    w.start_lab_all(CODE);
    w
}

fn assert_clean(w: &World) {
    assert!(
        w.audit.violations.is_empty(),
        "global assertions: {:#?}",
        w.audit.violations
    );
}

#[test]
fn a_pair_links_up_and_identical_seeds_give_identical_worlds() {
    let run = |seed: u64| {
        let mut w = lab_world(Preset::Pair.config(seed, PresetOptions::default()));
        w.run_s(20);
        assert_clean(&w);
        assert_eq!(
            w.links_between(0, 1).len(),
            1,
            "one Lab link between the pair"
        );
        assert_eq!(lab::lab_links(&w, 0), 1);
        (w.digest(), w.stats, w.audit.frames)
    };
    let a = run(11);
    assert_eq!(a, run(11), "same seed, same world");
    assert_ne!(a.0, run(12).0, "another seed, another world");
    assert!(
        a.2.lab > 0 && a.2.sec == 0,
        "Lab links carry LAB frames only: {:?}",
        a.2
    );
}

#[test]
fn out_of_range_links_drop_after_the_supervision_window_and_rejoin() {
    let mut w = lab_world(WorldConfig::iphones_on_line(5, 2, 5.0));
    assert!(lab::wait_links(&mut w, 0, 1, 15_000), "pair never linked");
    let moved = w.now;
    w.mobility.set_position(1, (400.0, 0.0));
    while !w.links_between(0, 1).is_empty() {
        assert!(w.now.since(moved) < 10_000_000, "link never dropped");
        w.run_ms(50);
    }
    let dropped_after = w.now.since(moved);
    // 3 s below PDR 0.5 (§13.2), checked on the 100 ms radio tick.
    assert!(
        (2_900_000..=3_400_000).contains(&dropped_after),
        "dropped after {dropped_after} µs"
    );
    w.run_s(5);
    assert_eq!(lab::lab_links(&w, 0), 0, "the engine saw the link go down");
    let back = w.now;
    w.mobility.set_position(1, (5.0, 0.0));
    assert!(lab::wait_links(&mut w, 0, 1, 20_000), "never relinked");
    assert!(w.now.since(back) <= 15_000_000);
    assert_clean(&w);
}

#[test]
fn a_connect_outstanding_while_the_peer_is_out_of_reach_completes_when_it_returns() {
    // iOS `connect` never times out by itself (§9.3) and Android `connectGatt` waits
    // too: a target that is out of reach when the connect starts is connected once it
    // is back, by the same attempt, before the engine's 8 s timeout aborts it.
    for seed in 1..=4 {
        let mut w = lab_world(WorldConfig::iphones_on_line(seed, 2, 5.0));
        let start = w.now;
        while w.nodes.iter().all(|n| n.pending.is_empty()) {
            assert!(w.now.since(start) < 15_000_000, "no connect");
            w.run_ms(1);
        }
        let connect = w.now;
        let i = usize::from(w.nodes[0].pending.is_empty());
        let disc = *w.nodes[i].pending.keys().next().unwrap();
        // Out of reach past the longest connect + HELLO read (3 s).
        w.block(0, 1, true);
        w.run_ms(3_200);
        assert!(w.links_between(0, 1).is_empty());
        assert!(
            w.nodes[i].pending.contains_key(&disc),
            "seed {seed}: still outstanding"
        );
        w.block(0, 1, false);
        assert!(
            lab::wait_links(&mut w, i, 1, 4_500),
            "seed {seed}: never linked"
        );
        assert!(
            w.now.since(connect) < 8_000_000,
            "seed {seed}: linked {} µs after the connect, by a later attempt",
            w.now.since(connect)
        );
        assert_clean(&w);
    }
}

/// Bytes node 0 delivered to node 1 during a 5 s L2CAP saturate run.
fn saturate_bytes(budget_bps: Option<u32>) -> u64 {
    let mut cfg = WorldConfig::iphones_on_line(21, 2, 3.0);
    if let Some(b) = budget_bps {
        let mut l = Limits::for_platform(Platform::Ios);
        l.routing.node_ble_budget_bps = b;
        cfg.limits = Some(l);
    }
    let mut w = lab_world(cfg);
    assert!(lab::wait_links(&mut w, 0, 1, 15_000));
    let li = w.links_between(0, 1)[0];
    // The direction whose sender is node 0.
    let d = w.links[li]
        .ends
        .iter()
        .position(|e| e.node == 0)
        .expect("node 0 is an end");
    let mut p = lab::params(LabAudio::Off, LabAnchor::None, 5);
    p.transport = Some(LabTransport::L2cap);
    p.pattern = Some(LabPattern::Saturate);
    let before = w.links[li].dirs[d].bytes;
    let run = lab::start(&mut w, 0, LabBench::T0, p).expect("labStart");
    assert!(lab::wait_finished(&mut w, 0, &run, 20_000));
    assert_clean(&w);
    w.links[li].dirs[d].bytes - before
}

#[test]
fn the_node_airtime_budget_comes_from_limits() {
    let default = saturate_bytes(None) as f64 * 8.0 / 5.0;
    let tight = saturate_bytes(Some(40_000)) as f64 * 8.0 / 5.0;
    // Default: the 120 kbps link serialization binds (budget 180 kbps).
    assert!(
        (90_000.0..=125_000.0).contains(&default),
        "default {default} bit/s"
    );
    // 40 kbps budget: the node cannot exceed it, whatever the link could carry.
    assert!(tight <= 40_000.0 * 1.10, "tight budget {tight} bit/s");
    assert!(
        tight >= 20_000.0,
        "tight budget still carries traffic: {tight} bit/s"
    );
}

#[test]
fn android_never_initiates_a_link_to_an_iphone() {
    let nodes = vec![
        NodeSpec::iphone(),
        NodeSpec::android(),
        NodeSpec::iphone(),
        NodeSpec::android(),
    ];
    let cfg = WorldConfig::new(
        8,
        nodes,
        mesh_sim::Mobility::Static(vec![(0.0, 0.0), (4.0, 0.0), (0.0, 4.0), (4.0, 4.0)]),
    );
    let mut w = lab_world(cfg);
    w.run_s(40);
    assert_clean(&w);
    let mut cross = 0;
    for l in &w.links {
        let (a, b) = (l.ends[0].node, l.ends[1].node);
        let (pa, pb) = (w.nodes[a].spec.platform, w.nodes[b].spec.platform);
        assert!(
            !(pa == Platform::Android && pb == Platform::Ios),
            "Android {a} initiated a link to iPhone {b}"
        );
        if pa != pb {
            cross += 1;
            // iPhone-central links run at the iOS connection interval.
            assert_eq!(l.ci_us, 30_000);
        }
    }
    assert!(
        cross > 0,
        "no iPhone↔Android link formed: {:?}",
        w.components()
    );
    assert_eq!(w.components().len(), 1, "the four phones are connected");
}

#[test]
fn background_iphones_do_not_find_each_other_by_default() {
    let mut cfg = WorldConfig::iphones_on_line(3, 2, 3.0);
    for n in &mut cfg.nodes {
        n.app = AppState::Background;
    }
    let mut w = lab_world(cfg);
    w.run_s(30);
    assert!(w.links.is_empty(), "bg↔bg iPhones discovered each other");
    // One side in the foreground: the backgrounded one is found in its overflow area.
    w.set_app_state(1, AppState::Foreground);
    assert!(
        lab::wait_links(&mut w, 0, 1, 30_000),
        "fg↔bg pair never linked"
    );
    assert_clean(&w);
}

#[test]
fn every_web_preset_runs_clean_within_the_link_max() {
    for p in Preset::ALL {
        let opts = PresetOptions {
            nodes: None,
            loss: Some(0.05),
        };
        let mut cfg = p.config(31, opts);
        // Lab engines have no §5.7 over-max policy (WP-10D): in dense presets two
        // initiators may read one `accepting` HELLO at once. Report-only here.
        cfg.link_cap_grace_us = None;
        let mut w = lab_world(cfg);
        w.run_until(Instant::ZERO.plus_ms(40_000));
        assert_clean(&w);
        let max = Limits::for_platform(Platform::Ios).ble_max() as usize;
        for i in 0..w.nodes.len() {
            // At most one extra link from that race.
            assert!(
                w.nodes[i].active_links(true) <= max + 1,
                "{} node {i}",
                p.name()
            );
        }
        if matches!(p, Preset::Pair | Preset::Chain5) {
            assert_eq!(w.audit.over_max_episodes, 0, "{}", p.name());
        }
        assert!(!w.up_links().is_empty(), "{}: no link formed", p.name());
        if p == Preset::Cluster15 {
            assert_eq!(
                w.components().len(),
                1,
                "cluster15 is one mesh: {:?}",
                w.components()
            );
        }
    }
}
