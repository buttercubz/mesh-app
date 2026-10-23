//! One sequence per `(origin, sess)` (DESIGN §4.5, §5.3): every packet a node
//! originates — TALK/VOICE, control and bulk, and the FLOOR_REJECTs the relay router
//! builds itself — takes the next value of one shared counter, so a relay's single
//! 1024-seq dedup window per origin relays all classes interleaved, across the
//! 65535 → 0 wrap, with no "too old" drop.

use mesh_core::media::MediaRequest;
use mesh_wire::mesh::{FloorRejectReason, MeshPacket, MeshPayload, PacketType};

use crate::talk::{Talker, run_talking};
use crate::world::World;

/// `n` consecutive seqs from `first` (serial-number arithmetic).
fn run_of(first: u16, n: usize) -> Vec<u16> {
    (0..n).map(|k| first.wrapping_add(k as u16)).collect()
}

fn sorted_seqs(v: &[(PacketType, u16)], first: u16) -> Vec<u16> {
    let mut s: Vec<u16> = v.iter().map(|(_, q)| *q).collect();
    // Order by distance from `first`, so a run across the wrap reads as consecutive.
    s.sort_by_key(|q| q.wrapping_sub(first));
    s
}

fn types(v: &[(PacketType, u16)]) -> Vec<PacketType> {
    let mut t: Vec<PacketType> = v.iter().map(|(p, _)| *p).collect();
    t.sort_by_key(|p| format!("{p:?}"));
    t.dedup();
    t
}

#[test]
fn every_class_from_one_origin_is_relayed_through_one_window_across_the_wrap() {
    // Chain 0 – 1 – 2; node 0's counter starts 48 below the wrap.
    let first = 0xFFD0;
    let mut w = World::new(3, 61);
    w.nodes[0].shared.borrow_mut().first_seq = Some(first);
    w.start_all();
    let l01 = w.connect(0, 1);
    let l12 = w.connect(1, 2);
    w.run_ms(2_000);
    // TALK (P0), VOICE (P1) and POSITION (P2) interleaved for 2 s: the link queues
    // reorder them by priority, and the counter wraps half way.
    let mut t = vec![Talker::new(0, 3)];
    for _ in 0..40 {
        w.position(0);
        run_talking(&mut w, &mut t, 50);
    }
    t[0].stop(&mut w);
    w.run_ms(500);
    let sent = w.own_on(0, w.local(l01, 0));
    let n = sent.len();
    assert!(n > 90, "node 0 originated {n} packets");
    assert_eq!(
        sorted_seqs(&sent, first),
        run_of(first, n),
        "one counter for every class, consecutive across the wrap"
    );
    assert!(sent.iter().any(|(_, q)| *q == 0xFFFF) && sent.iter().any(|(_, q)| *q == 0));
    assert_eq!(
        types(&sent),
        types(&[
            (PacketType::Position, 0),
            (PacketType::Talk, 0),
            (PacketType::Voice, 0)
        ])
    );
    // The relay deduplicates node 0 in a single §5.3 window and still relays every
    // packet of every class: nothing was dropped as a duplicate or as too old.
    let me = w.nodes[0].node_id;
    let mut relayed: Vec<(PacketType, u16)> = w
        .sent(1)
        .into_iter()
        .filter(|(_, l, h, _)| *l == w.local(l12, 1) && h.origin == me)
        .map(|(_, _, h, _)| (h.ptype, h.seq))
        .collect();
    let mut want = sent.clone();
    relayed.sort_by_key(|(_, q)| q.wrapping_sub(first));
    want.sort_by_key(|(_, q)| q.wrapping_sub(first));
    assert_eq!(relayed, want, "node 1 relayed everything node 0 originated");
}

/// Node `i`'s own FLOOR_REJECT reasons on `link`.
fn reject_reasons(w: &World, i: usize, link: mesh_core::LinkId) -> Vec<FloorRejectReason> {
    let me = w.nodes[i].node_id;
    w.sent(i)
        .into_iter()
        .filter(|(_, l, h, _)| *l == link && h.origin == me && h.ptype == PacketType::FloorReject)
        .filter_map(|(_, _, _, b)| {
            let p = MeshPacket::decode(&b).ok()?;
            match MeshPayload::decode(&p).ok()? {
                MeshPayload::FloorReject(r) => Some(r.reason),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn media_rejects_and_module_packets_share_the_relay_counter() {
    // Relay 1 links talkers 0, 2 and 3 and listener 4: the relay's media module sends
    // a near-talker FLOOR_REJECT to the third talker while the relay also originates
    // POSITIONs.
    let first = 0xFFF0;
    let mut w = World::new(5, 62);
    w.nodes[1].shared.borrow_mut().first_seq = Some(first);
    w.start_all();
    for n in [0, 2, 3, 4] {
        w.connect(n, 1);
    }
    w.run_ms(2_500);
    let mut talkers: Vec<Talker> = Vec::new();
    for node in [0, 2, 3] {
        talkers.push(Talker::new(node, 1));
        w.position(1);
        run_talking(&mut w, &mut talkers, 100);
    }
    for _ in 0..10 {
        w.position(1);
        run_talking(&mut w, &mut talkers, 100);
    }
    w.run_ms(300);
    // World link 3 is 4 – 1.
    let to4 = w.local(3, 1);
    let own = w.own_on(1, to4);
    assert!(
        reject_reasons(&w, 1, to4).contains(&FloorRejectReason::Outranked),
        "the media module's near-talker reject"
    );
    assert!(own.iter().any(|(p, _)| *p == PacketType::Position));
    assert_eq!(
        sorted_seqs(&own, first),
        run_of(first, own.len()),
        "media-originated and other packets share one counter"
    );
    // The relay originated no voice: it is not talking.
    assert!(!own.iter().any(|(p, _)| *p == PacketType::Voice));
}

#[test]
fn packets_the_relay_router_builds_take_the_same_counter() {
    // The relay's radio budget is 40 kbps: its two voice links get 20 kbps each, below
    // cost(1), so after 3 s the router floods PATH_HINT{BLE12} and refuses bursts with
    // FLOOR_REJECT{congestion}: packets the router builds itself, interleaved with the
    // POSITIONs the relay originates.
    let first = 0xFFF8;
    let mut w = World::configured(3, 65, mesh_core::Platform::Ios, false, |i, cfg| {
        if i == 1 {
            cfg.limits.routing.node_ble_budget_bps = 40_000;
        }
    });
    w.nodes[1].shared.borrow_mut().first_seq = Some(first);
    w.start_all();
    w.connect(0, 1);
    let l12 = w.connect(1, 2);
    w.run_ms(2_000);
    let mut t = vec![Talker::new(0, 1), Talker::new(2, 1)];
    for _ in 0..20 {
        w.position(1);
        run_talking(&mut w, &mut t, 250);
    }
    let to2 = w.local(l12, 1);
    let own = w.own_on(1, to2);
    assert!(
        own.iter().any(|(p, _)| *p == PacketType::PathHint),
        "router-built PATH_HINT: {own:?}"
    );
    assert!(own.iter().any(|(p, _)| *p == PacketType::Position));
    assert_eq!(
        sorted_seqs(&own, first),
        run_of(first, own.len()),
        "router-built and module-originated packets share one counter"
    );
}

#[test]
fn without_a_group_identity_originated_packets_go_on_every_link_directly() {
    // Node 0 never learns its identity: its mesh glue stays inactive, and the engine
    // queues what it originates on every Up link itself.
    let mut w = World::new(3, 63);
    w.nodes[0].shared.borrow_mut().no_identity = true;
    w.nodes[0].shared.borrow_mut().first_seq = Some(500);
    w.start_all();
    let a = w.connect(0, 1);
    let b = w.connect(0, 2);
    w.run_ms(500);
    w.position(0);
    w.run_ms(100);
    for l in [a, b] {
        assert_eq!(
            w.own_on(0, w.local(l, 0)),
            [(PacketType::Position, 500)],
            "on {l:?}"
        );
    }
    assert!(
        !w.bus(0)
            .iter()
            .any(|r| matches!(r, MediaRequest::Identity(_))),
        "no identity: nothing told the media module"
    );
}
