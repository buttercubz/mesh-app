//! PING/PONG timing (DESIGN §5.1, §5.6, §9.6): `t_tx` and `hold_us` are measured when
//! the frame carrying them is sealed, so the RTT excludes both sides' token-bucket
//! waits; the PING period depends on the link kind (Wi-Fi Aware keeps 500 ms).

use super::*;
use crate::links::security::Plain;
use crate::links::types::LinkRole;
use mesh_wire::frame::lab_tag;
use mesh_wire::record::RecordIter;

const A: LinkId = LinkId(1);
const B: LinkId = LinkId(2);

fn lab_link(t: &mut LinkTable, id: LinkId, kind: LinkKind, role: LinkRole, fx: &mut EffectQueue) {
    let sec = Box::new(Plain::new(lab_tag(77)));
    t.add(Instant(0), id, LinkInfo::new(kind, role), sec, fx);
    assert_eq!(t.get(id).map(|l| l.state), Some(LinkState::Up));
    t.drain_events();
}

/// Records of every unit `from` emits now on `id`, after `to` (on `to_id`) read them.
fn deliver(
    now: Instant,
    from: &mut LinkTable,
    to: &mut LinkTable,
    to_id: LinkId,
    fx: &mut EffectQueue,
) -> Vec<Record<'static>> {
    let mut seen = Vec::new();
    while let Some(t) = from.poll_transmit(now, fx) {
        for pt in to.inbound(now, to_id, &t.bytes, fx) {
            for rec in RecordIter::new(&pt).flatten() {
                to.on_link_record(now, to_id, &rec, fx);
                match rec {
                    Record::Ping(p) => seen.push(Record::Ping(p)),
                    Record::Pong(p) => seen.push(Record::Pong(p)),
                    _ => {}
                }
            }
        }
    }
    seen
}

#[test]
fn ping_and_pong_are_stamped_when_sealed_so_pacing_waits_are_not_rtt() {
    let mut fx = EffectQueue::new();
    let (mut a, mut b) = (LinkTable::new(), LinkTable::new());
    lab_link(&mut a, A, LinkKind::BleL2cap, LinkRole::Initiator, &mut fx);
    lab_link(&mut b, B, LinkKind::BleL2cap, LinkRole::Responder, &mut fx);
    let limits = crate::config::Limits::for_platform(crate::config::Platform::Ios);
    // A ~810 B frame puts each token bucket in debt: A's at 0 for ~81 ms (80 kbps), B's
    // at 60 ms for ~40 ms (160 kbps).
    for (t, id, rate, at) in [(&mut a, A, 80_000, 0), (&mut b, B, 160_000, 60_000)] {
        t.set_pace(Instant(at), id, Some(rate));
        assert!(t.send_record(id, Prio::P2, &Record::Pad(800), None));
        assert!(t.poll_transmit(Instant(at), &mut fx).is_some());
    }
    // The PING is due at 1 ms but waits for the bucket: nothing is built yet.
    a.on_timer(Instant(1_000), &limits, &mut fx);
    assert!(a.poll_transmit(Instant(1_000), &mut fx).is_none());
    assert_eq!(a.get(A).map(|l| l.counters.pings_sent), Some(0));
    // A pacing deadline wakes the engine for it.
    assert!(
        a.next_deadline(&limits)
            .is_some_and(|d| d > Instant(1_000) && d <= Instant(90_000))
    );
    let sealed = Instant(90_000);
    let got = deliver(sealed, &mut a, &mut b, B, &mut fx);
    match got.as_slice() {
        [Record::Ping(p)] => assert_eq!(p.t_tx, sealed.0 as u32, "t_tx is the sealing instant"),
        other => panic!("expected one PING, got {other:?}"),
    }
    assert_eq!(a.get(A).map(|l| l.counters.pings_sent), Some(1));
    // B still owes its bucket: the PONG waits, and its hold_us covers that wait.
    assert!(deliver(sealed, &mut b, &mut a, A, &mut fx).is_empty());
    let answered = Instant(120_000);
    let got = deliver(answered, &mut b, &mut a, A, &mut fx);
    match got.as_slice() {
        [Record::Pong(p)] => assert_eq!(u64::from(p.hold_us), answered.0 - sealed.0),
        other => panic!("expected one PONG, got {other:?}"),
    }
    // Zero path delay in this test: the RTT is 0, not the 119 ms of local pacing.
    assert_eq!(a.get(A).and_then(|l| l.srtt_us()), Some(0));
}

#[test]
fn aware_links_ping_every_500_ms_and_ble_links_follow_the_burst_flag() {
    let mut fx = EffectQueue::new();
    let mut t = LinkTable::new();
    let (ble, aware) = (LinkId(1), LinkId(2));
    lab_link(
        &mut t,
        ble,
        LinkKind::BleL2cap,
        LinkRole::Initiator,
        &mut fx,
    );
    lab_link(
        &mut t,
        aware,
        LinkKind::WifiAware,
        LinkRole::Initiator,
        &mut fx,
    );
    let limits = crate::config::Limits::for_platform(crate::config::Platform::Ios);
    assert_eq!(t.ping_period(LinkKind::WifiAware, &limits), 500_000);
    assert_eq!(t.ping_period(LinkKind::BleGatt, &limits), 2_000_000);
    let pings = |t: &LinkTable, id| t.get(id).map_or(0, |l| l.counters.pings_sent);
    let mut step = |t: &mut LinkTable, us: u64| {
        t.on_timer(Instant(us), &limits, &mut fx);
        while t.poll_transmit(Instant(us), &mut fx).is_some() {}
    };
    for k in 0..=6 {
        step(&mut t, k * 250_000);
    }
    // 0 … 1.5 s: Aware at 0, 0.5, 1.0, 1.5 s; BLE (idle) only at 0.
    assert_eq!((pings(&t, ble), pings(&t, aware)), (1, 4));
    // A burst starts: BLE links PING every 500 ms too.
    t.set_fast_ping(true);
    assert_eq!(t.ping_period(LinkKind::BleL2cap, &limits), 500_000);
    for k in 7..=10 {
        step(&mut t, k * 250_000);
    }
    assert_eq!((pings(&t, ble), pings(&t, aware)), (3, 6));
}

#[test]
fn periodic_link_timers_keep_their_phase_when_they_run_late() {
    let t = |s: f64| Instant((s * 1e6) as u64);
    // Due at 2 s, run 30 ms late: next at 4 s, not 4.03 s (PING and LSTAT stay in step).
    assert_eq!(on_grid(t(2.0), 2_000_000, t(2.03)), t(4.0));
    assert_eq!(on_grid(t(2.0), 1_000_000, t(2.0)), t(3.0));
    // A timer that slept through several periods skips them.
    assert_eq!(on_grid(t(2.0), 1_000_000, t(5.5)), t(6.0));
}
