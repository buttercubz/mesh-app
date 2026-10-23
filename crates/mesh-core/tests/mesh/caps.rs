//! Per-link caps under contention and at steady state (DESIGN §5.6, §6.3).

use mesh_wire::mesh::PacketType;

use crate::talk::{Talker, run_talking};
use crate::world::World;
use crate::{max_concurrent_bursts, nbrs, of_type, voice_on};

/// Hub 0 with leaves 1, 2, 3; tails 4 and 7 hang off leaf 1, 5 off leaf 2, 6 off leaf 3.
fn hub_with_tails(salt: u8) -> World {
    let mut w = World::new(8, salt);
    w.start_all();
    for leaf in 1..=3 {
        w.connect(0, leaf);
    }
    for (leaf, tail) in [(1, 4), (2, 5), (3, 6), (1, 7)] {
        w.connect(leaf, tail);
    }
    w.run_ms(3_000);
    w
}

#[test]
fn no_link_carries_more_than_its_cap_under_contention() {
    let mut w = hub_with_tails(11);
    // Four tail talkers start 50-150 ms apart: hub → leaf 2 and hub → leaf 3 have
    // three candidate bursts each, leaf 1 → tails has three as well.
    let mut active: Vec<Talker> = Vec::new();
    for (k, node) in [4, 5, 6, 7].into_iter().enumerate() {
        active.push(Talker::new(node, 1));
        run_talking(&mut w, &mut active, if k == 1 { 50 } else { 150 });
    }
    run_talking(&mut w, &mut active, 3_000);
    for i in 0..w.nodes.len() {
        let links: Vec<_> = w.nodes[i].engine.ctx().links.iter().map(|l| l.id).collect();
        for l in links {
            let n = max_concurrent_bursts(&w, i, l);
            assert!(n <= 2, "node {i} link {l:?} carried {n} bursts at once");
        }
    }
    // Bursts over the cap were refused with FLOOR_REJECT (near-talker rejection by the
    // leaves and relay admission); the scripted talkers ignore it and keep sending, yet
    // relays never forward a refused burst (hard enforcement).
    let rejects: usize = (0..4)
        .map(|i| {
            let me = w.nodes[i].node_id;
            w.sent(i)
                .iter()
                .filter(|(_, _, h, _)| h.ptype == PacketType::FloorReject && h.origin == me)
                .count()
        })
        .sum();
    assert!(rejects > 0, "a burst over the cap was refused");
    assert!(active.iter().all(|t| !t.stopped && t.frames > 70));
    // The floor converged: in the last second the hub forwarded one or two bursts.
    let since = w.now - 1_000_000;
    let mut live: Vec<(u16, u8)> = w
        .sent(0)
        .iter()
        .filter(|(t, _, h, _)| *t >= since && h.ptype == PacketType::Voice)
        .map(|(_, _, h, b)| (h.origin, b[mesh_wire::mesh::MeshHeader::LEN]))
        .collect();
    live.sort_unstable();
    live.dedup();
    assert!(
        (1..=2).contains(&live.len()),
        "live bursts at the hub: {live:?}"
    );
}

#[test]
fn cap_two_holds_at_steady_state_on_a_three_link_hub() {
    let mut w = World::new(4, 12);
    w.start_all();
    for leaf in 1..=3 {
        w.connect(0, leaf);
    }
    w.run_ms(3_000);
    let start = w.now;
    let mut t = vec![Talker::new(1, 5), Talker::new(2, 6)];
    run_talking(&mut w, &mut t, 20_000);
    // Every NBR of the hub advertised vcap 2 on all three links the whole time.
    let reports: Vec<_> = nbrs(&w, 0)
        .into_iter()
        .filter(|(t, _, _)| *t >= start)
        .collect();
    assert!(reports.len() > 50, "NBR heartbeat at ≥ 1 Hz");
    for (t, _, n) in &reports {
        assert_eq!(n.entries().len(), 3);
        for e in n.entries() {
            assert_eq!(e.vcap, 2, "vcap at {t} µs: {:?}", n.entries());
        }
    }
    // Three voice links share the 180 kbps budget: each drains at 60 kbps.
    for l in w.nodes[0].engine.ctx().links.iter() {
        assert!(
            l.pace.rate_bps.is_some_and(|r| r >= 55_800),
            "{:?} paced at {:?}",
            l.id,
            l.pace.rate_bps
        );
    }
    // No refusal, no PATH_HINT, and leaf 3 hears both talkers frame for frame.
    let l3 = w.local(2, 0);
    assert!(of_type(&w, 0, l3, PacketType::FloorReject).is_empty());
    assert!(of_type(&w, 0, l3, PacketType::PathHint).is_empty());
    for (k, talker) in t.iter().enumerate() {
        let got = voice_on(&w, 0, l3, w.nodes[k + 1].node_id).len() as u32;
        assert!(
            got * 100 >= talker.frames * 99,
            "talker {k}: {got} of {}",
            talker.frames
        );
    }
}

#[test]
fn a_starved_relay_floods_path_hint_ble12() {
    use mesh_core::Platform;
    use mesh_wire::mesh::{EncoderProfile, MeshPacket, MeshPayload};
    // The relay's radio budget is 40 kbps: two voice links get 20 kbps each, below
    // cost(1) = 26.6 kbps for BLE16.
    let mut w = World::configured(3, 13, Platform::Ios, false, |i, cfg| {
        if i == 1 {
            cfg.limits.routing.node_ble_budget_bps = 40_000;
        }
    });
    w.start_all();
    let l01 = w.connect(0, 1);
    let l12 = w.connect(1, 2);
    w.run_ms(2_000);
    let start = w.now;
    let mut t = vec![Talker::new(0, 1), Talker::new(2, 1)];
    run_talking(&mut w, &mut t, 5_000);
    let me = w.nodes[1].node_id;
    for l in [w.local(l01, 1), w.local(l12, 1)] {
        let hints: Vec<_> = w
            .sent(1)
            .into_iter()
            .filter(|(at, link, h, _)| {
                *link == l && *at >= start && h.ptype == PacketType::PathHint
            })
            .collect();
        assert!(!hints.is_empty(), "PATH_HINT on {l:?}");
        let (at, _, h, body) = &hints[0];
        assert_eq!((h.origin, h.ttl), (me, 3));
        assert!(*at - start >= 3_000_000, "only after 3 s below cost(1)");
        let p = MeshPacket::decode(body).expect("packet");
        match MeshPayload::decode(&p).expect("payload") {
            MeshPayload::PathHint(x) => assert_eq!(x.profile, EncoderProfile::Ble12),
            other => panic!("{other:?}"),
        }
    }
    // The published cap fell to what the grant allows; the NBR says so.
    let last = nbrs(&w, 1).last().map(|(_, _, n)| *n).expect("nbr");
    assert!(
        last.entries().iter().all(|e| e.vcap < 2),
        "{:?}",
        last.entries()
    );
}
