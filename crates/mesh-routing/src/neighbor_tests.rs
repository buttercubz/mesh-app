use super::*;
use mesh_wire::record::{NbrEntry, NbrFlags};

fn table() -> NeighborTable {
    let mut t = NeighborTable::new();
    let now = Instant::ZERO;
    t.link_up(now, 2, LinkId(1), LinkKind::Ble, LinkRole::Initiator, 1024)
        .expect("up");
    t.link_up(now, 3, LinkId(2), LinkKind::Ble, LinkRole::Responder, 1024)
        .expect("up");
    t
}

#[test]
fn bundle_and_duplicates() {
    let mut t = table();
    let now = Instant::ZERO;
    assert_eq!(
        t.link_up(now, 2, LinkId(9), LinkKind::Ble, LinkRole::Responder, 1024),
        Err(LinkUpError::Duplicate(LinkId(1)))
    );
    assert_eq!(
        t.link_up(now, 4, LinkId(1), LinkKind::Ble, LinkRole::Responder, 1024),
        Err(LinkUpError::KnownLink)
    );
    t.link_up(
        now,
        2,
        LinkId(3),
        LinkKind::Aware,
        LinkRole::Initiator,
        1200,
    )
    .expect("aware slot is free");
    assert_eq!(t.len(), 2);
    assert_eq!(t.count_kind(LinkKind::Ble), 2);
    // Aware is preferred when both are good and uncongested.
    let n = t.neighbor(2).expect("n");
    assert_eq!(n.best_link(now).map(|l| l.id), Some(LinkId(3)));
    // A congested Aware loses to an uncongested BLE.
    t.mark_congested(now, LinkId(3));
    let n = t.neighbor(2).expect("n");
    assert_eq!(n.best_link(now).map(|l| l.id), Some(LinkId(1)));
    assert!(!t.link(LinkId(3)).expect("l").congested(now.plus_ms(2_000)));
    assert_eq!(t.link_down(LinkId(1)), Some((2, LinkKind::Ble)));
    assert_eq!(t.len(), 2);
    t.link_down(LinkId(3));
    assert_eq!(t.len(), 1);
    assert_eq!(t.node_of(LinkId(3)), None);
}

#[test]
fn liveness_and_death() {
    let mut t = table();
    assert!(t.on_rx(Instant::from_secs(3), LinkId(1), 1, 100));
    let dead = t.dead_links(Instant::from_secs(4));
    assert_eq!(dead, vec![LinkId(2)]);
    assert!(t.dead_links(Instant::from_ms(3_999)).is_empty());
}

#[test]
fn ingress_caps() {
    let mut t = table();
    let now = Instant::from_secs(1);
    // 300 records/s.
    assert!(t.on_rx(now, LinkId(1), 300, 1_000));
    assert!(!t.on_rx(now, LinkId(1), 1, 10));
    // Refill after a second.
    assert!(t.on_rx(now.plus_ms(1_000), LinkId(1), 10, 1_000));
    // 64 KB/s.
    assert!(!t.on_rx(now.plus_ms(1_000), LinkId(2), 1, 70_000));
    assert!(!t.on_rx(now, LinkId(77), 1, 1));
}

#[test]
fn penalties_and_ban() {
    let mut t = table();
    let now = Instant::from_secs(10);
    for _ in 0..3 {
        assert!(!t.penalize(now, 2, 5));
    }
    // Old points expire after a minute.
    assert!(!t.penalize(now.plus_ms(61_000), 2, 5));
    assert_eq!(t.neighbor(2).expect("n").penalty(now.plus_ms(61_000)), 5);
    for _ in 0..3 {
        t.penalize(now.plus_ms(61_000), 2, 5);
    }
    assert!(t.banned_until(now.plus_ms(61_000), 2).is_some());
    t.link_down(LinkId(1));
    assert!(matches!(
        t.link_up(
            now.plus_ms(62_000),
            2,
            LinkId(5),
            LinkKind::Ble,
            LinkRole::Initiator,
            1024
        ),
        Err(LinkUpError::Banned(_))
    ));
    t.tick(now.plus_ms(122_000));
    assert!(t.banned_until(now.plus_ms(122_000), 2).is_none());
    assert!(
        t.link_up(
            now.plus_ms(122_000),
            2,
            LinkId(5),
            LinkKind::Ble,
            LinkRole::Initiator,
            1024
        )
        .is_ok()
    );
}

#[test]
fn nbr_ingest_and_own_entries() {
    let mut t = table();
    let now = Instant::from_secs(1);
    let nbr = Nbr::new(
        1,
        &[NbrEntry {
            node_id: 3,
            q: 90,
            flags: NbrFlags::new(true, false, 0),
            vcap: 2,
        }],
    )
    .expect("nbr");
    t.on_nbr(now, LinkId(1), &nbr);
    let n = t.neighbor(2).expect("n");
    assert!(n.fresh_nbr(now).is_some());
    assert!(n.fresh_nbr(now.plus_ms(1_500)).is_none());
    t.set_vcap(LinkId(2), 1);
    t.mark_voice_tx(now, LinkId(2));
    assert!(
        t.link(LinkId(2))
            .expect("l")
            .is_voice_link(now.plus_ms(1_999))
    );
    assert!(
        !t.link(LinkId(2))
            .expect("l")
            .is_voice_link(now.plus_ms(2_000))
    );
    let own = t.own_entries(now);
    assert_eq!(own.len(), 2);
    assert!(own.iter().any(|e| e.node_id == 3 && e.vcap == 1 && e.good));
}
