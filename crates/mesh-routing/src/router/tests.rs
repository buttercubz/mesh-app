use mesh_wire::mesh::{
    EncoderProfile, FloorRejectReason, MeshHeader, MeshPacket, PacketType, Presence, Talk,
    TalkMode, TalkPrio, TalkState, Voice, VoiceCfg,
};
use mesh_wire::record::{Nbr, NbrEntry, NbrFlags};

use super::*;
use crate::types::{LinkKind, LinkRole};

fn cfg(me: NodeId) -> RouterConfig {
    RouterConfig {
        me,
        sess: 1,
        initial_seq: 100,
        epoch: 1,
        limits: Limits::V1,
        multi_hop: true,
        rng_seed: [7; 32],
    }
}

/// Router `me` with links 1→node 2, 2→node 3, 3→node 4.
fn router(me: NodeId) -> Router {
    let mut r = Router::new(cfg(me));
    for (l, n) in [(1u32, 2u16), (2, 3), (3, 4)] {
        r.neighbors
            .link_up(
                Instant::ZERO,
                n,
                LinkId(l),
                LinkKind::Ble,
                LinkRole::Initiator,
                1024,
            )
            .expect("up");
    }
    r
}

fn pkt(ptype: PacketType, origin: u16, hops: u8, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut h = MeshHeader::new(ptype, origin, 1, seq);
    h.hops = hops;
    MeshPacket { header: h, payload }.encode().expect("encode")
}

fn voice_payload(burst: u8, ts: u16) -> Vec<u8> {
    Voice {
        burst,
        ts,
        cfg: VoiceCfg(0x80),
        opus: &[1u8; 80],
    }
    .encode()
    .expect("voice")
}

fn presence_payload() -> Vec<u8> {
    Presence::default().encode()
}

fn auth(b: &[u8]) -> Authenticated<'_> {
    Authenticated::after_verified_frame(b)
}

fn links(o: &RouteOutcome) -> Vec<u32> {
    o.out.iter().map(|x| x.link.0).collect()
}

#[test]
fn voice_relays_immediately_except_to_sender_and_origin() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    let b = pkt(PacketType::Voice, 2, 0, 5, &voice_payload(1, 0));
    let o = r.on_mesh_packet(t, LinkId(1), auth(&b));
    assert!(o.deliver.is_some());
    assert_eq!(links(&o), vec![2, 3]);
    let q = &o.out[0].packet;
    assert_eq!((q.header.ttl, q.header.hops), (3, 1));
    assert_eq!(o.out[0].prio, Priority::P1);
    // Duplicate: nothing.
    let o = r.on_mesh_packet(
        t,
        LinkId(2),
        auth(&pkt(PacketType::Voice, 2, 1, 5, &voice_payload(1, 0))),
    );
    assert!(o.deliver.is_none() && o.out.is_empty());
    assert_eq!(r.stats.dup, 1);
}

#[test]
fn first_hop_origin_check_and_malformed_penalties() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    // hops 0 but origin 9 ≠ sender 2: +5 each.
    for s in 0..3 {
        let b = pkt(PacketType::Presence, 9, 0, s, &presence_payload());
        let o = r.on_mesh_packet(t, LinkId(1), auth(&b));
        assert!(o.deliver.is_none());
        assert!(o.actions.is_empty());
    }
    assert_eq!(r.stats.spoofed, 3);
    // Dedup state unchanged by dropped input.
    assert!(!r.dedup.seen(t, 9, 1, 0));
    // Malformed: +1 (16 points).
    let o = r.on_mesh_packet(t, LinkId(1), auth(&[0xFF, 0x00]));
    assert!(o.actions.is_empty());
    assert_eq!(r.stats.malformed, 1);
    // The fourth spoof reaches 21 ≥ 20 within a minute: disconnect and ban.
    let b = pkt(PacketType::Presence, 9, 0, 9, &presence_payload());
    let o = r.on_mesh_packet(t, LinkId(1), auth(&b));
    assert!(matches!(
        o.actions[..],
        [RouterAction::Disconnect { node: 2, .. }]
    ));
    assert!(r.neighbors.banned_until(t, 2).is_some());
    // Unknown links are ignored entirely.
    let o = r.on_mesh_packet(
        t,
        LinkId(99),
        auth(&pkt(PacketType::Presence, 2, 0, 1, &presence_payload())),
    );
    assert_eq!(o, RouteOutcome::default());
}

#[test]
fn ttl_age_unknown_and_direct_delivery() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    // TTL 1: delivered, not forwarded.
    let mut h = MeshHeader::new(PacketType::Presence, 2, 1, 1);
    h.ttl = 1;
    let b = MeshPacket {
        header: h,
        payload: &presence_payload(),
    }
    .encode()
    .expect("e");
    let o = r.on_mesh_packet(t, LinkId(1), auth(&b));
    assert!(o.deliver.is_some());
    assert!(r.poll(t.plus_ms(30)).is_empty());
    // Stale voice (age 152 ms): delivered, not relayed.
    let mut h = MeshHeader::new(PacketType::Voice, 2, 1, 2);
    h.age = 76;
    let b = MeshPacket {
        header: h,
        payload: &voice_payload(1, 0),
    }
    .encode()
    .expect("e");
    let o = r.on_mesh_packet(t, LinkId(1), auth(&b));
    assert!(o.deliver.is_some() && o.out.is_empty());
    assert_eq!(r.stats.stale, 1);
    // Unknown ptype: relayed, not delivered.
    let b = pkt(PacketType::Unknown(0x15), 2, 0, 3, &[1, 2, 3]);
    let o = r.on_mesh_packet(t, LinkId(1), auth(&b));
    assert!(o.deliver.is_none());
    assert_eq!(r.poll(t.plus_ms(30)).len(), 2);
    // Phase 1 direct delivery: no relaying at all.
    let mut d = Router::new(RouterConfig {
        multi_hop: false,
        ..cfg(1)
    });
    d.neighbors
        .link_up(t, 2, LinkId(1), LinkKind::Ble, LinkRole::Initiator, 1024)
        .expect("up");
    d.neighbors
        .link_up(t, 3, LinkId(2), LinkKind::Ble, LinkRole::Initiator, 1024)
        .expect("up");
    let o = d.on_mesh_packet(
        t,
        LinkId(1),
        auth(&pkt(PacketType::Voice, 2, 0, 1, &voice_payload(1, 0))),
    );
    assert!(o.deliver.is_some() && o.out.is_empty());
}

#[test]
fn control_jitter_pruning_and_counter_suppression() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    // Neighbor 2 reports 3 as good and uncongested: 3 is pruned for ctrl.
    let nbr = Nbr::new(
        0,
        &[NbrEntry {
            node_id: 3,
            q: 90,
            flags: NbrFlags::new(true, false, 0),
            vcap: 2,
        }],
    )
    .expect("nbr");
    r.neighbors.on_nbr(t, LinkId(1), &nbr);
    let b = pkt(PacketType::Presence, 2, 0, 1, &presence_payload());
    let o = r.on_mesh_packet(t, LinkId(1), auth(&b));
    assert!(o.out.is_empty(), "control waits for its jitter");
    assert!(r.poll(t.plus_ms(4)).is_empty());
    let rel = r.poll(t.plus_ms(25));
    assert_eq!(rel.iter().map(|x| x.link.0).collect::<Vec<_>>(), vec![3]);
    assert_eq!(r.stats.pruned, 1);
    // Voice is never pruned.
    let o = r.on_mesh_packet(
        t,
        LinkId(1),
        auth(&pkt(PacketType::Voice, 2, 0, 2, &voice_payload(1, 0))),
    );
    assert_eq!(links(&o), vec![2, 3]);
    // Counter suppression: heard from 3 distinct neighbors before the send.
    let mut r = router(1);
    let b = pkt(PacketType::Presence, 9, 2, 1, &presence_payload());
    r.on_mesh_packet(t, LinkId(1), auth(&b));
    r.on_mesh_packet(t.plus_ms(1), LinkId(2), auth(&b));
    r.on_mesh_packet(t.plus_ms(2), LinkId(3), auth(&b));
    assert!(r.poll(t.plus_ms(30)).is_empty());
    assert_eq!(r.stats.suppressed, 2);
}

#[test]
fn origination_goes_everywhere_and_updates_my_floor() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    let talk = Talk {
        burst: 3,
        state: TalkState::Start,
        prio: TalkPrio::Member,
        mode: TalkMode::Ptt,
        profile: EncoderProfile::Ble16,
        elapsed_10ms: 0,
    };
    let o = r.originate(t, PacketType::Talk, false, talk.encode());
    assert_eq!(links(&o), vec![1, 2, 3]);
    assert_eq!(o.out[0].prio, Priority::P0);
    assert!(r.floor.get(BurstKey::new(1, 1, 3)).is_some());
    let o = r.originate(t, PacketType::Voice, false, voice_payload(3, 0));
    assert_eq!(links(&o), vec![1, 2, 3]);
    assert_eq!(o.out[0].packet.header.ttl, 4);
    assert!(r.neighbors.link(LinkId(1)).expect("l").is_voice_link(t));
    // An echo of my own packet is a duplicate.
    let seq = o.out[0].packet.header.seq;
    let mut h = o.out[0].packet.header;
    h.hops = 1;
    let echo = MeshPacket {
        header: h,
        payload: &voice_payload(3, 0),
    }
    .encode()
    .expect("e");
    assert!(
        r.on_mesh_packet(t, LinkId(2), auth(&echo))
            .deliver
            .is_none()
    );
    assert_eq!(r.stats.dup, 1);
    assert_eq!(seq, 101);
    // Segmented bodies: one packet (and seq) per segment, to every neighbor.
    let o = r.originate_segmented(t, PacketType::Roster, true, 9, &[5u8; 2_000]);
    assert_eq!(o.out.len(), 9);
    assert!(
        o.out
            .iter()
            .all(|x| x.packet.header.seg && x.packet.header.leader)
    );
}

#[test]
fn link_cap_admission_and_rejects() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    // Link 2 (to node 3) only carries one stream.
    r.set_link_cap(LinkId(2), 1);
    assert_eq!(r.link_cap(LinkId(2)), 1);
    assert_eq!(r.cap_local(), 1);
    let a = pkt(PacketType::Voice, 2, 0, 1, &voice_payload(1, 0));
    let o = r.on_mesh_packet(t, LinkId(1), auth(&a));
    assert_eq!(links(&o), vec![2, 3]);
    // A later burst from node 4 (hops 1 via 4) is refused on link 2 → FLOOR_REJECT{CONGESTION}.
    let mut h = MeshHeader::new(PacketType::Voice, 9, 1, 1);
    h.hops = 2;
    let b = MeshPacket {
        header: h,
        payload: &voice_payload(1, 0),
    }
    .encode()
    .expect("e");
    let o = r.on_mesh_packet(t.plus_ms(500), LinkId(3), auth(&b));
    let rejects: Vec<_> = o
        .out
        .iter()
        .filter(|x| x.packet.header.ptype == PacketType::FloorReject)
        .collect();
    assert_eq!(rejects.len(), 3);
    // The doomed frame goes nowhere, not even on the links that had room.
    assert!(
        !o.out
            .iter()
            .any(|x| x.packet.header.ptype == PacketType::Voice)
    );
    let fr = mesh_wire::mesh::FloorReject::decode(&rejects[0].packet.payload).expect("fr");
    assert_eq!(
        (fr.target, fr.reason, fr.at),
        (9, FloorRejectReason::Congestion, 1)
    );
    // The rejected burst is tombstoned: later frames go nowhere.
    let mut h2 = h;
    h2.seq = 2;
    let b2 = MeshPacket {
        header: h2,
        payload: &voice_payload(1, 4),
    }
    .encode()
    .expect("e");
    let o = r.on_mesh_packet(t.plus_ms(540), LinkId(3), auth(&b2));
    assert!(o.out.is_empty());
}

#[test]
fn rejected_me_and_path_hint_actions() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    let fr = mesh_wire::mesh::FloorReject {
        target: 1,
        target_sess: 1,
        burst: 7,
        reason: FloorRejectReason::Cap,
        at: 3,
    };
    let o = r.on_mesh_packet(
        t,
        LinkId(2),
        auth(&pkt(PacketType::FloorReject, 3, 0, 1, &fr.encode())),
    );
    assert!(o.actions.contains(&RouterAction::RejectedMe {
        burst: 7,
        reason: FloorRejectReason::Cap
    }));
    let hint = mesh_wire::mesh::PathHint {
        profile: EncoderProfile::Ble12,
        at: 3,
        target: 0,
    };
    let o = r.on_mesh_packet(
        t,
        LinkId(2),
        auth(&pkt(
            PacketType::PathHint,
            3,
            0,
            2,
            &hint.encode().expect("h"),
        )),
    );
    assert!(o.actions.contains(&RouterAction::PathHint {
        profile: EncoderProfile::Ble12,
        from: 3
    }));
    // Our own hint goes to every neighbor with TTL 3.
    let o = r.originate_path_hint(t);
    assert_eq!(links(&o), vec![1, 2, 3]);
    assert_eq!(o.out[0].packet.header.ttl, 3);
}

#[test]
fn near_talker_rejection() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    // Two older bursts fill the global top 2; a third arrives from a direct neighbor.
    r.on_mesh_packet(
        t,
        LinkId(1),
        auth(&pkt(PacketType::Voice, 2, 0, 1, &voice_payload(1, 300))),
    );
    r.on_mesh_packet(
        t,
        LinkId(2),
        auth(&pkt(PacketType::Voice, 3, 0, 1, &voice_payload(1, 200))),
    );
    let o = r.on_mesh_packet(
        t,
        LinkId(3),
        auth(&pkt(PacketType::Voice, 4, 0, 1, &voice_payload(1, 0))),
    );
    let rej: Vec<_> = o
        .out
        .iter()
        .filter(|x| x.packet.header.ptype == PacketType::FloorReject)
        .collect();
    assert!(!rej.is_empty());
    assert_eq!(rej[0].packet.header.ttl, 4);
    let fr = mesh_wire::mesh::FloorReject::decode(&rej[0].packet.payload).expect("fr");
    assert_eq!((fr.target, fr.reason), (4, FloorRejectReason::Outranked));
}

#[test]
fn mesh_probe_trace_is_appended() {
    let mut r = router(1);
    let t = Instant::from_secs(1);
    let probe = mesh_wire::mesh::MeshProbe::new(1, 2);
    let o = r.on_mesh_packet(
        t,
        LinkId(1),
        auth(&pkt(
            PacketType::MeshProbe,
            2,
            0,
            1,
            &probe.encode().expect("p"),
        )),
    );
    assert_eq!(links(&o), vec![2, 3]);
    let p = mesh_wire::mesh::MeshProbe::decode(&o.out[0].packet.payload).expect("p");
    assert_eq!(p.trace(), &[1]);
}

/// Hybrid node 1: Aware + BLE (fallback) to node 2, Aware to node 3, BLE only to node 4
/// (a BLE-only member hanging off the Wi-Fi backbone).
fn hybrid_router() -> Router {
    let mut r = Router::new(cfg(1));
    for (l, n, k) in [
        (1u32, 2u16, LinkKind::Ble),
        (2, 2, LinkKind::Aware),
        (3, 3, LinkKind::Aware),
        (4, 4, LinkKind::Ble),
    ] {
        r.neighbors
            .link_up(Instant::ZERO, n, LinkId(l), k, LinkRole::Initiator, 1024)
            .expect("up");
    }
    r
}

#[test]
fn hybrid_edges_caps_and_near_talker_per_segment() {
    let mut r = hybrid_router();
    // BLE links of a node holding an Aware link to another neighbor are edges.
    assert!(r.is_edge_link(LinkId(1)));
    assert!(r.is_edge_link(LinkId(4)));
    assert!(!r.is_edge_link(LinkId(2)));
    // The edge does not bound my own floor admission: the Aware cap does.
    assert_eq!(r.cap_local(), 4);
    // Near-talker cap: 4 from the backbone, 2 from a BLE-only neighbor.
    assert_eq!(r.near_talker_cap(LinkId(2)), 4);
    assert_eq!(r.near_talker_cap(LinkId(1)), 4);
    assert_eq!(r.near_talker_cap(LinkId(4)), 2);
    // A BLE-only neighbor whose NBR lists an Aware link is on the backbone too.
    let nbr = Nbr::new(
        1,
        &[NbrEntry {
            node_id: 9,
            q: 90,
            flags: NbrFlags::new(true, false, NbrFlags::KIND_AWARE),
            vcap: 4,
        }],
    )
    .expect("nbr");
    r.neighbors.on_nbr(Instant::ZERO, LinkId(4), &nbr);
    assert_eq!(r.near_talker_cap(LinkId(4)), 4);
    // A BLE-only node (no Aware link at all) keeps the BLE rules.
    let plain = router(1);
    assert_eq!(plain.cap_local(), 2);
    assert!(!plain.is_edge_link(LinkId(1)));
    assert_eq!(plain.near_talker_cap(LinkId(1)), 2);
}

#[test]
fn full_ble_edge_sheds_there_and_never_rejects_group_wide() {
    let mut r = hybrid_router();
    let t = Instant::from_secs(1);
    // Four island talkers arrive over the Aware link from node 3 (origins 10..13, two
    // hops away), 300 ms apart. Every one is relayed on the Aware link to node 2; the
    // BLE edge to node 4 carries the first two only; nobody gets a FLOOR_REJECT.
    for (k, origin) in (10u16..14).enumerate() {
        let mut h = MeshHeader::new(PacketType::Voice, origin, 1, 1);
        h.hops = 1;
        let b = MeshPacket {
            header: h,
            payload: &voice_payload(1, 0),
        }
        .encode()
        .expect("e");
        let o = r.on_mesh_packet(t.plus_ms(300 * k as u64), LinkId(3), auth(&b));
        assert!(
            !o.out
                .iter()
                .any(|x| x.packet.header.ptype == PacketType::FloorReject),
            "talker {origin} rejected"
        );
        let voice: Vec<u32> = o
            .out
            .iter()
            .filter(|x| x.packet.header.ptype == PacketType::Voice)
            .map(|x| x.link.0)
            .collect();
        // The bundle picks Aware or BLE for node 2 later; the router offers the best link.
        assert!(voice.contains(&2), "talker {origin}: {voice:?}");
        assert_eq!(voice.contains(&4), k < 2, "talker {origin}: {voice:?}");
    }
    assert_eq!(r.admission.admitted(LinkId(4)).len(), 2);
    assert_eq!(r.admission.admitted(LinkId(2)).len(), 4);
    assert_eq!(r.stats.rejects_sent, 0);
}

/// Regression: a leaf of a star-shaped Aware island holds one Aware link and its BLE
/// fallback link to the same hub. That fallback is not an edge (no other Aware
/// neighbor) but carries no voice either, so it must not cap my floor admission at the
/// BLE cap (4 island talkers fit S_MAX_AWARE).
#[test]
fn ble_fallback_to_my_only_aware_peer_does_not_bound_cap_local() {
    let mut r = Router::new(cfg(1));
    for (l, k) in [(1u32, LinkKind::Ble), (2, LinkKind::Aware)] {
        r.neighbors
            .link_up(Instant::ZERO, 2, LinkId(l), k, LinkRole::Initiator, 1024)
            .expect("up");
    }
    assert!(!r.is_edge_link(LinkId(1)));
    assert!(r.is_ble_fallback_link(LinkId(1)));
    assert!(!r.is_ble_fallback_link(LinkId(2)));
    assert_eq!(r.cap_local(), 4);
    // A BLE-only neighbor next to it (an edge) still does not bound it; once the Aware
    // link drops the BLE link carries voice again and the BLE cap applies.
    r.neighbors
        .link_up(
            Instant::ZERO,
            3,
            LinkId(3),
            LinkKind::Ble,
            LinkRole::Initiator,
            1024,
        )
        .expect("up");
    assert!(r.is_edge_link(LinkId(3)));
    assert_eq!(r.cap_local(), 4);
    r.on_link_down(LinkId(2));
    assert!(!r.is_ble_fallback_link(LinkId(1)));
    assert_eq!(r.cap_local(), 2);
}
