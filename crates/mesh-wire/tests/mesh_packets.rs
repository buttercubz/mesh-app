//! MeshPacket header and every ptype (DESIGN §4.5-§4.6): round trips, strict sizes,
//! relay rewrite, malformed input never panics.

use mesh_wire::mesh::{
    EncoderProfile, Ext, FloorReject, FloorRejectReason, GroupCtl, GroupCtlOp, Leave, LeaveReason,
    MeshHeader, MeshPacket, MeshPayload, MeshProbe, PacketType, PathHint, Position, Presence,
    PresenceAudio, PresenceCaps, PresenceLinks, PresenceState, RideCtl, RideCtlOp, Segment, Talk,
    TalkMode, TalkPrio, TalkState, TrafficClass, Voice, VoiceCfg, add_age, encode_packet,
};
use proptest::prelude::*;

fn header() -> impl Strategy<Value = MeshHeader> {
    (
        0u8..32,
        0u8..16,
        0u8..16,
        any::<u16>(),
        1u8..=255,
        any::<u16>(),
        any::<u8>(),
        0u8..4,
        any::<bool>(),
        any::<bool>(),
        0u8..16,
    )
        .prop_map(
            |(pt, ttl, hops, origin, sess, seq, age, class, leader, seg, reserved)| MeshHeader {
                ptype: PacketType::from_bits(pt),
                ttl,
                hops,
                origin,
                sess,
                seq,
                age,
                class: match class {
                    0 => TrafficClass::Ctrl,
                    1 => TrafficClass::Voice,
                    2 => TrafficClass::Bulk,
                    _ => TrafficClass::Lab,
                },
                leader,
                seg,
                reserved,
            },
        )
}

fn sig() -> impl Strategy<Value = [u8; 64]> {
    (any::<[u8; 32]>(), any::<[u8; 32]>()).prop_map(|(a, b)| {
        let mut s = [0u8; 64];
        s[..32].copy_from_slice(&a);
        s[32..].copy_from_slice(&b);
        s
    })
}

/// Encoded payloads of every fixed-layout ptype, paired with their type.
fn payload() -> impl Strategy<Value = (PacketType, Vec<u8>)> {
    let profile = (1u8..=5).prop_map(|p| EncoderProfile::from_u8(p).unwrap());
    prop_oneof![
        (
            any::<u8>(),
            any::<u16>(),
            any::<u8>(),
            prop::collection::vec(any::<u8>(), 1..=400)
        )
            .prop_map(|(burst, ts, cfg, opus)| {
                (
                    PacketType::Voice,
                    Voice {
                        burst,
                        ts,
                        cfg: VoiceCfg(cfg),
                        opus: &opus,
                    }
                    .encode()
                    .unwrap(),
                )
            }),
        (
            any::<u8>(),
            1u8..=3,
            1u8..=2,
            1u8..=2,
            profile.clone(),
            any::<u16>()
        )
            .prop_map(|(burst, s, p, m, profile, e)| {
                let t = Talk {
                    burst,
                    state: TalkState::from_u8(s).unwrap(),
                    prio: TalkPrio::from_u8(p).unwrap(),
                    mode: TalkMode::from_u8(m).unwrap(),
                    profile,
                    elapsed_10ms: e,
                };
                (PacketType::Talk, t.encode())
            }),
        (
            any::<u16>(),
            any::<u8>(),
            any::<u8>(),
            1u8..=4,
            any::<u16>()
        )
            .prop_map(|(target, ts, burst, r, at)| {
                let f = FloorReject {
                    target,
                    target_sess: ts,
                    burst,
                    reason: FloorRejectReason::from_u8(r).unwrap(),
                    at,
                };
                (PacketType::FloorReject, f.encode())
            }),
        any::<[u8; 14]>()
            .prop_map(|b| (PacketType::Presence, Presence::decode(&b).unwrap().encode())),
        (1u8..=3, sig()).prop_map(|(r, sig)| (
            PacketType::Leave,
            Leave {
                reason: LeaveReason::from_u8(r).unwrap(),
                sig
            }
            .encode()
        )),
        (1u8..=2, any::<u32>(), sig()).prop_map(|(op, ride_id, sig)| {
            (
                PacketType::RideCtl,
                RideCtl {
                    op: RideCtlOp::from_u8(op).unwrap(),
                    ride_id,
                    sig,
                }
                .encode(),
            )
        }),
        (
            any::<u32>(),
            any::<u64>(),
            prop::collection::vec(any::<u16>(), 0..=8)
        )
            .prop_map(|(id, t, trace)| {
                let mut p = MeshProbe::new(id, t);
                for n in trace {
                    p.append(n).unwrap();
                }
                (PacketType::MeshProbe, p.encode().unwrap())
            }),
        (1u8..=2, any::<u16>(), any::<u16>()).prop_map(|(pr, at, target)| {
            let h = PathHint {
                profile: EncoderProfile::from_u8(pr).unwrap(),
                at,
                target,
            };
            (PacketType::PathHint, h.encode().unwrap())
        }),
        (
            any::<bool>(),
            any::<u32>(),
            any::<u32>(),
            any::<[u8; 32]>(),
            any::<u16>(),
            sig()
        )
            .prop_map(|(t, cv, ep, pk, node, sig)| {
                let g = GroupCtl {
                    op: if t {
                        GroupCtlOp::TransferLeader
                    } else {
                        GroupCtlOp::Close
                    },
                    ctl_version: cv,
                    epoch: ep,
                    new_leader: t.then_some((pk, node)),
                    sig,
                };
                (PacketType::GroupCtl, g.encode().unwrap())
            }),
        any::<[u8; 14]>()
            .prop_map(|b| (PacketType::Position, Position::decode(&b).unwrap().encode())),
        (any::<u8>(), prop::collection::vec(any::<u8>(), 0..50)).prop_map(|(t, b)| (
            PacketType::Ext,
            Ext {
                ext_type: t,
                body: &b
            }
            .encode()
        )),
        (
            any::<u16>(),
            1u8..=16,
            prop::collection::vec(any::<u8>(), 0..=900)
        )
            .prop_map(|(msg_id, count, data)| {
                let idx = (msg_id % u16::from(count)) as u8;
                (
                    PacketType::Roster,
                    Segment {
                        msg_id,
                        idx,
                        count,
                        data: &data,
                    }
                    .encode()
                    .unwrap(),
                )
            }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn header_round_trip(h in header()) {
        let b = h.encode().unwrap();
        prop_assert_eq!(b.len(), 9);
        prop_assert_eq!(MeshHeader::decode(&b).unwrap(), h);
    }

    /// Every ptype: header + payload → bytes → typed payload → identical bytes.
    #[test]
    fn packet_round_trip(h in header(), (pt, body) in payload()) {
        let mut h = h;
        h.ptype = pt;
        h.seg = pt.is_segmented();
        let bytes = MeshPacket { header: h, payload: &body }.encode().unwrap();
        let pkt = MeshPacket::decode(&bytes).unwrap();
        let typed = MeshPayload::decode(&pkt).unwrap();
        prop_assert_eq!(typed.ptype(), pt);
        prop_assert_eq!(encode_packet(&h, &typed).unwrap(), bytes);
    }

    #[test]
    fn mesh_garbage_never_panics(b in prop::collection::vec(any::<u8>(), 0..1000)) {
        if let Ok(pkt) = MeshPacket::decode(&b)
            && let Ok(p) = MeshPayload::decode(&pkt)
        {
            let _ = p.encode();
        }
        let _ = Voice::decode(&b);
        let _ = Talk::decode(&b);
        let _ = FloorReject::decode(&b);
        let _ = Presence::decode(&b);
        let _ = Leave::decode(&b);
        let _ = RideCtl::decode(&b);
        let _ = GroupCtl::decode(&b);
        let _ = MeshProbe::decode(&b);
        let _ = PathHint::decode(&b);
        let _ = Position::decode(&b);
        let _ = Segment::decode(&b);
        let _ = Ext::decode(&b);
    }

    /// Relays never increase TTL, saturate hops at 15 and age at 255.
    #[test]
    fn relay_rewrite(h in header(), added in 0u64..2_000_000) {
        match h.relayed(added) {
            None => prop_assert_eq!(h.ttl, 0),
            Some(r) => {
                prop_assert_eq!(r.ttl, h.ttl - 1);
                prop_assert_eq!(r.hops, (h.hops + 1).min(15));
                prop_assert!(r.age >= h.age);
                prop_assert_eq!(r.dedup_key(), h.dedup_key());
                prop_assert!(r.encode().is_ok());
            }
        }
    }
}

#[test]
fn header_layout() {
    let mut h = MeshHeader::new(PacketType::Voice, 0x0102, 0x5A, 0x0304);
    assert_eq!((h.ttl, h.class, h.seg), (4, TrafficClass::Voice, false));
    h.age = 7;
    assert_eq!(
        h.encode().unwrap(),
        [0x21, 0x40, 0x02, 0x01, 0x5A, 0x04, 0x03, 0x07, 0x40]
    );
    assert_eq!(
        MeshHeader::new(PacketType::Roster, 1, 1, 1)
            .encode()
            .unwrap()[8],
        0x10
    );
    assert_eq!(
        MeshHeader::new(PacketType::Position, 1, 1, 1).class,
        TrafficClass::Bulk
    );
    assert_eq!(MeshHeader::new(PacketType::MeshProbe, 1, 1, 1).ttl, 8);
    assert_eq!(MeshHeader::new(PacketType::PathHint, 1, 1, 1).ttl, 3);
    assert_eq!(MeshHeader::new(PacketType::Presence, 1, 1, 1).ttl, 15);
    // Version 2 and session 0 are refused.
    assert!(MeshHeader::decode(&[0x41, 0x40, 2, 1, 0x5A, 4, 3, 7, 0x40]).is_err());
    assert!(MeshHeader::decode(&[0x21, 0x40, 2, 1, 0, 4, 3, 7, 0x40]).is_err());
    assert!(MeshHeader::decode(&[0x21, 0x40, 2, 1]).is_err());
    let mut bad = h;
    bad.ttl = 16;
    assert!(bad.encode().is_err());
    // Unknown ptypes still parse (relayed, not delivered).
    let unk = MeshHeader::decode(&[0x35, 0x40, 2, 1, 0x5A, 4, 3, 7, 0x00]).unwrap();
    assert_eq!(unk.ptype, PacketType::Unknown(0x15));
    let pkt = MeshPacket {
        header: unk,
        payload: &[1, 2],
    };
    assert!(matches!(
        MeshPayload::decode(&pkt).unwrap(),
        MeshPayload::Opaque(PacketType::Unknown(0x15), _)
    ));
}

#[test]
fn age_units() {
    assert_eq!(add_age(0, 999), 0);
    assert_eq!(add_age(0, 1_000), 1);
    assert_eq!(add_age(10, 13_000), 17);
    assert_eq!(add_age(250, 1_000_000), 255);
    let h = MeshHeader {
        age: 255,
        ..MeshHeader::new(PacketType::Voice, 1, 1, 1)
    };
    assert_eq!(h.age_us(), 510_000);
    let mut z = h;
    z.ttl = 0;
    assert!(z.relayed(0).is_none());
}

#[test]
fn payload_sizes_match_design() {
    let sig = [0u8; 64];
    assert_eq!(
        Talk {
            burst: 0,
            state: TalkState::Start,
            prio: TalkPrio::Member,
            mode: TalkMode::Vox,
            profile: EncoderProfile::BleDred,
            elapsed_10ms: 0
        }
        .encode()
        .len(),
        7
    );
    assert_eq!(
        FloorReject {
            target: 0,
            target_sess: 1,
            burst: 0,
            reason: FloorRejectReason::Outranked,
            at: 0
        }
        .encode()
        .len(),
        7
    );
    assert_eq!(Presence::default().encode().len(), 14);
    assert_eq!(
        Leave {
            reason: LeaveReason::AppExit,
            sig
        }
        .encode()
        .len(),
        65
    );
    assert_eq!(
        RideCtl {
            op: RideCtlOp::End,
            ride_id: 1,
            sig
        }
        .encode()
        .len(),
        69
    );
    let close = GroupCtl {
        op: GroupCtlOp::Close,
        ctl_version: 1,
        epoch: 1,
        new_leader: None,
        sig,
    };
    assert_eq!(close.encode().unwrap().len(), 73);
    let tr = GroupCtl {
        op: GroupCtlOp::TransferLeader,
        new_leader: Some(([0; 32], 2)),
        ..close
    };
    assert_eq!(tr.encode().unwrap().len(), 107);
    assert!(
        GroupCtl {
            new_leader: None,
            ..tr
        }
        .encode()
        .is_err()
    );
    assert_eq!(Position::default().encode().len(), 14);
    assert_eq!(
        PathHint {
            profile: EncoderProfile::Ble16,
            at: 0,
            target: 0
        }
        .encode()
        .unwrap()
        .len(),
        5
    );
    assert!(
        PathHint {
            profile: EncoderProfile::Wifi20,
            at: 0,
            target: 0
        }
        .encode()
        .is_err()
    );
    let mut p = MeshProbe::new(1, 2);
    for i in 0..8 {
        p.append(i).unwrap();
    }
    assert!(p.append(9).is_err());
    assert_eq!(p.encode().unwrap().len(), 13 + 16);
    assert!(
        Voice {
            burst: 0,
            ts: 0,
            cfg: VoiceCfg(0),
            opus: &[]
        }
        .encode()
        .is_err()
    );
    assert!(
        Voice {
            burst: 0,
            ts: 0,
            cfg: VoiceCfg(0),
            opus: &[0; 401]
        }
        .encode()
        .is_err()
    );
    // TALK profile 5 (BLE-DRED) is valid, 6 is not.
    assert!(Talk::decode(&[0, 1, 1, 1, 5, 0, 0]).is_ok());
    assert!(Talk::decode(&[0, 1, 1, 1, 6, 0, 0]).is_err());
}

#[test]
fn voice_ts_is_burst_relative() {
    use mesh_wire::mesh::FrameDur;
    assert_eq!(Voice::ts_for_frame(0, FrameDur::Ms40), 0);
    assert_eq!(Voice::ts_for_frame(5, FrameDur::Ms40), 20);
    assert_eq!(Voice::ts_for_frame(5, FrameDur::Ms20), 10);
    let cfg = VoiceCfg::new(
        FrameDur::Ms40,
        mesh_wire::mesh::Bandwidth::Wide,
        true,
        false,
        true,
        true,
    );
    assert_eq!(cfg.dur(), FrameDur::Ms40);
    assert_eq!(cfg.bw(), mesh_wire::mesh::Bandwidth::Wide);
    assert!(cfg.lbrr() && cfg.first() && cfg.preroll() && !cfg.dred());
}

#[test]
fn presence_fields() {
    let p = Presence {
        state: PresenceState::new(2, 3, true, true, false, true),
        audio: PresenceAudio::new(3, 6, true, false, true),
        links: PresenceLinks::new(20, 3),
        caps: PresenceCaps::new(1, false, true),
        battery: Presence::BATTERY_UNKNOWN,
        heard: 1 << 31,
        ..Presence::default()
    }
    .with_versions(0x0001_0000, 0xFFFF_FFFF);
    assert_eq!((p.roster_ver16, p.epoch16), (0, 0xFFFF));
    assert_eq!(
        (
            p.state.ride(),
            p.state.talk(),
            p.state.leader(),
            p.state.on_call(),
            p.state.capture_ok(),
            p.state.live()
        ),
        (2, 3, true, true, false, true)
    );
    assert_eq!(
        (
            p.audio.ladder(),
            p.audio.route(),
            p.audio.tx_muted_no_headset(),
            p.audio.render_ok(),
            p.audio.sharing_distance()
        ),
        (3, 6, true, false, true)
    );
    assert_eq!((p.links.ble(), p.links.aware()), (15, 3));
    assert_eq!(
        (
            p.caps.platform(),
            p.caps.dred_ok(),
            p.caps.radio_off_recently()
        ),
        (1, false, true)
    );
    assert!(p.heard_index(31) && !p.heard_index(0) && !p.heard_index(32));
    assert_eq!(Presence::decode(&p.encode()).unwrap(), p);
    // Serial comparison of the truncated fields: 0xFFFF is just behind 0x0000.
    assert!(mesh_wire::serial::serial16_lt(p.epoch16, 0));
}
