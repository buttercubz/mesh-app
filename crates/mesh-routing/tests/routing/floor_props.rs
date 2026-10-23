//! Floor property tests (DESIGN §6, WP-09 acceptance): random floor contention with
//! per-link caps (every node whose path admits a burst receives it; the top-ranked burst
//! is never refused), safety under misbehaving talkers, convergence of the rank order,
//! liveness of a blocked talker, a mid-burst join and leader preemption.

use std::collections::{BTreeMap, BTreeSet};

use mesh_routing::floor::smear_us;
use mesh_routing::{BurstKey, Instant, Limits, NodeId, RouterAction, floor_hash};
use mesh_wire::mesh::{FloorRejectReason, PacketType, Voice};
use proptest::prelude::*;

use crate::net::{Net, lid, random_geometric};
use crate::talker::{Phase, Talker, run};

const EPOCH: u32 = 7;

fn voice_receivers(net: &Net, b: BurstKey) -> BTreeSet<NodeId> {
    net.rx
        .iter()
        .filter(|r| {
            r.header.ptype == PacketType::Voice
                && r.header.origin == b.origin
                && r.header.sess == b.sess
        })
        .filter(|r| Voice::decode(&r.payload).is_ok_and(|v| v.burst == b.burst))
        .map(|r| r.node)
        .collect()
}

/// eff_start of a burst started at `start` at its origin (age 0).
fn eff(b: BurstKey, start: Instant) -> i64 {
    start.us() as i64 + smear_us(floor_hash(EPOCH, b), Limits::V1.t_tie_us)
}

fn set_caps(net: &mut Net, edges: &[(NodeId, NodeId)], caps: &[u8]) {
    for (i, &(u, v)) in edges.iter().enumerate() {
        let c = caps[i % caps.len()];
        net.routers
            .get_mut(&u)
            .expect("u")
            .set_link_cap(lid(u, v), c);
        net.routers
            .get_mut(&v)
            .expect("v")
            .set_link_cap(lid(v, u), c);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn random_contention_with_per_link_caps(
        seed in 1u64..u64::MAX,
        n in 5u16..12,
        caps in proptest::collection::vec(1u8..=2, 1..8),
        starts in proptest::collection::vec((0u16..1000, 0u64..60), 2..5),
    ) {
        let (_, edges) = random_geometric(n, 0.55, 3, seed);
        let mut net = Net::new(n, 15_000, Limits::V1, seed);
        for &(u, v) in &edges {
            net.connect(u, v);
        }
        set_caps(&mut net, &edges, &caps);
        let t0 = net.now;
        let mut talkers: Vec<Talker> = Vec::new();
        for &(node, s) in &starts {
            let node = 1 + node % n;
            if talkers.iter().any(|t| t.node == node) {
                continue;
            }
            let start = t0.plus_ms(s * 10);
            talkers.push(Talker::new(node, start, start.plus_ms(2_000)));
        }
        run(&mut net, &mut talkers, t0.plus_ms(1_500));
        // Convergence after ≥ 1 TALK KEEP: nodes order commonly known live bursts alike
        // (pairs whose effective starts are within 10 ms of each other are excused: age
        // estimation error, resolved by FLOOR_REJECT).
        let mut start_of: BTreeMap<BurstKey, Instant> = BTreeMap::new();
        for t in &talkers {
            if let Phase::Talking { started, .. } = t.phase {
                start_of.insert(t.key(), started);
            }
        }
        for (a, ra) in &net.routers {
            for (b, rb) in &net.routers {
                if a >= b { continue; }
                let la: Vec<BurstKey> = ra.floor.top(8);
                let lb: Vec<BurstKey> = rb.floor.top(8);
                let common: Vec<BurstKey> = la.iter().copied().filter(|k| lb.contains(k)).collect();
                let ob: Vec<BurstKey> = lb.iter().copied().filter(|k| common.contains(k)).collect();
                for (x, y) in common.iter().zip(ob.iter()) {
                    if x != y {
                        let close = match (start_of.get(x), start_of.get(y)) {
                            (Some(sx), Some(sy)) => (eff(*x, *sx) - eff(*y, *sy)).abs() < 10_000,
                            _ => true,
                        };
                        prop_assert!(close, "nodes {} and {} disagree: {:?} vs {:?}", a, b, common, ob);
                    }
                }
            }
        }
        run(&mut net, &mut talkers, t0.plus_ms(3_500));
        prop_assert!(net.violations.is_empty(), "{:?}", net.violations);
        // The globally top-ranked burst is never refused.
        let started: Vec<(BurstKey, Instant)> = talkers
            .iter()
            .flat_map(|t| t.bursts_started.iter().map(move |b| (*b, t.start)))
            .collect();
        let best = started.iter().min_by_key(|(b, s)| (eff(*b, *s), b.origin)).map(|x| x.0);
        let rejected: BTreeSet<BurstKey> = talkers.iter().flat_map(|t| t.rejected.iter().copied()).collect();
        if let Some(best) = best {
            prop_assert!(!rejected.contains(&best), "top burst {:?} was rejected", best);
        }
        // Every node whose path admits a surviving burst receives it: survivors reach
        // every node within the voice TTL (4 hops).
        for (b, _) in &started {
            if rejected.contains(b) { continue; }
            let expect: BTreeSet<NodeId> = net.distances(b.origin).into_iter()
                .filter(|&(v, d)| v != b.origin && d <= 4).map(|(v, _)| v).collect();
            let got = voice_receivers(&net, *b);
            prop_assert!(expect.is_subset(&got), "burst {:?}: expected {:?}, got {:?}", b, expect, got);
        }
    }

    #[test]
    fn caps_hold_even_if_talkers_ignore_rejects(seed in 1u64..u64::MAX, n in 4u16..10) {
        let (_, edges) = random_geometric(n, 0.6, 3, seed);
        let mut net = Net::new(n, 15_000, Limits::V1, seed);
        for &(u, v) in &edges {
            net.connect(u, v);
        }
        set_caps(&mut net, &edges, &[1, 2, 1]);
        let t0 = net.now;
        let mut talkers: Vec<Talker> = (1..=n.min(5))
            .map(|i| {
                let mut t = Talker::new(i, t0.plus_ms(u64::from(i) * 30), t0.plus_ms(2_000));
                // Misbehaving origins: start regardless of local_admit, ignore rejects.
                t.ignore_rejects = true;
                t.force = true;
                t
            })
            .collect();
        run(&mut net, &mut talkers, t0.plus_ms(2_500));
        prop_assert!(net.violations.is_empty(), "{:?}", net.violations);
        // Per link, bursts streaming *across* any instant (a frame within 80 ms before
        // and one within 80 ms after) never exceed the cap; churn (a burst rejected and
        // replaced) does not count as concurrency.
        let mut by_link: BTreeMap<(NodeId, u32), Vec<(u64, BurstKey)>> = BTreeMap::new();
        for v in &net.voice_tx {
            by_link.entry((v.node, v.link.0)).or_default().push((v.at.us(), v.burst));
        }
        for ((node, link), txs) in by_link {
            let cap = net.routers[&node].link_cap(mesh_routing::LinkId(link));
            for &(t, _) in &txs {
                let bursts: BTreeSet<BurstKey> = txs.iter().map(|x| x.1).collect();
                let across = bursts.iter().filter(|b| {
                    let before = txs.iter().any(|&(u, k)| k == **b && u <= t && t - u <= 80_000);
                    let after = txs.iter().any(|&(u, k)| k == **b && u >= t && u - t <= 80_000);
                    before && after
                }).count();
                prop_assert!(across <= usize::from(cap), "link {} carried {} streams at {}", link, across, t);
            }
        }
    }
}

fn chain(n: u16) -> Net {
    let mut net = Net::new(n, 15_000, Limits::V1, 99);
    for u in 1..n {
        net.connect(u, u + 1);
    }
    net
}

#[test]
fn blocked_talker_gets_the_floor_when_a_slot_frees() {
    let mut net = chain(4);
    let t0 = net.now;
    let mut talkers = vec![
        Talker::new(1, t0, t0.plus_ms(1_500)),
        Talker::new(4, t0.plus_ms(100), t0.plus_ms(1_500)),
        Talker::new(2, t0.plus_ms(300), t0.plus_ms(4_000)),
    ];
    talkers[2].retry = true;
    run(&mut net, &mut talkers, t0.plus_ms(1_000));
    assert_eq!(
        talkers[2].phase,
        Phase::Blocked,
        "two bursts fill cap_local = 2"
    );
    run(&mut net, &mut talkers, t0.plus_ms(3_000));
    assert!(
        matches!(talkers[2].phase, Phase::Talking { .. }),
        "{:?}",
        talkers[2].phase
    );
    let b = *talkers[2].bursts_started.last().expect("burst");
    assert_eq!(voice_receivers(&net, b), BTreeSet::from([1, 3, 4]));
    assert!(net.violations.is_empty());
    // A lone talker in an idle network is never refused.
    assert!(talkers[0].rejected.is_empty());
}

#[test]
fn mid_burst_join_agrees_within_one_keep() {
    // 1-2 and 4-2 talk; node 3 links to 2 two seconds into the bursts (missed START).
    let mut net = Net::new(4, 15_000, Limits::V1, 5);
    net.connect(1, 2);
    net.connect(4, 2);
    let t0 = net.now;
    let mut talkers = vec![
        Talker::new(1, t0, t0.plus_ms(8_000)),
        Talker::new(4, t0.plus_ms(50), t0.plus_ms(8_000)),
    ];
    run(&mut net, &mut talkers, t0.plus_ms(2_000));
    net.connect(2, 3);
    let joined = net.now;
    run(&mut net, &mut talkers, joined.plus_ms(1_100));
    let r2 = &net.routers[&2];
    let r3 = &net.routers[&3];
    assert_eq!(r3.floor.top(4), r2.floor.top(4));
    assert_eq!(r3.floor.top(4).len(), 2);
    for k in r2.floor.top(4) {
        let a = r2.floor.get(k).expect("2").t_origin_us;
        let b = r3.floor.get(k).expect("3").t_origin_us;
        // Same local clock in the sim: the burst-relative ts puts a 2 s-old burst at the
        // same origin time, up to the 2 ms age quantization per hop.
        assert!((a - b).abs() <= 4_000, "{k:?}: {a} vs {b}");
    }
}

#[test]
fn leader_preempts_lowest_ranked_member() {
    // Leaves 1, 5, 6 hang off hub 2, which links on to 3: link 2→3 carries 1's and 5's
    // bursts (cap 2) when leader 6 starts.
    let mut net = Net::new(6, 15_000, Limits::V1, 3);
    for (u, v) in [(1, 2), (5, 2), (6, 2), (2, 3)] {
        net.connect(u, v);
    }
    let t0 = net.now;
    let mut talkers = vec![
        Talker::new(1, t0, t0.plus_ms(3_000)),
        // 400 ms apart (> T_TIE): node 5's burst ranks below node 1's whatever the smear.
        Talker::new(5, t0.plus_ms(400), t0.plus_ms(3_000)),
        Talker::new(6, t0.plus_ms(1_000), t0.plus_ms(3_000)),
    ];
    talkers[2].leader = true;
    run(&mut net, &mut talkers, t0.plus_ms(2_000));
    // The leader is admitted; the latest member burst (node 5) was preempted.
    assert!(matches!(talkers[2].phase, Phase::Talking { .. }));
    assert_eq!(talkers[1].phase, Phase::Blocked);
    assert!(voice_receivers(&net, *talkers[2].bursts_started.last().expect("b")).contains(&3));
    assert!(net.actions.iter().any(|(_, n, a)| *n == 5
        && matches!(
            a,
            RouterAction::RejectedMe {
                reason: FloorRejectReason::Preempted,
                ..
            }
        )));
    assert!(talkers[0].rejected.is_empty());
    assert!(net.violations.is_empty());
}
