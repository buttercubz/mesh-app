//! Multi-hop relay of voice with the age cap (DESIGN §5.2, §5.5).

use mesh_wire::mesh::PacketType;

use crate::talk::{Talker, run_talking};
use crate::world::World;
use crate::{of_type, voice_on};

/// A chain 0 – 1 – 2 – 3 with `delays[k]` on link k.
fn chain(salt: u8, delays: [u64; 3]) -> World {
    let mut w = World::new(4, salt);
    w.start_all();
    for (k, d) in delays.into_iter().enumerate() {
        w.delay_us = d;
        w.connect(k, k + 1);
    }
    // Links up, NBRs exchanged, PING RTTs measured.
    w.run_ms(3_000);
    w
}

#[test]
fn chain_of_four_relays_voice_hop_by_hop() {
    let mut w = chain(1, [10_000; 3]);
    let origin = w.nodes[0].node_id;
    let mut t = vec![Talker::new(0, 7)];
    run_talking(&mut w, &mut t, 2_000);
    let sent = t[0].frames as usize;
    assert!(sent >= 49, "talker sent {sent} frames");
    // Node 1 relays to 2, node 2 relays to 3; nobody relays back toward the origin.
    let l12 = w.local(1, 1);
    let l23 = w.local(2, 2);
    let at1 = voice_on(&w, 1, l12, origin);
    let at2 = voice_on(&w, 2, l23, origin);
    assert!(
        at1.len() + 2 >= sent,
        "hop 1 relayed {} of {sent}",
        at1.len()
    );
    assert!(
        at2.len() + 2 >= sent,
        "hop 2 relayed {} of {sent}",
        at2.len()
    );
    assert!(
        voice_on(&w, 1, w.local(0, 1), origin).is_empty(),
        "no echo to the origin"
    );
    assert!(
        voice_on(&w, 3, w.local(2, 3), origin).is_empty(),
        "the end of the chain is a leaf"
    );
    // Relay rewrite: ttl −1 and hops +1 per traversal; age grows with each hop.
    assert!(at1.iter().all(|(_, h)| h.ttl == 3 && h.hops == 1));
    assert!(at2.iter().all(|(_, h)| h.ttl == 2 && h.hops == 2));
    let age = |v: &[(u64, mesh_wire::mesh::MeshHeader)]| {
        v.iter().map(|(_, h)| h.age_us()).sum::<u64>() / v.len() as u64
    };
    let (a1, a2) = (age(&at1), age(&at2));
    assert!((10_000..40_000).contains(&a1), "age after one hop {a1} µs");
    assert!(
        a2 >= a1 + 10_000 && a2 < 80_000,
        "age after two hops {a2} µs"
    );
    // TALK is relayed too (ctrl class, jittered 5-25 ms).
    assert!(!of_type(&w, 2, l23, PacketType::Talk).is_empty());
}

#[test]
fn relays_drop_voice_older_than_the_age_cap() {
    // 10 ms, then 150 ms: at node 2 the frame is ~160 ms old (> VOICE_MAX_AGE).
    let mut w = chain(2, [10_000, 150_000, 10_000]);
    let origin = w.nodes[0].node_id;
    let mut t = vec![Talker::new(0, 3)];
    run_talking(&mut w, &mut t, 2_000);
    let l12 = w.local(1, 1);
    let l23 = w.local(2, 2);
    assert!(
        voice_on(&w, 1, l12, origin).len() >= 45,
        "fresh at hop 1: relayed"
    );
    let stale = voice_on(&w, 2, l23, origin);
    assert!(
        stale.is_empty(),
        "{} stale frames relayed past the age cap",
        stale.len()
    );
    // Control is not age-capped: TALK still reaches node 3.
    assert!(!of_type(&w, 2, l23, PacketType::Talk).is_empty());
}
