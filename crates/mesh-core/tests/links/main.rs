//! Links (WP-10A): Lab link bring-up over the real framing, CAPS, PING/PONG, link
//! death, Lab code mismatch, GATT links, WouldBlock ordering, RadioState Off, the
//! discovery/connect policy and determinism.

mod harness;

use harness::{World, disc_of};
use mesh_api::{NoticeKey, UiEvent};
use mesh_core::effects::{DiscoveryMode, Effect, RadioCommand};
use mesh_core::links::LinkState;
use mesh_core::{Instant, LinkDownReason, LinkKind, Platform, PlatformEvent, RadioAvail};
use mesh_wire::frame::RejectReason;

fn up_links(w: &World, i: usize) -> usize {
    w.nodes[i]
        .engine
        .ctx()
        .links
        .iter()
        .filter(|l| l.state == LinkState::Up)
        .count()
}

#[test]
fn lab_links_come_up_with_caps_announce_and_rtt() {
    let mut w = World::new(2, Platform::Ios, 1);
    w.start_lab(1234);
    w.connect(0, 1);
    w.run_ms(3_000);
    for i in 0..2 {
        assert_eq!(up_links(&w, i), 1);
        let l = w.nodes[i].engine.ctx().links.iter().next().unwrap();
        let caps = l.peer_caps.expect("peer CAPS received");
        assert_eq!(
            (caps.epoch, caps.roster_version, caps.ctl_version),
            (0, 0, 0)
        );
        assert_eq!(caps.proto, 1);
        let other = w.nodes[1 - i].engine.lab_node_id().unwrap();
        assert_eq!(l.peer_node.map(|n| n.0), Some(other), "LAB_CTL announce");
        let srtt = l.srtt_us().expect("PING/PONG measured an RTT");
        assert!((19_000..=22_000).contains(&srtt), "rtt {srtt}");
        assert!(l.peer_lstat.is_some(), "LSTAT every second");
        assert!(l.counters.pings_sent >= 5, "500 ms pings in Lab");
    }
    assert!(w.ui_events(0).iter().any(|e| matches!(
        e,
        UiEvent::Notice {
            key: NoticeKey::LabPeerJoined,
            ..
        }
    )));
}

#[test]
fn wrong_lab_code_is_rejected() {
    let mut w = World::new(2, Platform::Ios, 2);
    w.start_lab_one(0, 1234);
    w.start_lab_one(1, 4321);
    w.connect(0, 1);
    w.run_ms(2_000);
    assert_eq!(up_links(&w, 0), 0);
    assert_eq!(up_links(&w, 1), 0);
    let closes: Vec<_> = (0..2)
        .flat_map(|i| w.radio_cmds(i))
        .filter_map(|c| match c {
            RadioCommand::Close { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect();
    assert!(
        closes.contains(&Some(RejectReason::LabCodeMismatch)),
        "{closes:?}"
    );
}

#[test]
fn link_dies_after_four_silent_seconds() {
    let mut w = World::new(2, Platform::Ios, 3);
    w.start_lab(77);
    let li = w.connect(0, 1);
    w.run_ms(1_000);
    w.links[li].blackout = true;
    w.run_ms(3_500);
    assert_eq!(up_links(&w, 0), 1, "still alive before 4 s");
    w.run_ms(1_000);
    assert_eq!(up_links(&w, 0), 0);
    assert!(
        w.radio_cmds(0)
            .iter()
            .any(|c| matches!(c, RadioCommand::Close { reason: None, .. }))
    );
}

#[test]
fn gatt_links_chunk_frames_and_keep_order_after_would_block() {
    let mut w = World::new(2, Platform::Android, 4);
    w.kind = LinkKind::BleGatt;
    w.att_mtu = 23;
    w.start_lab(55);
    w.nodes[0].would_block_next = 3;
    w.connect(0, 1);
    w.run_ms(3_000);
    assert_eq!(up_links(&w, 0), 1);
    assert_eq!(up_links(&w, 1), 1);
    let l = w.nodes[1].engine.ctx().links.iter().next().unwrap();
    assert_eq!(l.info.kind, LinkKind::BleGatt);
    assert!(
        l.peer_caps.is_some(),
        "CAPS (21 B) crossed 20 B GATT chunks"
    );
    assert_eq!(l.counters.bad_frames, 0, "no chunk was reordered or lost");
    assert!(
        w.nodes[0].tx_log.iter().all(|(_, _, b)| b.len() <= 20),
        "every ATT value fits MTU 23"
    );
}

#[test]
fn radio_off_marks_ble_links_down_and_pauses_discovery() {
    let mut w = World::new(2, Platform::Ios, 5);
    w.start_lab(900);
    w.connect(0, 1);
    w.run_ms(1_000);
    let before = w.nodes[0].effects.len();
    w.event(
        0,
        PlatformEvent::RadioState {
            ble: RadioAvail::Off,
            aware: RadioAvail::Unsupported,
        },
    );
    assert_eq!(up_links(&w, 0), 0, "BLE links marked down");
    let new = &w.nodes[0].effects[before..];
    assert!(new.iter().any(|e| matches!(
        e,
        Effect::Radio(RadioCommand::SetDiscovery {
            mode: DiscoveryMode::Off,
            ..
        })
    )));
    assert!(new.iter().any(|e| matches!(
        e,
        Effect::Ui(UiEvent::Notice {
            key: NoticeKey::BluetoothOff,
            ..
        })
    )));
    assert_eq!(w.nodes[0].discovery.0, DiscoveryMode::Off);
    let s = w.nodes[0].engine.ctx().ui.last().cloned();
    w.run_ms(200);
    let s2 = w.nodes[0].engine.ctx().ui.last().cloned().unwrap();
    assert_eq!(s2.device.bluetooth, mesh_api::BluetoothState::Off, "{s:?}");
    // Discovered peers are ignored while off.
    w.event(
        0,
        PlatformEvent::Discovered(mesh_core::Discovery {
            disc_id: disc_of(1),
            transport: mesh_core::Transport::Ble,
            rssi: Some(-50),
            adv: mesh_core::AdvData::UuidOnly,
            platform_hint: None,
        }),
    );
    assert!(
        !w.radio_cmds(0)
            .iter()
            .any(|c| matches!(c, RadioCommand::Connect { .. }))
    );
    w.event(
        0,
        PlatformEvent::RadioState {
            ble: RadioAvail::On,
            aware: RadioAvail::Unsupported,
        },
    );
    assert_eq!(w.nodes[0].discovery.0, DiscoveryMode::Forming);
}

#[test]
fn discovery_connects_by_role_rule_and_aborts_strangers() {
    let mut w = World::new(3, Platform::Ios, 6);
    w.start_lab_one(0, 1111);
    w.start_lab_one(1, 1111);
    w.start_lab_one(2, 2222);
    w.in_range = vec![(0, 1), (0, 2), (1, 2)];
    for _ in 0..3 {
        w.advertise();
        w.run_ms(500);
    }
    assert_eq!(up_links(&w, 0), 1);
    assert_eq!(up_links(&w, 1), 1);
    assert_eq!(up_links(&w, 2), 0, "other Lab code never links");
    // Exactly one side initiated (lower 14-bit token prefix), via L2CAP.
    let connects: usize = (0..2)
        .map(|i| {
            w.radio_cmds(i)
                .iter()
                .filter(|c| matches!(c, RadioCommand::OpenLink { .. }))
                .count()
        })
        .sum();
    assert_eq!(connects, 1);
    // At target 2 / max 3 with one link, discovery stays Forming; HELLO accepting.
    assert_eq!(w.nodes[0].discovery.0, DiscoveryMode::Forming);
    let hello = mesh_wire::hello::Hello::decode(&w.nodes[0].hello.unwrap()).unwrap();
    assert!(hello.flags.lab() && hello.flags.accepting());
}

#[test]
fn limit_reached_lowers_the_ble_max_for_ten_minutes() {
    let mut w = World::new(2, Platform::Ios, 7);
    w.start_lab(31);
    let li = w.connect(0, 1);
    w.run_ms(500);
    let limits = w.nodes[0].engine.ctx().cfg.limits;
    let now = w.now;
    assert_eq!(w.nodes[0].engine.ctx().radio.ble_max(&limits, now), 3);
    w.drop_link(li, LinkDownReason::LimitReached);
    let now = w.now;
    assert_eq!(w.nodes[0].engine.ctx().radio.ble_max(&limits, now), 2);
    let later = Instant(now.0 + 600_000_001);
    assert_eq!(w.nodes[0].engine.ctx().radio.ble_max(&limits, later), 3);
}

#[test]
fn same_seeds_give_identical_wire_bytes() {
    let run = |salt: u8| {
        let mut w = World::new(2, Platform::Ios, salt);
        w.start_lab(4242);
        w.connect(0, 1);
        w.run_ms(5_000);
        (w.nodes[0].tx_log.clone(), w.nodes[1].effects.clone())
    };
    let (a1, e1) = run(9);
    let (a2, e2) = run(9);
    assert_eq!(a1, a2);
    assert_eq!(e1, e2);
    let (a3, _) = run(10);
    assert_ne!(a1, a3, "a different seed draws other ids and tokens");
}
