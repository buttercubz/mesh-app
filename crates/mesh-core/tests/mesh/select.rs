//! Link selection policy (DESIGN §5.7, §9.5): redundant links over the max are closed
//! (never the only path), `LimitReached` lowers the max, and a lost iPhone link arms a
//! pending reconnect.

use mesh_core::effects::RadioCommand;
use mesh_core::{LinkDownReason, LinkId, Platform};
use mesh_wire::frame::RejectReason;

use crate::world::World;

fn closes(w: &World, i: usize) -> Vec<(LinkId, Option<RejectReason>)> {
    w.radio(i)
        .into_iter()
        .filter_map(|c| match c {
            RadioCommand::Close { link, reason } => Some((link, reason)),
            _ => None,
        })
        .collect()
}

fn ble_links(w: &World, i: usize) -> usize {
    w.nodes[i].engine.ctx().links.active_count(true)
}

#[test]
fn over_max_closes_a_redundant_link_never_the_only_path() {
    let mut w = World::new(5, 31);
    w.start_all();
    let l01 = w.connect(0, 1);
    let l02 = w.connect(0, 2);
    w.connect(1, 2);
    let l03 = w.connect(0, 3);
    let l04 = w.connect(0, 4);
    assert_eq!(ble_links(&w, 0), 4, "one over the BLE max of 3");
    w.run_ms(4_500);
    let c = closes(&w, 0);
    let redundant: Vec<LinkId> = c
        .iter()
        .filter(|(_, r)| *r == Some(RejectReason::Redundant))
        .map(|(l, _)| *l)
        .collect();
    assert_eq!(redundant.len(), 1, "exactly one link closed: {c:?}");
    assert!(
        [w.local(l01, 0), w.local(l02, 0)].contains(&redundant[0]),
        "only a link whose peer is reachable another way"
    );
    for only in [l03, l04] {
        assert!(
            !redundant.contains(&w.local(only, 0)),
            "an articulation link was closed"
        );
    }
    assert_eq!(ble_links(&w, 0), 3);
    // Nodes 3 and 4 are untouched.
    assert!(closes(&w, 3).is_empty() && closes(&w, 4).is_empty());
}

#[test]
fn limit_reached_lowers_the_max_for_ten_minutes() {
    let mut w = World::new(5, 32);
    w.start_all();
    w.connect(0, 1);
    w.connect(0, 2);
    w.connect(1, 2);
    let l03 = w.connect(0, 3);
    w.run_ms(3_000);
    assert!(closes(&w, 0).is_empty(), "3 links are within the max");
    // The OS refused a connection: this device's BLE max is 2 for 10 min.
    w.drop_link(l03, LinkDownReason::LimitReached);
    w.connect(0, 4);
    w.run_ms(4_500);
    let redundant = closes(&w, 0)
        .into_iter()
        .filter(|(_, r)| *r == Some(RejectReason::Redundant))
        .count();
    assert_eq!(redundant, 1, "3 links over the lowered max of 2");
    assert_eq!(ble_links(&w, 0), 2);
}

#[test]
fn lost_iphone_link_arms_a_pending_reconnect() {
    let mut w = World::new(3, 33);
    w.start_all();
    let a = w.connect(0, 1);
    let b = w.connect(0, 2);
    w.run_ms(2_000);
    w.drop_link(a, LinkDownReason::Timeout);
    w.drop_link(b, LinkDownReason::LimitReached);
    let armed: Vec<RadioCommand> = w
        .radio(0)
        .into_iter()
        .filter(|c| matches!(c, RadioCommand::ArmReconnect { .. }))
        .collect();
    assert_eq!(
        armed,
        vec![RadioCommand::ArmReconnect {
            peer_ref: 5_001,
            until_s: 900
        }],
        "the initiator arms toward the timed-out peer only"
    );
    // The responder side of the same link arms nothing.
    assert!(
        !w.radio(1)
            .iter()
            .any(|c| matches!(c, RadioCommand::ArmReconnect { .. }))
    );
    // Android uses normal scanning (the policy is the iPhone T3 variant).
    let mut w = World::with_platform(2, 34, Platform::Android);
    w.start_all();
    let a = w.connect(0, 1);
    w.run_ms(2_000);
    w.drop_link(a, LinkDownReason::Timeout);
    assert!(
        !w.radio(0)
            .iter()
            .any(|c| matches!(c, RadioCommand::ArmReconnect { .. }))
    );
}
