use mesh_wire::record::{Nbr, NbrEntry, NbrFlags};

use super::*;
use crate::consts::{BLE_LINKS_MAX, LIMIT_BACKOFF_US};

fn cand(disc_id: u32, node: Option<NodeId>, class: CandidateClass, rssi: i16) -> Candidate {
    Candidate {
        disc_id,
        peer_ref: u64::from(disc_id) * 1000,
        node,
        class,
        rssi,
        transport: LinkKind::Ble,
    }
}

fn up(t: &mut NeighborTable, node: NodeId, link: u32) {
    t.link_up(
        Instant::ZERO,
        node,
        LinkId(link),
        LinkKind::Ble,
        LinkRole::Initiator,
        1024,
    )
    .expect("up");
}

fn nbr(t: &mut NeighborTable, now: Instant, via: u32, nodes: &[NodeId]) {
    let list: Vec<NbrEntry> = nodes
        .iter()
        .map(|&n| NbrEntry {
            node_id: n,
            q: 90,
            flags: NbrFlags::new(true, false, 0),
            vcap: 2,
        })
        .collect();
    t.on_nbr(now, LinkId(via), &Nbr::new(0, &list).expect("nbr"));
}

#[test]
fn duplicate_resolution() {
    // me 5, peer 9: my initiated link has initiator 5 < 9 → keep it, close theirs.
    let close = resolve_duplicate(
        5,
        9,
        (LinkId(1), LinkRole::Initiator),
        (LinkId(2), LinkRole::Responder),
    );
    assert_eq!(close, LinkId(2));
    // me 9, peer 5: the peer-initiated link (initiator 5) wins.
    let close = resolve_duplicate(
        9,
        5,
        (LinkId(1), LinkRole::Initiator),
        (LinkId(2), LinkRole::Responder),
    );
    assert_eq!(close, LinkId(1));
    // Same initiator: lower link id stays.
    let close = resolve_duplicate(
        9,
        5,
        (LinkId(4), LinkRole::Initiator),
        (LinkId(3), LinkRole::Initiator),
    );
    assert_eq!(close, LinkId(4));
}

#[test]
fn forming_order_and_one_outstanding() {
    let mut s = LinkSelector::new(1, Limits::V1, Instant::ZERO);
    let t = NeighborTable::new();
    let input = SelectInput {
        candidates: vec![
            cand(10, None, CandidateClass::Unknown, -50),
            cand(11, Some(7), CandidateClass::UnreachableMember, -90),
            cand(12, None, CandidateClass::Unknown, -40),
            cand(13, None, CandidateClass::Speculative, -30),
        ],
        unreachable_members: vec![7],
        ..SelectInput::default()
    };
    let now = Instant::from_secs(1);
    let a = s.evaluate(now, &input, &t);
    assert_eq!(
        a,
        vec![
            SelectAction::SetDiscovery(DiscoveryMode::Forming),
            SelectAction::Connect {
                disc_id: 11,
                speculative: false
            }
        ]
    );
    // Outstanding: nothing new until it resolves or times out (8 s).
    assert!(s.evaluate(now.plus_ms(2_000), &input, &t).is_empty());
    let a = s.evaluate(now.plus_ms(8_000), &input, &t);
    assert_eq!(a[0], SelectAction::Abort { disc_id: 11 });
    // 11 is backing off (5 s): the strongest unknown is next.
    assert_eq!(
        a[1],
        SelectAction::Connect {
            disc_id: 12,
            speculative: false
        }
    );
    s.on_connect_result(now.plus_ms(9_000), 12, false);
    let a = s.evaluate(now.plus_ms(10_000), &input, &t);
    assert_eq!(
        a,
        vec![SelectAction::Connect {
            disc_id: 10,
            speculative: false
        }]
    );
    s.on_connect_result(now.plus_ms(10_500), 10, false);
    // 11's backoff (5 s from +8 s) is over at +13 s; 12 and 10 still back off.
    let a = s.evaluate(now.plus_ms(13_000), &input, &t);
    assert_eq!(
        a,
        vec![SelectAction::Connect {
            disc_id: 11,
            speculative: false
        }]
    );
    s.on_connect_result(now.plus_ms(14_000), 11, false);
    // 12 (5 s from +9 s) is free again; its second failure doubles to 10 s.
    let a = s.evaluate(now.plus_ms(15_000), &input, &t);
    assert_eq!(
        a,
        vec![SelectAction::Connect {
            disc_id: 12,
            speculative: false
        }]
    );
    s.on_connect_result(now.plus_ms(15_500), 12, false);
    let a = s.evaluate(now.plus_ms(16_000), &input, &t);
    assert_eq!(
        a,
        vec![SelectAction::Connect {
            disc_id: 10,
            speculative: false
        }]
    );
    s.on_connect_result(now.plus_ms(16_500), 10, false);
    // Everyone else backs off: the speculative peer, lowest priority, goes last.
    let a = s.evaluate(now.plus_ms(17_000), &input, &t);
    assert_eq!(
        a,
        vec![SelectAction::Connect {
            disc_id: 13,
            speculative: true
        }]
    );
    s.on_connect_result(now.plus_ms(17_100), 13, true);
}

#[test]
fn speculative_rate_limit_and_new_token_reset() {
    let mut s = LinkSelector::new(1, Limits::V1, Instant::ZERO);
    let t = NeighborTable::new();
    let input = SelectInput {
        candidates: vec![
            cand(20, None, CandidateClass::Speculative, -30),
            cand(21, None, CandidateClass::Speculative, -30),
        ],
        unreachable_members: vec![7],
        ..SelectInput::default()
    };
    let now = Instant::from_secs(1);
    assert!(
        s.evaluate(now, &input, &t)
            .contains(&SelectAction::Connect {
                disc_id: 20,
                speculative: true
            })
    );
    s.on_connect_result(now.plus_ms(100), 20, true);
    // ≤ 1 speculative link per 30 s.
    assert!(
        !s.evaluate(now.plus_ms(10_000), &input, &t)
            .iter()
            .any(|a| matches!(a, SelectAction::Connect { .. }))
    );
    assert!(
        s.evaluate(now.plus_ms(30_000), &input, &t)
            .iter()
            .any(|a| matches!(a, SelectAction::Connect { .. }))
    );
    s.on_connect_result(now.plus_ms(30_100), 20, false);
    s.on_new_token(20_000);
    assert!(!s.in_backoff(now.plus_ms(30_200), 20_000));
}

#[test]
fn stable_at_target_and_limit_reached() {
    let mut s = LinkSelector::new(1, Limits::V1, Instant::ZERO);
    let mut t = NeighborTable::new();
    up(&mut t, 2, 1);
    up(&mut t, 3, 2);
    let input = SelectInput {
        candidates: vec![cand(30, None, CandidateClass::Unknown, -40)],
        reachable_members: vec![2, 3],
        ..SelectInput::default()
    };
    let now = Instant::from_secs(1);
    assert_eq!(
        s.evaluate(now, &input, &t),
        vec![SelectAction::SetDiscovery(DiscoveryMode::Stable)]
    );
    assert_eq!(s.ble_max(now), BLE_LINKS_MAX);
    s.on_limit_reached(now);
    s.on_limit_reached(now);
    s.on_limit_reached(now);
    assert_eq!(s.ble_max(now), 1);
    assert_eq!(s.ble_max(now.plus_us(LIMIT_BACKOFF_US)), BLE_LINKS_MAX);
}

#[test]
fn over_max_closes_redundant_but_never_an_articulation() {
    let mut s = LinkSelector::new(1, Limits::V1, Instant::ZERO);
    let mut t = NeighborTable::new();
    let now = Instant::from_secs(1);
    for (n, l) in [(2u16, 1u32), (3, 2), (4, 3), (5, 4)] {
        up(&mut t, n, l);
    }
    // 2 and 3 see each other (each is redundant); 5 is the only path to member 9.
    nbr(&mut t, now, 1, &[3]);
    nbr(&mut t, now, 2, &[2]);
    nbr(&mut t, now, 4, &[9]);
    t.on_rssi(LinkId(2), Some(-95)); // node 3 has the worst q
    let input = SelectInput {
        reachable_members: vec![2, 3, 4, 5, 9],
        ..SelectInput::default()
    };
    let a = s.evaluate(now, &input, &t);
    assert!(
        a.contains(&SelectAction::CloseRedundant { link: LinkId(2) }),
        "{a:?}"
    );
    // Without redundancy reports nothing is closed.
    let mut t2 = NeighborTable::new();
    for (n, l) in [(2u16, 1u32), (3, 2), (4, 3), (5, 4)] {
        up(&mut t2, n, l);
    }
    nbr(&mut t2, now, 4, &[9]);
    let a = s.evaluate(now.plus_ms(2_000), &input, &t2);
    assert!(
        !a.iter()
            .any(|x| matches!(x, SelectAction::CloseRedundant { .. })),
        "{a:?}"
    );
    // The 2-hop graph helper: removing me–5 cuts 9 off.
    let adj = two_hop_graph(now, 1, &t2);
    assert!(reachable_without(&adj, 1, None).contains(&9));
    assert!(!reachable_without(&adj, 1, Some(5)).contains(&9));
}
