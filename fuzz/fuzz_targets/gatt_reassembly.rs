//! GATT reassembly (DESIGN §4.2): the input is split into ATT values by a length byte
//! before each value. Completed frames are bounded and re-chunk/reassemble identically.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::link::{GattChunker, GattEvent, GattReassembler, MAX_STREAM_FRAME};

fuzz_target!(|data: &[u8]| {
    let mut rx = GattReassembler::new();
    let mut rest = data;
    while let Some((&len, tail)) = rest.split_first() {
        let n = usize::from(len).min(tail.len());
        let (value, next) = tail.split_at(n);
        rest = next;
        if let GattEvent::Frame(f) = rx.push(value) {
            assert!(!f.is_empty() && f.len() <= MAX_STREAM_FRAME);
            let mtu = 23 + u16::from(len) * 2;
            let mut tx = GattChunker::new(mtu);
            let mut check = GattReassembler::new();
            let mut out = None;
            for v in tx.chunk(&f).expect("bounded frame chunks") {
                assert!(v.len() <= usize::from(mtu.min(517)) - 3);
                if let GattEvent::Frame(g) = check.push(&v) {
                    out = Some(g);
                }
            }
            assert_eq!(out.as_deref(), Some(f.as_slice()));
        }
        assert!(rx.partial_len() <= MAX_STREAM_FRAME);
    }
});
