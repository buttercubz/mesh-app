//! The global assertions (DESIGN §13.2) catch what they must: a reused `(key, nonce)`,
//! a surviving id collision, a node over its link max, a broken own stream; and stay
//! silent on legal traffic (counter restarts after a key-phase flip, independent
//! links, collisions resolved within the grace period).

use mesh_core::Instant;
use mesh_core::LinkUnit;
use mesh_core::links::framing::Framer;
use mesh_sim::{Audit, ViolationKind};
use mesh_wire::frame::{KpCtr, LinkFrame};

fn sec(phase: bool, ctr: u32) -> Vec<u8> {
    let sealed = [0xA5u8; 24];
    LinkFrame::Sec {
        kp_ctr: KpCtr::new(phase, ctr),
        sealed: &sealed,
    }
    .encode()
    .expect("SEC frame")
}

fn t(ms: u64) -> Instant {
    Instant(ms * 1_000)
}

fn kinds(a: &Audit) -> Vec<ViolationKind> {
    a.violations.iter().map(|v| v.kind).collect()
}

#[test]
fn increasing_counters_per_key_are_legal() {
    let mut a = Audit::new();
    for ctr in 0..50 {
        a.on_air(
            t(ctr.into()),
            0,
            7,
            LinkUnit::Datagram,
            23,
            &sec(false, ctr),
        );
        // Another node and another link reuse the same counters with their own keys.
        a.on_air(
            t(ctr.into()),
            1,
            7,
            LinkUnit::Datagram,
            23,
            &sec(false, ctr),
        );
        a.on_air(
            t(ctr.into()),
            0,
            9,
            LinkUnit::Datagram,
            23,
            &sec(false, ctr),
        );
    }
    // A key update flips the phase: the new key restarts its counter.
    for ctr in 0..10 {
        a.on_air(t(100), 0, 7, LinkUnit::Datagram, 23, &sec(true, ctr));
    }
    assert!(a.violations.is_empty(), "{:?}", a.violations);
    assert_eq!(a.frames.sec, 160);
    assert_eq!(a.keys_seen(), 4);
}

#[test]
fn a_repeated_nonce_is_a_violation() {
    let mut a = Audit::new();
    for ctr in [0, 1, 2, 2, 5, 3] {
        a.on_air(t(1), 0, 4, LinkUnit::Datagram, 23, &sec(false, ctr));
    }
    assert_eq!(kinds(&a), vec![ViolationKind::NonceReuse; 2]);
    // Back to the old phase after an update is a new key generation, not a reuse.
    let mut b = Audit::new();
    b.on_air(t(1), 0, 4, LinkUnit::Datagram, 23, &sec(false, 3));
    b.on_air(t(2), 0, 4, LinkUnit::Datagram, 23, &sec(true, 0));
    b.on_air(t(3), 0, 4, LinkUnit::Datagram, 23, &sec(false, 0));
    assert!(b.violations.is_empty(), "{:?}", b.violations);
}

#[test]
fn stream_and_gatt_units_are_reassembled_before_the_check() {
    let mut a = Audit::new();
    let mut stream = Framer::new(LinkUnit::Stream, None);
    let mut gatt = Framer::new(LinkUnit::Message, Some(23));
    for ctr in [10, 11, 11] {
        let f = sec(false, ctr);
        // The stream path is fed in odd-sized chunks, as the OS may split it.
        let unit = stream.outbound(&f).expect("stream unit").concat();
        for chunk in unit.chunks(5) {
            a.on_air(t(5), 2, 1, LinkUnit::Stream, 23, chunk);
        }
        for unit in gatt.outbound(&f).expect("gatt units") {
            a.on_air(t(5), 3, 1, LinkUnit::Message, 23, &unit);
        }
    }
    assert_eq!(kinds(&a), vec![ViolationKind::NonceReuse; 2]);
    assert_eq!(a.frames.sec, 6);
    // A link dropped mid-frame: its partial framer state is forgotten, so the next
    // unit on that tap parses from a clean boundary.
    let unit = stream.outbound(&sec(false, 12)).unwrap().concat();
    a.on_air(t(6), 2, 1, LinkUnit::Stream, 23, &unit[..4]);
    a.forget_link(2, 1);
    a.on_air(
        t(7),
        2,
        1,
        LinkUnit::Stream,
        23,
        &stream.outbound(&sec(false, 13)).unwrap()[0],
    );
    assert_eq!(a.violations.len(), 2, "{:?}", a.violations);
    assert_eq!(a.frames.sec, 7);
}

#[test]
fn lab_and_handshake_frames_are_counted_and_garbage_is_flagged() {
    let mut a = Audit::new();
    let lab =
        mesh_wire::frame::encode_lab_padded(mesh_wire::frame::lab_tag(2710), &[1, 2, 3]).unwrap();
    a.on_air(t(1), 0, 1, LinkUnit::Datagram, 23, &lab);
    let hs = LinkFrame::Hs {
        step: 1,
        proto_ver: 1,
        noise_msg: &[0u8; 32],
    }
    .encode()
    .unwrap();
    a.on_air(t(1), 0, 1, LinkUnit::Datagram, 23, &hs);
    assert_eq!((a.frames.lab, a.frames.hs), (1, 1));
    assert!(a.violations.is_empty());
    // A stream whose length prefix is impossible: our own stream is broken.
    a.on_air(t(2), 0, 2, LinkUnit::Stream, 23, &[0xFF, 0xFF, 0x01]);
    // An undecodable frame type inside a valid datagram.
    a.on_air(t(2), 0, 3, LinkUnit::Datagram, 23, &[0x7E, 0, 0, 0, 0]);
    let k = kinds(&a);
    assert!(
        k.iter().all(|k| *k == ViolationKind::MalformedOwnFrame),
        "{k:?}"
    );
    assert!(!k.is_empty());
}

#[test]
fn id_collisions_and_link_caps_fail_only_after_their_grace() {
    let mut a = Audit::new();
    let grace = 5_000_000;
    let pair = [(42u16, 1usize, 4usize)];
    a.check_ids(t(0), &pair, grace);
    a.check_ids(t(4_000), &pair, grace);
    // Resolved in time: the episode restarts from zero.
    a.check_ids(t(4_500), &[], grace);
    a.check_ids(t(5_000), &pair, grace);
    a.check_ids(t(9_900), &pair, grace);
    assert!(a.violations.is_empty());
    a.check_ids(t(10_100), &pair, grace);
    a.check_ids(t(20_000), &pair, grace);
    assert_eq!(
        kinds(&a),
        vec![ViolationKind::IdCollision],
        "reported once per episode"
    );

    let mut c = Audit::new();
    c.check_link_cap(t(0), 3, 4, 3, true, Some(1_500_000));
    c.check_link_cap(t(1_000), 3, 4, 3, true, Some(1_500_000));
    c.check_link_cap(t(1_200), 3, 3, 3, true, Some(1_500_000));
    c.check_link_cap(t(2_000), 3, 4, 3, true, Some(1_500_000));
    assert!(c.violations.is_empty());
    c.check_link_cap(t(3_600), 3, 4, 3, true, Some(1_500_000));
    assert_eq!(kinds(&c), vec![ViolationKind::LinkCap]);
    assert_eq!(c.over_max_episodes, 2);
    // Report-only: episodes are counted, never a violation.
    let mut r = Audit::new();
    r.check_link_cap(t(0), 1, 4, 3, true, None);
    r.check_link_cap(t(60_000), 1, 4, 3, true, None);
    assert!(r.violations.is_empty());
    assert_eq!(r.over_max_episodes, 1);
    // Over the max with no redundant link (§5.7 may close none): no deadline; the
    // grace starts when a redundant link appears.
    let mut u = Audit::new();
    u.check_link_cap(t(0), 2, 4, 3, false, Some(1_500_000));
    u.check_link_cap(t(30_000), 2, 4, 3, false, Some(1_500_000));
    assert!(u.violations.is_empty());
    u.check_link_cap(t(31_000), 2, 4, 3, true, Some(1_500_000));
    u.check_link_cap(t(32_000), 2, 4, 3, true, Some(1_500_000));
    assert!(u.violations.is_empty());
    u.check_link_cap(t(32_600), 2, 4, 3, true, Some(1_500_000));
    assert_eq!(kinds(&u), vec![ViolationKind::LinkCap]);
    assert_eq!(u.over_max_episodes, 1);

    let mut s = Audit::new();
    s.observe_streams(t(0), 0, 1, true, 2, 2);
    s.observe_streams(t(0), 0, 1, true, 3, 2);
    assert_eq!(kinds(&s), vec![ViolationKind::StreamCap]);
    assert_eq!(s.max_streams, (3, 0));
}
