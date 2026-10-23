//! Records (DESIGN §4.4) and LinkFrames (§4.3): round trips for every record type,
//! malformed input never panics, bounds (32 records, LEB128 forms), LAB padding.

use mesh_wire::frame::{
    KpCtr, LAB_PAD_BODY, LfType, LinkFrame, RejectReason, SEC_OVERHEAD, encode_lab_padded, lab_tag,
    sec_frame_len,
};
use mesh_wire::mesh::segment::{seg_byte, split_seg_byte};
use mesh_wire::record::{
    Caps, Close, IdProof, LabCtl, LabCtlOp, LabGps, Lecho, Lprobe, Lstat, MAX_RECORDS_PER_FRAME,
    Nbr, NbrEntry, NbrFlags, Ping, Pong, Record, RecordIter, RecordWriter, Sync, SyncKind, Upgrade,
    UpgradeOp, UpgradeTransport, encode_records, rtype,
};
use proptest::prelude::*;

fn mesh_body() -> impl Strategy<Value = Vec<u8>> {
    (any::<[u8; 9]>(), prop::collection::vec(any::<u8>(), 0..64)).prop_map(|(mut h, p)| {
        h[0] = (1 << 5) | (h[0] & 0x1F);
        h[4] |= 1;
        let mut v = h.to_vec();
        v.extend(p);
        v
    })
}

/// Owned inputs for one record; borrowed parts live in the tuple.
#[derive(Debug, Clone)]
enum Spec {
    Fixed(u8, Vec<u8>),
}

fn reject_reason(close: bool) -> impl Strategy<Value = RejectReason> {
    (1u8..=if close { 10 } else { 8 }).prop_map(move |v| RejectReason::from_u8(v, close).unwrap())
}

/// Encoded bodies of every record type, built from typed values.
fn any_record_bytes() -> impl Strategy<Value = Spec> {
    let nbr = (
        any::<u8>(),
        prop::collection::vec(
            (
                any::<u16>(),
                any::<u8>(),
                any::<bool>(),
                any::<bool>(),
                0u8..3,
                any::<u8>(),
            ),
            0..=8,
        ),
    )
        .prop_map(|(nseq, es)| {
            let list: Vec<NbrEntry> = es
                .into_iter()
                .map(|(node_id, q, g, c, k, vcap)| NbrEntry {
                    node_id,
                    q,
                    flags: NbrFlags::new(g, c, k),
                    vcap,
                })
                .collect();
            Record::Nbr(Nbr::new(nseq, &list).unwrap())
                .encode()
                .unwrap()
        });
    let caps = (
        any::<[u8; 4]>(),
        any::<u16>(),
        any::<u16>(),
        any::<[u32; 3]>(),
    )
        .prop_map(|(b, app_ver, codec, v)| {
            Record::Caps(Caps {
                proto: b[0],
                platform: b[1],
                app_ver,
                codec,
                transports: b[2],
                max_links: b[3],
                role: b[0] & 1,
                epoch: v[0],
                roster_version: v[1],
                ctl_version: v[2],
            })
            .encode()
            .unwrap()
        });
    let simple = prop_oneof![
        (any::<u16>(), any::<u32>())
            .prop_map(|(id, t_tx)| Record::Ping(Ping { id, t_tx }).encode().unwrap()),
        (any::<u16>(), any::<u32>(), any::<u16>()).prop_map(|(id, t_tx, hold_us)| Record::Pong(
            Pong { id, t_tx, hold_us }
        )
        .encode()
        .unwrap()),
        (
            any::<u16>(),
            any::<u32>(),
            any::<u16>(),
            any::<u16>(),
            any::<u8>()
        )
            .prop_map(|(a, b, c, d, e)| {
                Record::Lstat(Lstat {
                    rx_frames: a,
                    rx_bytes: b,
                    gaps: c,
                    pace_kbps: d,
                    queue_ms: e,
                })
                .encode()
                .unwrap()
            }),
        (any::<u32>(), any::<u64>(), any::<u8>(), 0u16..300).prop_map(
            |(probe_id, t_tx, flags, pad_len)| {
                Record::Lprobe(Lprobe {
                    probe_id,
                    t_tx,
                    flags,
                    pad_len,
                })
                .encode()
                .unwrap()
            }
        ),
        (any::<u32>(), any::<u64>(), any::<u32>(), any::<u32>()).prop_map(
            |(probe_id, t_tx, hold_us, rx_count)| {
                Record::Lecho(Lecho {
                    probe_id,
                    t_tx,
                    hold_us,
                    rx_count,
                })
                .encode()
                .unwrap()
            }
        ),
        reject_reason(true).prop_map(|reason| Record::Close(Close { reason }).encode().unwrap()),
        (
            any::<[u8; 32]>(),
            any::<[u8; 32]>(),
            any::<[u8; 32]>(),
            any::<[u8; 32]>()
        )
            .prop_map(|(a, b, c, d)| {
                let mut sig = [0u8; 64];
                sig[..32].copy_from_slice(&c);
                sig[32..].copy_from_slice(&d);
                Record::IdProof(IdProof {
                    ed_pk: a,
                    x_pk: b,
                    sig,
                })
                .encode()
                .unwrap()
            }),
        (any::<u32>(), any::<i32>(), any::<i32>(), any::<[u16; 3]>()).prop_map(
            |(t_ms, lat_e7, lon_e7, s)| {
                Record::LabGps(LabGps {
                    t_ms,
                    lat_e7,
                    lon_e7,
                    speed_cms: s[0],
                    acc_dm: s[1],
                    heading_cdeg: s[2],
                })
                .encode()
                .unwrap()
            }
        ),
        (0u16..2000).prop_map(|n| Record::Pad(n).encode().unwrap()),
    ];
    let borrowed = prop_oneof![
        (
            1u8..=5,
            1u8..=3,
            any::<u32>(),
            prop::collection::vec(any::<u8>(), 0..40)
        )
            .prop_map(|(op, t, nonce, params)| {
                let u = Upgrade {
                    op: UpgradeOp::from_u8(op).unwrap(),
                    transport: UpgradeTransport::from_u8(t).unwrap(),
                    nonce,
                    params: &params,
                };
                Record::Upgrade(u).encode().unwrap()
            }),
        (
            1u8..=4,
            any::<u32>(),
            any::<u8>(),
            prop::collection::vec(any::<u8>(), 0..40)
        )
            .prop_map(|(op, run_id, bench, params)| {
                Record::LabCtl(LabCtl {
                    op: LabCtlOp::from_u8(op).unwrap(),
                    run_id,
                    bench,
                    params: &params,
                })
                .encode()
                .unwrap()
            }),
        (any::<u16>(), any::<u64>(), any::<u32>()).prop_map(|(n, nonce, run_id)| {
            let p = LabCtl::announce_params(n, nonce);
            Record::LabCtl(LabCtl {
                op: LabCtlOp::Announce,
                run_id,
                bench: 0,
                params: &p,
            })
            .encode()
            .unwrap()
        }),
        (
            1u8..=4,
            any::<u16>(),
            1u8..=16,
            prop::collection::vec(any::<u8>(), 0..=900)
        )
            .prop_map(|(k, msg_id, count, chunk)| {
                let idx = (msg_id % u16::from(count)) as u8;
                let s = Sync {
                    kind: SyncKind::from_u8(k).unwrap(),
                    msg_id,
                    idx,
                    count,
                    chunk: &chunk,
                };
                Record::Sync(s).encode().unwrap()
            }),
        mesh_body().prop_map(|b| Record::Mesh(&b).encode().unwrap()),
        (0x10u8..=0xFF, prop::collection::vec(any::<u8>(), 0..300))
            .prop_map(|(t, b)| Record::Unknown { rtype: t, body: &b }.encode().unwrap()),
    ];
    prop_oneof![nbr, caps, simple, borrowed].prop_map(|b| Spec::Fixed(b[0], b))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// decode(encode(x)) re-encodes to the same bytes for every record type.
    #[test]
    fn record_round_trip(spec in any_record_bytes()) {
        let Spec::Fixed(rt, bytes) = spec;
        let recs = RecordIter::collect_all(&bytes).unwrap();
        prop_assert_eq!(recs.len(), 1);
        prop_assert_eq!(recs[0].rtype(), rt);
        prop_assert_eq!(recs[0].encode().unwrap(), bytes);
    }

    /// Several records coalesce into one plaintext and come back in order.
    #[test]
    fn coalesced_round_trip(specs in prop::collection::vec(any_record_bytes(), 1..=MAX_RECORDS_PER_FRAME)) {
        let mut plain = Vec::new();
        for Spec::Fixed(_, b) in &specs {
            plain.extend_from_slice(b);
        }
        let recs = RecordIter::collect_all(&plain).unwrap();
        prop_assert_eq!(recs.len(), specs.len());
        let mut w = RecordWriter::with_capacity_limit(usize::MAX);
        for r in &recs {
            w.push(r).unwrap();
        }
        prop_assert_eq!(w.into_bytes(), plain);
    }

    #[test]
    fn records_garbage_never_panics(b in prop::collection::vec(any::<u8>(), 0..600)) {
        for rec in RecordIter::new(&b).flatten() {
            let _ = rec.encode();
        }
        for t in 0u8..=0x10 {
            let _ = Record::decode(t, &b);
        }
    }

    #[test]
    fn link_frame_garbage_never_panics(b in prop::collection::vec(any::<u8>(), 0..1300)) {
        if let Ok(f) = LinkFrame::decode(&b) {
            prop_assert_eq!(f.encode().unwrap(), b);
        }
    }

    #[test]
    fn kp_ctr_layout(phase in any::<bool>(), ctr in 0u32..=0x7FFF_FFFF) {
        let k = KpCtr::new(phase, ctr);
        prop_assert_eq!(k.phase(), phase);
        prop_assert_eq!(k.ctr(), ctr);
        let aad = k.aad();
        prop_assert_eq!(aad[0], LfType::Sec as u8);
        prop_assert_eq!(&aad[1..], &k.0.to_le_bytes());
        let n = k.nonce();
        prop_assert_eq!(&n[..4], &[0u8; 4]);
        prop_assert_eq!(u64::from_le_bytes(n[4..].try_into().unwrap()), u64::from(k.0));
    }

    #[test]
    fn seg_byte_round_trip(count in 1u8..=16, idx in 0u8..16) {
        prop_assume!(idx < count);
        prop_assert_eq!(split_seg_byte(seg_byte(idx, count).unwrap()).unwrap(), (idx, count));
    }
}

#[test]
fn at_most_32_records() {
    let one = Record::Ping(Ping { id: 1, t_tx: 2 }).encode().unwrap();
    let plain: Vec<u8> = one.repeat(33);
    let mut it = RecordIter::new(&plain);
    for _ in 0..32 {
        assert!(it.next().unwrap().is_ok());
    }
    assert_eq!(it.next(), Some(Err(mesh_wire::WireError::TooManyRecords)));
    assert_eq!(it.next(), None);
    let mut w = RecordWriter::new();
    for _ in 0..32 {
        w.push(&Record::Ping(Ping { id: 1, t_tx: 2 })).unwrap();
    }
    assert!(w.push(&Record::Ping(Ping { id: 1, t_tx: 2 })).is_err());
}

#[test]
fn leb128_forms() {
    let pad = Record::Pad(200).encode().unwrap();
    assert_eq!(&pad[..3], &[rtype::PAD, 0xC8, 0x01]);
    assert_eq!(Record::Pad(127).encode().unwrap()[1], 127);
    // Non-shortest two-byte form, a third byte, and a value above 16383 are refused.
    assert!(RecordIter::collect_all(&[rtype::PAD, 0x85, 0x00]).is_err());
    assert!(RecordIter::collect_all(&[rtype::PAD, 0x80, 0x80, 0x01]).is_err());
    assert!(
        Record::Unknown {
            rtype: 0x40,
            body: &vec![0; 16384]
        }
        .encode()
        .is_err()
    );
    assert!(RecordIter::collect_all(&[rtype::PING, 6, 1, 2]).is_err());
}

#[test]
fn fixed_sizes_and_strictness() {
    let sizes = [
        (Record::Ping(Ping { id: 0, t_tx: 0 }), 6),
        (
            Record::Pong(Pong {
                id: 0,
                t_tx: 0,
                hold_us: 0,
            }),
            8,
        ),
        (
            Record::Lstat(Lstat {
                rx_frames: 0,
                rx_bytes: 0,
                gaps: 0,
                pace_kbps: 0,
                queue_ms: 0,
            }),
            11,
        ),
        (
            Record::Lecho(Lecho {
                probe_id: 0,
                t_tx: 0,
                hold_us: 0,
                rx_count: 0,
            }),
            20,
        ),
        (
            Record::LabGps(LabGps {
                t_ms: 0,
                lat_e7: 0,
                lon_e7: 0,
                speed_cms: 0,
                acc_dm: 0,
                heading_cdeg: 0,
            }),
            18,
        ),
        (
            Record::IdProof(IdProof {
                ed_pk: [0; 32],
                x_pk: [0; 32],
                sig: [0; 64],
            }),
            128,
        ),
        (
            Record::Nbr(Nbr::new(0, &[NbrEntry::default(); 4]).unwrap()),
            23,
        ),
    ];
    for (r, n) in sizes {
        let body = r.encode_body().unwrap();
        assert_eq!(body.len(), n, "{r:?}");
        let mut longer = body.clone();
        longer.push(0);
        assert!(Record::decode(r.rtype(), &longer).is_err());
        assert!(Record::decode(r.rtype(), &body[..n - 1]).is_err());
    }
    let caps = Caps {
        proto: 1,
        platform: 0,
        app_ver: 1,
        codec: 1,
        transports: 1,
        max_links: 3,
        role: 0,
        epoch: 1,
        roster_version: 1,
        ctl_version: 0,
    };
    assert_eq!(Record::Caps(caps).encode_body().unwrap().len(), Caps::LEN);
    assert!(Nbr::new(0, &[NbrEntry::default(); 9]).is_err());
    assert!(Record::decode(rtype::NBR, &[2, 0, 0]).is_err());
    assert!(Record::decode(rtype::CLOSE, &[11]).is_err());
    assert!(Record::decode(rtype::MESH, &[0x21, 0, 0]).is_err());
    let unknown = encode_records(&[Record::Unknown {
        rtype: 0x7F,
        body: &[1, 2],
    }])
    .unwrap();
    assert!(matches!(
        RecordIter::collect_all(&unknown).unwrap()[0],
        Record::Unknown { rtype: 0x7F, .. }
    ));
}

#[test]
fn lab_frames_match_sec_size() {
    let records =
        encode_records(&[Record::Ping(Ping { id: 1, t_tx: 2 }), Record::Pad(10)]).unwrap();
    let lab = encode_lab_padded(lab_tag(1234), &records).unwrap();
    assert_eq!(lab.len(), sec_frame_len(records.len()));
    assert_eq!(SEC_OVERHEAD, 21);
    assert_eq!(LAB_PAD_BODY, 16);
    let LinkFrame::Lab {
        lab_tag: t,
        records: body,
    } = LinkFrame::decode(&lab).unwrap()
    else {
        panic!("not a LAB frame");
    };
    assert_eq!(t, lab_tag(1234));
    let recs = RecordIter::collect_all(body).unwrap();
    assert_eq!(recs.last(), Some(&Record::Pad(16)));
    assert_ne!(lab_tag(1234), lab_tag(1235));
}

#[test]
fn link_frame_rules() {
    assert!(LinkFrame::decode(&[]).is_err());
    assert!(LinkFrame::decode(&[0x04]).is_err());
    assert!(LinkFrame::decode(&[0x00, 0, 1]).is_err());
    assert!(LinkFrame::decode(&[0x00, 4, 1]).is_err());
    assert!(LinkFrame::decode(&[0x01, 0, 0, 0, 0]).is_err());
    assert!(LinkFrame::decode(&[0x03, 9]).is_err());
    assert!(LinkFrame::decode(&[0x03, 1, 0]).is_err());
    assert!(LinkFrame::decode(&vec![0x02; 1201]).is_err());
    let sec = [vec![0x01, 5, 0, 0, 0x80], vec![0xEE; 16]].concat();
    let LinkFrame::Sec { kp_ctr, sealed } = LinkFrame::decode(&sec).unwrap() else {
        panic!()
    };
    assert!(kp_ctr.phase());
    assert_eq!(kp_ctr.ctr(), 5);
    assert_eq!(sealed.len(), 16);
    assert!(
        LinkFrame::Hs {
            step: 0,
            proto_ver: 1,
            noise_msg: &[]
        }
        .encode()
        .is_err()
    );
    assert!(RejectReason::from_u8(9, false).is_err());
    assert!(RejectReason::from_u8(9, true).is_ok());
}
