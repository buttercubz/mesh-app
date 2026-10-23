use super::*;
use mesh_routing::{LinkRole, NeighborTable};

fn up(t: &mut NeighborTable, now: Instant, link: u32, kind: LinkKind) {
    t.link_up(now, 2, LinkId(link), kind, LinkRole::Initiator, 1024)
        .expect("link up");
}

fn touch(t: &mut NeighborTable, now: Instant, link: u32) {
    if let Some(l) = t.link_mut(LinkId(link)) {
        l.last_rx = now;
    }
}

#[test]
fn ble_alone_then_aware_upgrade_with_overlap() {
    let mut t = NeighborTable::new();
    let mut b = Bundles::new();
    let t0 = Instant::from_ms(1_000);
    up(&mut t, t0, 1, LinkKind::Ble);
    let (s, sw) = b.select(t0, t.neighbor(2).unwrap());
    assert_eq!(s.unwrap().primary, LinkId(1));
    assert!(sw.is_none());

    up(&mut t, t0.plus_ms(10), 2, LinkKind::Aware);
    if let Some(l) = t.link_mut(LinkId(2)) {
        l.quality.rtt.sample(t0.plus_ms(10), 20_000);
    }
    let at = t0.plus_ms(20);
    touch(&mut t, at, 2);
    let (s, sw) = b.select(at, t.neighbor(2).unwrap());
    let s = s.unwrap();
    assert_eq!(s.primary, LinkId(2));
    assert_eq!(s.overlap, Some(LinkId(1)), "1 s overlap on the old link");
    assert_eq!(
        sw,
        Some(Switch {
            node: 2,
            from: LinkId(1),
            to: LinkId(2),
            downgrade: false
        })
    );
    // Still overlapping 999 ms later, not at 1 s.
    let later = at.plus_ms(999);
    touch(&mut t, later, 2);
    assert_eq!(
        b.select(later, t.neighbor(2).unwrap()).0.unwrap().overlap,
        Some(LinkId(1))
    );
    let after = at.plus_ms(1_000);
    touch(&mut t, after, 2);
    let (s, sw) = b.select(after, t.neighbor(2).unwrap());
    assert_eq!(s.unwrap().overlap, None);
    assert!(sw.is_none());
}

#[test]
fn slow_aware_is_not_used_and_silence_downgrades() {
    let mut t = NeighborTable::new();
    let mut b = Bundles::new();
    let t0 = Instant::from_ms(1_000);
    up(&mut t, t0, 1, LinkKind::Ble);
    up(&mut t, t0, 2, LinkKind::Aware);
    if let Some(l) = t.link_mut(LinkId(2)) {
        l.quality.rtt.sample(t0, 80_000);
    }
    // srtt 80 ms ≥ 50 ms: BLE.
    assert_eq!(
        b.select(t0, t.neighbor(2).unwrap()).0.unwrap().primary,
        LinkId(1)
    );

    let mut t = NeighborTable::new();
    let mut b = Bundles::new();
    up(&mut t, t0, 1, LinkKind::Ble);
    up(&mut t, t0, 2, LinkKind::Aware);
    assert_eq!(
        b.select(t0, t.neighbor(2).unwrap()).0.unwrap().primary,
        LinkId(2)
    );
    // 599 ms without Aware RX: kept; 600 ms: downgrade with overlap.
    assert_eq!(
        b.select(t0.plus_ms(599), t.neighbor(2).unwrap())
            .0
            .unwrap()
            .primary,
        LinkId(2)
    );
    let at = t0.plus_ms(600);
    let (s, sw) = b.select(at, t.neighbor(2).unwrap());
    assert_eq!(s.unwrap().primary, LinkId(1));
    assert_eq!(s.unwrap().overlap, Some(LinkId(2)));
    assert!(sw.unwrap().downgrade);
}

#[test]
fn tunnel_only_after_five_seconds_without_radio() {
    let mut t = NeighborTable::new();
    let mut b = Bundles::new();
    let t0 = Instant::from_ms(1_000);
    up(&mut t, t0, 3, LinkKind::Tunnel);
    let n = t.neighbor(2).unwrap().clone();
    assert!(b.select(t0, &n).0.is_none());
    assert!(b.select(t0.plus_ms(4_999), &n).0.is_none());
    assert_eq!(
        b.select(t0.plus_ms(5_000), &n).0.unwrap().primary,
        LinkId(3)
    );
    // A radio link appears: it wins at once.
    up(&mut t, t0.plus_ms(5_100), 1, LinkKind::Ble);
    let (s, sw) = b.select(t0.plus_ms(5_100), t.neighbor(2).unwrap());
    assert_eq!(s.unwrap().primary, LinkId(1));
    assert!(sw.is_some());
    b.forget(2);
    assert!(b.choice.is_empty());
}
