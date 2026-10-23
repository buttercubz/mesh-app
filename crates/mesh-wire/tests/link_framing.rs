//! Link framing (DESIGN §4.2): stream deframer at any chunk boundary, GATT chunking at
//! ATT MTU 23/185/517, reassembly rules, datagram limits.

use mesh_wire::WireError;
use mesh_wire::link::{
    GATT_END, GATT_START, GattChunker, GattDrop, GattEvent, GattReassembler, MAX_STREAM_FRAME,
    StreamDeframer, check_datagram, encode_stream,
};
use proptest::prelude::*;

fn frames_strategy() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(
        prop::collection::vec(any::<u8>(), 1..=MAX_STREAM_FRAME),
        1..6,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Frames survive any split of the byte stream into chunks.
    #[test]
    fn deframer_any_chunk_boundary(frames in frames_strategy(), cuts in prop::collection::vec(1usize..300, 0..40)) {
        let mut stream = Vec::new();
        for f in &frames {
            stream.extend_from_slice(&encode_stream(f).unwrap());
        }
        let mut d = StreamDeframer::new();
        let mut out = Vec::new();
        let mut pos = 0;
        for c in cuts {
            let end = (pos + c).min(stream.len());
            out.extend(d.feed(&stream[pos..end]).unwrap());
            pos = end;
        }
        out.extend(d.feed(&stream[pos..]).unwrap());
        prop_assert_eq!(out, frames);
        prop_assert_eq!(d.buffered(), 0);
    }

    /// Arbitrary bytes never panic; once poisoned the deframer stays poisoned.
    #[test]
    fn deframer_garbage(chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..200), 0..20)) {
        let mut d = StreamDeframer::new();
        let mut poisoned = false;
        for c in chunks {
            match d.feed(&c) {
                Ok(frames) => {
                    prop_assert!(!poisoned);
                    for f in frames {
                        prop_assert!(!f.is_empty() && f.len() <= MAX_STREAM_FRAME);
                    }
                }
                Err(_) => poisoned = true,
            }
            prop_assert_eq!(d.is_poisoned(), poisoned);
        }
    }

    /// Chunk → reassemble is the identity at every MTU, and every value fits the MTU.
    #[test]
    fn gatt_round_trip(frames in frames_strategy(), mtu in 23u16..=517) {
        let mut tx = GattChunker::new(mtu);
        let mut rx = GattReassembler::new();
        for f in &frames {
            let values = tx.chunk(f).unwrap();
            let last = values.len() - 1;
            for (i, v) in values.iter().enumerate() {
                prop_assert!(v.len() <= usize::from(mtu) - 3);
                let ev = rx.push(v);
                if i == last {
                    prop_assert_eq!(ev, GattEvent::Frame(f.clone()));
                } else {
                    prop_assert_eq!(ev, GattEvent::Pending);
                }
            }
        }
    }

    #[test]
    fn gatt_garbage_never_panics(values in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..520), 0..40)) {
        let mut rx = GattReassembler::new();
        for v in values {
            if let GattEvent::Frame(f) = rx.push(&v) {
                prop_assert!(!f.is_empty() && f.len() <= MAX_STREAM_FRAME);
            }
            prop_assert!(rx.partial_len() <= MAX_STREAM_FRAME);
        }
    }
}

#[test]
fn stream_lengths_are_bounded() {
    assert!(encode_stream(&[]).is_err());
    assert!(encode_stream(&[0; 1025]).is_err());
    assert_eq!(
        encode_stream(&[0xAA; 3]).unwrap(),
        vec![3, 0, 0xAA, 0xAA, 0xAA]
    );
    for bad in [[0u8, 0u8], [0x01, 0x04]] {
        let mut d = StreamDeframer::new();
        assert!(matches!(
            d.feed(&bad),
            Err(WireError::TooShort { .. } | WireError::TooLong { .. })
        ));
        // No resync: even a valid frame afterwards is refused.
        assert!(d.feed(&encode_stream(&[1]).unwrap()).is_err());
        assert!(d.is_poisoned());
    }
    let mut d = StreamDeframer::new();
    assert_eq!(d.feed(&[0x00, 0x04]).unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(d.buffered(), 2);
}

#[test]
fn gatt_chunk_sizes_at_reference_mtus() {
    for (mtu, per) in [(23u16, 19usize), (185, 181), (517, 513)] {
        let mut c = GattChunker::new(mtu);
        assert_eq!(c.chunk_payload(), per);
        let values = c.chunk(&vec![7u8; 1024]).unwrap();
        assert_eq!(values.len(), 1024usize.div_ceil(per));
        assert_eq!(values[0][0], GATT_START);
        assert_eq!(values.last().unwrap()[0], GATT_END);
        assert!(values.iter().all(|v| v.len() <= usize::from(mtu) - 3));
        assert_eq!(values.iter().map(|v| v.len() - 1).sum::<usize>(), 1024);
        // A frame that fits in one value carries START and END together.
        let one = c.chunk(&vec![1u8; per]).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0][0], GATT_START | GATT_END | 1);
    }
    // MTU below 23 or above 517 is clamped.
    assert_eq!(GattChunker::new(5).chunk_payload(), 19);
    assert_eq!(GattChunker::new(2000).chunk_payload(), 513);
}

#[test]
fn gatt_reassembly_rules() {
    let mut rx = GattReassembler::new();
    // Continuation without START.
    assert_eq!(
        rx.push(&[0x05, 1, 2]),
        GattEvent::Dropped(GattDrop::NoStart)
    );
    assert_eq!(rx.push(&[]), GattEvent::Dropped(GattDrop::Empty));
    // START of counter 1, then a chunk of counter 2: discarded.
    assert_eq!(rx.push(&[GATT_START | 1, 1, 2]), GattEvent::Pending);
    assert_eq!(
        rx.push(&[2, 3]),
        GattEvent::Dropped(GattDrop::CounterMismatch)
    );
    assert_eq!(rx.partial_len(), 0);
    // A START discards a partial frame.
    assert_eq!(rx.push(&[GATT_START | 3, 9, 9]), GattEvent::Pending);
    assert_eq!(
        rx.push(&[GATT_START | GATT_END | 4, 5]),
        GattEvent::Frame(vec![5])
    );
    // Empty completed frame.
    assert_eq!(
        rx.push(&[GATT_START | GATT_END]),
        GattEvent::Dropped(GattDrop::EmptyFrame)
    );
    // Overflow past 1024 B.
    assert_eq!(rx.push(&[GATT_START | 6]), GattEvent::Pending);
    let big = [6u8; 600];
    let mut v = vec![6u8];
    v.extend_from_slice(&big);
    assert_eq!(rx.push(&v), GattEvent::Pending);
    assert_eq!(rx.push(&v), GattEvent::Dropped(GattDrop::Overflow));
    assert_eq!(rx.partial_len(), 0);
}

#[test]
fn gatt_counter_wraps_mod_64() {
    let mut c = GattChunker::new(23);
    for i in 0..130u32 {
        assert_eq!(u32::from(c.next_counter()), i % 64);
        c.chunk(&[0]).unwrap();
    }
}

#[test]
fn datagram_limits() {
    assert!(check_datagram(&[0; 1200]).is_ok());
    assert!(check_datagram(&[0; 1201]).is_err());
    assert!(check_datagram(&[]).is_err());
}
