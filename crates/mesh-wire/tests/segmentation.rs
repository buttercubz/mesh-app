//! Segmentation, SYNC chunking, the bounded reassembler and control bodies (DESIGN
//! §4.4 SYNC, §4.6.1, §4.7).

use mesh_wire::mesh::segment::{MAX_LOGICAL_BODY, MAX_SEGMENT_DATA, REASSEMBLY_TIMEOUT_US};
use mesh_wire::mesh::{
    AssemblyEvent, ControlBody, JoinReq, MeshHeader, MeshPacket, MeshPayload, PacketType, Rekey,
    RekeyEntry, Roster, RosterEntry, RosterRole, Segment, SegmentAssembler, segment_body,
    segment_packets, sync_chunks,
};
use mesh_wire::record::{Record, RecordIter, SyncKind};
use proptest::prelude::*;

type Origin = (u16, u8);

fn reassemble(asm: &mut SegmentAssembler<Origin>, segs: &[Segment<'_>]) -> Option<Vec<u8>> {
    let mut out = None;
    for s in segs {
        if let AssemblyEvent::Complete(b) = asm.push(0, (1, 1), s).unwrap() {
            out = Some(b);
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Any body up to 16 × 900 B survives segmentation in any arrival order, with
    /// duplicates.
    #[test]
    fn segments_any_order(body in prop::collection::vec(any::<u8>(), 1..=MAX_LOGICAL_BODY), seed in any::<u64>()) {
        let segs = segment_body(9, &body).unwrap();
        prop_assert!(segs.len() <= 16);
        prop_assert!(segs.iter().all(|s| s.data.len() <= MAX_SEGMENT_DATA));
        let mut order: Vec<usize> = (0..segs.len()).collect();
        let mut x = seed | 1;
        for i in (1..order.len()).rev() {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            order.swap(i, (x % (i as u64 + 1)) as usize);
        }
        let mut shuffled: Vec<Segment<'_>> = order.iter().map(|&i| segs[i]).collect();
        // A duplicate of the first arrival (before completion) is ignored.
        shuffled.insert(1, shuffled[0]);
        let mut asm = SegmentAssembler::for_mesh();
        prop_assert_eq!(reassemble(&mut asm, &shuffled), Some(body));
        prop_assert_eq!(asm.in_progress(), 0);
    }

    /// SEG packets decode back into their segments and reassemble.
    #[test]
    fn segment_packets_round_trip(body in prop::collection::vec(any::<u8>(), 1..4000), first_seq in any::<u16>()) {
        let h = MeshHeader::new(PacketType::Rekey, 7, 3, 0);
        let pkts = segment_packets(&h, first_seq, 0xABCD, &body).unwrap();
        let mut asm = SegmentAssembler::for_mesh();
        let mut done = None;
        for (i, p) in pkts.iter().enumerate() {
            let pkt = MeshPacket::decode(p).unwrap();
            prop_assert!(pkt.header.seg);
            prop_assert_eq!(pkt.header.seq, first_seq.wrapping_add(i as u16));
            let MeshPayload::Segment(PacketType::Rekey, s) = MeshPayload::decode(&pkt).unwrap() else {
                return Err(TestCaseError::fail("not a segment"));
            };
            if let AssemblyEvent::Complete(b) = asm.push(0, (7, 3), &s).unwrap() {
                done = Some(b);
            }
        }
        prop_assert_eq!(done, Some(body));
    }

    #[test]
    fn sync_chunks_round_trip(body in prop::collection::vec(any::<u8>(), 1..3000)) {
        let mut asm: SegmentAssembler<SyncKind> = SegmentAssembler::for_sync();
        let mut done = None;
        for s in sync_chunks(SyncKind::Rekey, 5, &body).unwrap() {
            let rec = Record::Sync(s).encode().unwrap();
            let Record::Sync(back) = RecordIter::collect_all(&rec).unwrap()[0] else {
                return Err(TestCaseError::fail("not SYNC"));
            };
            let seg = Segment { msg_id: back.msg_id, idx: back.idx, count: back.count, data: back.chunk };
            if let AssemblyEvent::Complete(b) = asm.push(0, back.kind, &seg).unwrap() {
                done = Some(b);
            }
        }
        prop_assert_eq!(done, Some(body));
    }

    #[test]
    fn control_garbage_never_panics(b in prop::collection::vec(any::<u8>(), 0..3000)) {
        for t in [PacketType::Roster, PacketType::JoinReq, PacketType::Rekey, PacketType::Voice] {
            let _ = ControlBody::decode(t, &b);
        }
    }

    #[test]
    fn roster_round_trip(entries in prop::collection::vec((any::<u16>(), any::<[u8; 16]>(), 0u8..3, "\\PC{0,8}"), 0..32)) {
        let entries: Vec<RosterEntry> = entries
            .into_iter()
            .map(|(node_id, id_hash, role, name)| RosterEntry {
                node_id,
                id_hash,
                ed_pk: [role; 32],
                role: [RosterRole::Member, RosterRole::Leader, RosterRole::Denied][usize::from(role)],
                name: mesh_wire::name::truncate_graphemes(&name, 24).to_owned(),
            })
            .collect();
        let r = Roster { group_id: [1; 16], epoch: 2, roster_version: 3, invite_sig: [4; 64], entries, sig: [5; 64] };
        let b = r.encode().unwrap();
        prop_assert_eq!(Roster::decode(&b).unwrap(), r.clone());
        prop_assert_eq!(&r.signed_message().unwrap()[14..], &b[..b.len() - 64]);
    }
}

#[test]
fn assembler_bounds_and_timeout() {
    let body = vec![7u8; 2000];
    let segs = segment_body(1, &body).unwrap();
    assert_eq!(segs.len(), 3);
    let mut asm = SegmentAssembler::for_mesh();
    assert_eq!(
        asm.push(0, (1, 1), &segs[0]).unwrap(),
        AssemblyEvent::Pending
    );
    assert_eq!(
        asm.push(1, (1, 1), &segs[0]).unwrap(),
        AssemblyEvent::Duplicate
    );
    // After 5 s the partial assembly is gone; the late segment starts a new one.
    assert_eq!(
        asm.push(REASSEMBLY_TIMEOUT_US, (1, 1), &segs[1]).unwrap(),
        AssemblyEvent::Pending
    );
    assert_eq!(asm.in_progress(), 1);
    assert_eq!(
        asm.push(REASSEMBLY_TIMEOUT_US, (1, 1), &segs[2]).unwrap(),
        AssemblyEvent::Pending
    );
    // A different count for the same msg_id discards the assembly.
    let other = Segment {
        msg_id: 1,
        idx: 0,
        count: 2,
        data: &[1],
    };
    assert_eq!(
        asm.push(REASSEMBLY_TIMEOUT_US, (1, 1), &other).unwrap(),
        AssemblyEvent::Inconsistent
    );
    assert_eq!(asm.in_progress(), 0);
    // At most 4 concurrent assemblies per origin: the oldest is evicted.
    for (t, msg) in (10u16..15).enumerate() {
        let s = Segment {
            msg_id: msg,
            idx: 0,
            count: 2,
            data: &[0],
        };
        asm.push(t as u64, (1, 1), &s).unwrap();
    }
    assert_eq!(asm.in_progress(), 4);
    let late = Segment {
        msg_id: 10,
        idx: 1,
        count: 2,
        data: &[0],
    };
    assert_eq!(asm.push(10, (1, 1), &late).unwrap(), AssemblyEvent::Pending);
    // Other origins are not affected by one origin's limit.
    asm.push(10, (2, 9), &late).unwrap();
    assert_eq!(asm.in_progress(), 5);
    asm.expire(10 + REASSEMBLY_TIMEOUT_US);
    assert_eq!(asm.in_progress(), 0);
    // SYNC: at most 2 per link.
    let mut sync: SegmentAssembler<u8> = SegmentAssembler::for_sync();
    for msg in 0..3u16 {
        sync.push(
            0,
            1,
            &Segment {
                msg_id: msg,
                idx: 0,
                count: 2,
                data: &[0],
            },
        )
        .unwrap();
    }
    assert_eq!(sync.in_progress(), 2);
}

#[test]
fn segmentation_limits() {
    assert!(segment_body(0, &[]).is_err());
    assert!(segment_body(0, &vec![0; MAX_LOGICAL_BODY + 1]).is_err());
    assert_eq!(segment_body(0, &vec![0; 900]).unwrap().len(), 1);
    assert_eq!(segment_body(0, &vec![0; 901]).unwrap().len(), 2);
    assert!(Segment::decode(&[0, 0, 0x10]).is_err(), "idx 1 of count 1");
    assert!(Segment::decode(&[0, 0]).is_err());
    let mut big = vec![0, 0, 0x01];
    big.extend(vec![0; 901]);
    assert!(Segment::decode(&big).is_err());
    let h = MeshHeader::new(PacketType::Voice, 1, 1, 1);
    assert!(segment_packets(&h, 0, 0, &[1]).is_err());
}

#[test]
fn control_bodies() {
    let entry = |node_id, role, name: &str| RosterEntry {
        node_id,
        id_hash: [node_id as u8; 16],
        ed_pk: [2; 32],
        role,
        name: name.to_owned(),
    };
    let r = Roster {
        group_id: [9; 16],
        epoch: 1,
        roster_version: 2,
        invite_sig: [3; 64],
        entries: vec![
            entry(10, RosterRole::Leader, "Ana"),
            entry(11, RosterRole::Denied, "X"),
            entry(12, RosterRole::Member, "Bo"),
        ],
        sig: [4; 64],
    };
    assert_eq!(r.index_of(10), Some(0));
    assert_eq!(r.index_of(12), Some(1));
    assert_eq!(r.index_of(11), None);
    assert!(r.is_denied(&[11; 16]));
    assert!(
        r.signed_message()
            .unwrap()
            .starts_with(b"cvic/v1/roster\x01")
    );
    let full = Roster {
        entries: (0..33).map(|i| entry(i, RosterRole::Member, "")).collect(),
        ..r.clone()
    };
    assert!(full.encode().is_err(), "33 members");
    // A long name is truncated at a grapheme boundary when encoded.
    let long = Roster {
        entries: vec![entry(
            1,
            RosterRole::Member,
            "Juan Carlos de la Fuente Ruiz",
        )],
        ..r.clone()
    };
    assert_eq!(
        Roster::decode(&long.encode().unwrap()).unwrap().entries[0].name,
        "Juan Carlos de la Fuente"
    );

    let j = JoinReq {
        ed_pk: [1; 32],
        x_pk: [2; 32],
        platform: 0,
        name: "Zoë".into(),
        sig: [3; 64],
    };
    let jb = j.encode();
    assert_eq!(jb.len(), 1 + 32 + 32 + 1 + 1 + 4 + 64);
    assert_eq!(JoinReq::decode(&jb).unwrap(), j);
    assert!(j.signed_message(&[9; 16]).starts_with(b"cvic/v1/join\x09"));
    let mut bad = jb.clone();
    bad[0] = 2;
    assert!(JoinReq::decode(&bad).is_err());

    let k = Rekey {
        new_epoch: 5,
        roster_version: 6,
        leader_eph_pk: [7; 32],
        entries: vec![RekeyEntry {
            node_id: 12,
            sealed: [8; 48],
        }],
        sig: [9; 64],
    };
    let kb = k.encode().unwrap();
    assert_eq!(kb.len(), 4 + 4 + 32 + 1 + 50 + 64);
    assert_eq!(Rekey::decode(&kb).unwrap(), k);
    assert_eq!(k.entry_for(12), Some(&[8; 48]));
    assert_eq!(k.entry_for(13), None);
    assert!(matches!(
        ControlBody::decode(PacketType::Rekey, &kb).unwrap(),
        ControlBody::Rekey(_)
    ));
    assert!(ControlBody::decode(PacketType::Talk, &kb).is_err());
}
