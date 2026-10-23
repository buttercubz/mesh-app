//! Segment reassembly (DESIGN §4.6.1, §4.4 SYNC): the input is a sequence of
//! `time_step u8 · group u8 · seg_len u8 · segment payload` operations against one
//! bounded assembler. Assemblies stay bounded and completed bodies fit 16 × 900 B.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::mesh::segment::MAX_LOGICAL_BODY;
use mesh_wire::mesh::{AssemblyEvent, Segment, SegmentAssembler};

fuzz_target!(|data: &[u8]| {
    let mut asm: SegmentAssembler<u8> = SegmentAssembler::for_mesh();
    let mut now = 0u64;
    let mut rest = data;
    while rest.len() >= 3 {
        let (op, tail) = rest.split_at(3);
        let n = usize::from(op[2]).min(tail.len());
        let (seg_bytes, next) = tail.split_at(n);
        rest = next;
        now += u64::from(op[0]) * 50_000;
        let group = op[1] % 4;
        if let Ok(seg) = Segment::decode(seg_bytes)
            && let Ok(AssemblyEvent::Complete(body)) = asm.push(now, group, &seg)
        {
            assert!(body.len() <= MAX_LOGICAL_BODY);
        }
        assert!(asm.in_progress() <= 16);
    }
});
