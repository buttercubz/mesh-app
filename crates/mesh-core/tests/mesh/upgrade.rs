//! Wi-Fi Aware link bundles (DESIGN §9.6-§9.7, Phase 4 hooks): the UPGRADE exchange over
//! BLE, the Aware link joining the neighbor's bundle, per-packet link choice with the
//! 1 s overlap, and the fallback to BLE when the Aware link goes away.

use mesh_core::effects::RadioCommand;
use mesh_core::{LinkDownReason, LinkKind, Platform, PlatformEvent, RadioAvail};
use mesh_wire::record::rtype;

use crate::talk::{Talker, run_talking};
use crate::voice_on;
use crate::world::World;

fn aware_ops(w: &World, i: usize) -> Vec<(u16, u8)> {
    w.radio(i)
        .into_iter()
        .filter_map(|c| match c {
            RadioCommand::AwareUpgrade { peer, op } => Some((peer, op)),
            _ => None,
        })
        .collect()
}

#[test]
fn aware_upgrade_bundle_overlap_and_fallback() {
    let mut w = World::build(3, 41, Platform::Ios, true);
    w.start_all();
    for i in 0..3 {
        w.with(i, |e, now| {
            e.handle_event(
                now,
                PlatformEvent::RadioState {
                    ble: RadioAvail::On,
                    aware: RadioAvail::On,
                },
            )
        });
    }
    let ble01 = w.connect(0, 1);
    w.connect(2, 0);
    w.run_ms(2_500);
    let (n0, n1) = (w.nodes[0].node_id, w.nodes[1].node_id);
    let (lo, hi) = if n0 < n1 { (0, 1) } else { (1, 0) };
    let (id_lo, id_hi) = (w.nodes[lo].node_id, w.nodes[hi].node_id);
    // The lower node_id offers; the other accepts; both start the Aware data path.
    assert!(
        aware_ops(&w, lo).contains(&(id_hi, 1)),
        "offer: {:?}",
        aware_ops(&w, lo)
    );
    assert!(
        aware_ops(&w, lo).contains(&(id_hi, 2)),
        "accepted: {:?}",
        aware_ops(&w, lo)
    );
    assert!(
        aware_ops(&w, hi).contains(&(id_lo, 2)),
        "accept: {:?}",
        aware_ops(&w, hi)
    );
    let ble_lo = w.local(ble01, lo);
    assert!(
        w.sent_records(lo, ble_lo)
            .iter()
            .any(|(_, t)| *t == rtype::UPGRADE)
    );

    // The native layer brings the Aware link up: it joins the bundle and READY follows.
    w.kind = LinkKind::WifiAware;
    let aw = w.connect(lo, hi);
    w.kind = LinkKind::BleL2cap;
    w.run_ms(500);
    assert!(
        aware_ops(&w, lo).contains(&(id_hi, 4)),
        "ready: {:?}",
        aware_ops(&w, lo)
    );

    // Voice from node 2 relayed by node 0 to node 1 goes on the Aware link; BLE carries
    // copies only during the 1 s overlap after the switch.
    let origin = w.nodes[2].node_id;
    let mut t = vec![Talker::new(2, 9)];
    run_talking(&mut w, &mut t, 3_000);
    let aw0 = w.local(aw, 0);
    let ble0 = w.local(ble01, 0);
    let on_aware = voice_on(&w, 0, aw0, origin);
    let on_ble = voice_on(&w, 0, ble0, origin);
    assert!(
        on_aware.len() as u32 + 1 >= t[0].frames,
        "aware carried {}",
        on_aware.len()
    );
    if let (Some(first), Some(last)) = (on_ble.first(), on_ble.last()) {
        assert!(
            last.0 - first.0 < 1_000_000,
            "BLE copies only within the overlap"
        );
    }

    // My own voice to that neighbor follows the bundle too (origination goes through
    // the glue): Aware only, no copy on BLE once the overlap is over.
    let own_from = w.now;
    t.push(Talker::new(0, 4));
    run_talking(&mut w, &mut t, 1_000);
    t[1].stop(&mut w);
    w.run_ms(100);
    let me0 = w.nodes[0].node_id;
    let own_aware = voice_on(&w, 0, aw0, me0).len() as u32;
    let own_ble = voice_on(&w, 0, ble0, me0)
        .into_iter()
        .filter(|(at, _)| *at >= own_from)
        .count();
    assert!(
        own_aware + 1 >= t[1].frames,
        "own voice on Aware: {own_aware} of {}",
        t[1].frames
    );
    assert_eq!(own_ble, 0, "no duplicate of my own voice on the BLE link");

    // Aware goes silent (no RX for 600 ms): downgrade to BLE with overlap, and the
    // voice continues on BLE without a gap.
    w.links[aw].up = false;
    let t1 = w.now;
    run_talking(&mut w, &mut t, 2_000);
    let ble_after: Vec<u64> = voice_on(&w, 0, ble0, origin)
        .into_iter()
        .map(|(at, _)| at)
        .filter(|at| *at >= t1)
        .collect();
    let first = *ble_after.first().expect("voice moved to BLE");
    assert!(
        first - t1 <= 700_000,
        "downgrade after {} ms",
        (first - t1) / 1_000
    );
    assert!(
        ble_after.len() >= 30,
        "BLE carries the stream: {}",
        ble_after.len()
    );
    let gaps = ble_after.windows(2).map(|p| p[1] - p[0]).max().unwrap_or(0);
    assert!(
        gaps <= 45_000,
        "no gap on BLE after the downgrade ({gaps} µs)"
    );
    let aware_after = voice_on(&w, 0, aw0, origin)
        .into_iter()
        .filter(|(at, _)| *at >= first + 1_050_000)
        .count();
    assert_eq!(
        aware_after, 0,
        "the silent Aware link stops carrying voice after the overlap"
    );

    // The Aware link is torn down: retire.
    w.links[aw].up = true;
    w.drop_link(aw, LinkDownReason::PeerClosed);
    assert!(aware_ops(&w, 0).contains(&(w.nodes[1].node_id, 5)));
}
