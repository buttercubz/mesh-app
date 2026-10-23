//! First field test regression (DESIGN §5.7, §9.5): two Android phones and an iPhone
//! idle on a table, live in VOX with nobody talking. On the phones the group saturated
//! and split ("Perdido · oído hace 111 s", "Grupo dividido en 2 partes"): Android
//! scanned `LOW_LATENCY` whenever discovery was `Forming`, which starved the
//! connection events of its links into supervision timeouts, and every loss kept all
//! three phones `Forming`. With the scan cost charged by the radio model
//! ([`mesh_sim::scan`]) the triangle must stay one component with no link drop for 10
//! minutes; the control run, whose drivers ignore the intensity and scan continuously,
//! must lose links (the model does charge the field failure).

use mesh_api::TalkMode;
use mesh_core::{DiscoveryMode, ScanIntensity};
use mesh_sim::scenario::common::{all_reachable, form_ride, go_live_all, group_config};
use mesh_sim::{Mobility, NodeSpec, World, WorldConfig};

const IDLE_S: u64 = 600;

fn table(seed: u64, force_scan: Option<ScanIntensity>) -> World {
    let mut tab = NodeSpec::android();
    tab.model = "SM-X210".into();
    let mut galaxy = NodeSpec::android();
    galaxy.model = "SM-S921B".into();
    let mut iphone = NodeSpec::iphone();
    iphone.model = "iPhone15,5".into();
    // A triangle of about one metre at a table.
    let pos = vec![(0.0, 0.0), (1.0, 0.0), (0.5, 0.8)];
    let mut cfg = group_config(WorldConfig::new(
        seed,
        vec![tab, galaxy, iphone],
        Mobility::Static(pos),
    ));
    cfg.radio.scan.force_scan = force_scan;
    World::new(cfg)
}

/// Forms the ride with the tablet as leader and, when `live`, goes live in VOX with
/// nobody talking.
fn formed(seed: u64, force_scan: Option<ScanIntensity>, live: bool) -> World {
    let mut w = table(seed, force_scan);
    let (g, t) = form_ride(&mut w, 0);
    assert!(t.is_some(), "seed {seed}: the table never formed");
    if live {
        assert!(go_live_all(&mut w, &g, TalkMode::Vox, false), "go live");
    }
    w
}

#[test]
fn three_phones_idle_at_a_table_stay_one_group_for_ten_minutes() {
    // One seed under `cargo test` (10 simulated minutes of three live engines).
    for seed in [1u64] {
        let mut w = formed(seed, None, true);
        // Let the formation settle (the last link of the triangle, first PRESENCE).
        w.run_s(10);
        let dropped = w.stats.links_dropped;
        let bytes0: u64 = w
            .links
            .iter()
            .flat_map(|l| l.dirs.iter())
            .map(|d| d.bytes)
            .sum();
        let mut forming = [0u64; 3];
        let mut low_latency = [0u64; 3];
        for s in 0..IDLE_S {
            w.run_s(1);
            assert_eq!(w.components().len(), 1, "seed {seed}: split at +{s} s");
            assert!(
                all_reachable(&w, &[0, 1, 2]),
                "seed {seed}: a member lost at +{s} s"
            );
            for (i, n) in w.nodes.iter().enumerate() {
                forming[i] += u64::from(n.discovery.0 == DiscoveryMode::Forming);
                low_latency[i] += u64::from(n.scan == ScanIntensity::LowLatency);
            }
        }
        assert_eq!(
            w.stats.links_dropped, dropped,
            "seed {seed}: a link dropped while idle"
        );
        assert_eq!(w.stats.starved_drops, 0, "seed {seed}");
        // Every member reachable: Stable at LOW_POWER, never hunting.
        assert_eq!(forming, [0; 3], "seed {seed}: Forming seconds per node");
        assert_eq!(low_latency, [0; 3], "seed {seed}: LOW_LATENCY seconds");
        let ble: usize = (0..3).map(|i| w.nodes[i].active_links(true)).sum();
        assert_eq!(ble, 6, "seed {seed}: the triangle holds its three links");
        // Idle control traffic stays small: ≤ 1.5 kbit/s per link direction.
        let bytes: u64 = w
            .links
            .iter()
            .flat_map(|l| l.dirs.iter())
            .map(|d| d.bytes)
            .sum();
        let per_dir_bps = (bytes - bytes0) as f64 * 8.0 / IDLE_S as f64 / 6.0;
        assert!(per_dir_bps < 1_500.0, "seed {seed}: {per_dir_bps:.0} bit/s");
        assert!(w.audit.violations.is_empty(), "{:?}", w.audit.violations);
    }
}

#[test]
fn continuous_scanning_drivers_lose_links_at_the_same_table() {
    // The scan cost does not depend on audio: the lobby is enough.
    let mut w = formed(1, Some(ScanIntensity::LowLatency), false);
    w.run_s(300);
    assert!(
        w.stats.starved_drops > 0,
        "the cost model must charge continuous scanning"
    );
}

#[test]
fn perfect_links_keep_their_quality_and_relays_prune_presence() {
    // The PING loss accounting read a PONG that crossed the 1 s loss period as a lost
    // PING; with the 2 s PING period phase-locked to it, `q` of a perfect link decayed
    // to 0, its NBR listed the peer as not good, and the other neighbors relayed every
    // PRESENCE to it again (two-hop pruning miss, §5.4). Seed 3 is such a phase.
    let pos = vec![(0.0, 0.0), (3.0, 0.0), (1.5, 2.5)];
    let nodes = vec![NodeSpec::android(), NodeSpec::android(), NodeSpec::iphone()];
    let mut w = World::new(group_config(WorldConfig::new(
        3,
        nodes,
        Mobility::Static(pos),
    )));
    let (_, t) = form_ride(&mut w, 0);
    assert!(t.is_some());
    w.run_s(10);
    let counts = |w: &World| -> Vec<(u64, usize)> {
        w.nodes
            .iter()
            .map(|n| {
                let t = n.tap.borrow();
                (t.presence_copies, t.presence_keys.len())
            })
            .collect()
    };
    let before = counts(&w);
    w.run_s(60);
    let after = counts(&w);
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        let (copies, keys) = (a.0 - b.0, (a.1 - b.1) as u64);
        assert!(
            copies <= keys + 2,
            "node {i}: {copies} PRESENCE copies of {keys}"
        );
    }
    // The latest NBR view on every link lists its neighbors as good.
    for (i, n) in w.nodes.iter().enumerate() {
        let t = n.tap.borrow();
        let mut last = std::collections::BTreeMap::new();
        for (at, link, entries) in &t.nbr {
            last.insert(*link, (*at, entries.clone()));
        }
        assert_eq!(last.len(), 2, "node {i}: two neighbors");
        for (link, (at, entries)) in last {
            assert!(
                entries.iter().all(|e| e.1 >= 60),
                "node {i} link {link} at {at}: {entries:?}"
            );
        }
    }
}
