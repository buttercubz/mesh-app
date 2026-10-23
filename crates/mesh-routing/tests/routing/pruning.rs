//! Pruning equivalence for ctrl/bulk on random geometric graphs (DESIGN §5.4): pruned
//! flooding delivers to exactly the nodes plain flooding reaches within the TTL, with no
//! more transmissions. Voice is forwarded on every admitting link regardless of NBR
//! state.

use std::collections::BTreeSet;

use mesh_routing::{Limits, NodeId};
use mesh_wire::mesh::{PacketType, Position, Presence, Voice, VoiceCfg};
use proptest::prelude::*;

use crate::net::{Net, random_geometric};

fn build(n: u16, edges: &[(NodeId, NodeId)], seed: u64) -> Net {
    let mut net = Net::new(n, 15_000, Limits::V1, seed);
    for &(u, v) in edges {
        net.connect(u, v);
    }
    net
}

/// Floods one packet of `ptype` from `origin`; returns the receivers and transmissions.
fn flood(
    net: &mut Net,
    origin: NodeId,
    ptype: PacketType,
    payload: Vec<u8>,
) -> (BTreeSet<NodeId>, u64) {
    let before_tx = net.transmissions;
    let start = net.now;
    net.originate(origin, ptype, false, payload);
    net.run_until(start.plus_ms(1_000));
    let got: BTreeSet<NodeId> = net
        .rx
        .iter()
        .filter(|r| r.at >= start && r.header.origin == origin && r.header.ptype == ptype)
        .map(|r| r.node)
        .collect();
    (got, net.transmissions - before_tx)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn ctrl_and_bulk_pruning_equivalence(seed in 1u64..u64::MAX, n in 6u16..20, radius in 0.25f64..0.6) {
        let (_, edges) = random_geometric(n, radius, 3, seed);
        let origin: NodeId = 1 + (seed % u64::from(n)) as NodeId;
        for (ptype, payload) in [
            (PacketType::Presence, Presence::default().encode()),
            (PacketType::Position, Position { lat_e7: 1, lon_e7: 2, speed_dms: 0, heading_2deg: 0, acc_m: 1, fix_age_100ms: 0, rsv: 0 }.encode()),
        ] {
            // Plain flooding: no NBR reports, so nothing is ever pruned.
            let mut plain = build(n, &edges, seed);
            let (got_plain, tx_plain) = flood(&mut plain, origin, ptype, payload.clone());
            // Pruned flooding: fresh NBRs everywhere.
            let mut pruned = build(n, &edges, seed);
            pruned.exchange_nbr();
            let (got_pruned, tx_pruned) = flood(&mut pruned, origin, ptype, payload);
            // Plain flooding with TTL 15 reaches the origin's component (minus itself).
            let reach: BTreeSet<NodeId> = plain
                .distances(origin)
                .into_iter()
                .filter(|&(v, d)| v != origin && d <= 15)
                .map(|(v, _)| v)
                .collect();
            prop_assert_eq!(&got_plain, &reach);
            prop_assert_eq!(&got_pruned, &got_plain);
            prop_assert!(tx_pruned <= tx_plain, "pruned {} > plain {}", tx_pruned, tx_plain);
            prop_assert!(pruned.violations.is_empty());
        }
    }

    #[test]
    fn voice_ignores_nbr_state(seed in 1u64..u64::MAX, n in 4u16..14) {
        let (_, edges) = random_geometric(n, 0.5, 3, seed);
        let origin: NodeId = 1 + (seed % u64::from(n)) as NodeId;
        let mut net = build(n, &edges, seed);
        net.exchange_nbr();
        let start = net.now;
        let v = Voice { burst: 1, ts: 0, cfg: VoiceCfg(0x80), opus: &[0u8; 80] }.encode().expect("v");
        net.originate(origin, PacketType::Voice, false, v);
        net.run_until(start.plus_ms(500));
        // Voice TTL 4: every node within 4 hops (and the 150 ms age cap: 15 ms per hop)
        // receives it, even where ctrl would have been pruned.
        let expect: BTreeSet<NodeId> = net
            .distances(origin)
            .into_iter()
            .filter(|&(v, d)| v != origin && d <= 4)
            .map(|(v, _)| v)
            .collect();
        let got: BTreeSet<NodeId> = net.rx.iter().filter(|r| r.header.ptype == PacketType::Voice).map(|r| r.node).collect();
        prop_assert_eq!(got, expect);
        prop_assert_eq!(net.routers.values().map(|r| r.stats.pruned).sum::<u64>(), 0);
    }
}

#[test]
fn pruning_saves_transmissions_in_a_cluster() {
    // A 6-node clique capped at degree 3 still has redundancy.
    let (_, edges) = random_geometric(12, 0.9, 3, 42);
    let mut plain = build(12, &edges, 1);
    let (a, tx_plain) = flood(
        &mut plain,
        1,
        PacketType::Presence,
        Presence::default().encode(),
    );
    let mut pruned = build(12, &edges, 1);
    pruned.exchange_nbr();
    let (b, tx_pruned) = flood(
        &mut pruned,
        1,
        PacketType::Presence,
        Presence::default().encode(),
    );
    assert_eq!(a, b);
    assert!(
        tx_pruned < tx_plain,
        "pruned {tx_pruned} vs plain {tx_plain}"
    );
}
