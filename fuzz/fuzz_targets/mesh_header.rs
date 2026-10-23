//! MeshPacket header (DESIGN §4.5): accepted headers re-encode exactly and the relay
//! rewrite keeps every field in range.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::mesh::{MeshHeader, MeshPacket};

fuzz_target!(|data: &[u8]| {
    let Ok(pkt) = MeshPacket::decode(data) else {
        return;
    };
    assert_eq!(pkt.encode().expect("decoded packet encodes"), data);
    let h = pkt.header;
    let added = u64::from(data.last().copied().unwrap_or(0)) * 997;
    if let Some(r) = h.relayed(added) {
        assert_eq!(r.ttl + 1, h.ttl);
        assert!(r.hops <= MeshHeader::MAX_HOPS && r.age >= h.age);
        assert_eq!(
            MeshHeader::decode(&r.encode().expect("relayed header encodes")),
            Ok(r)
        );
    } else {
        assert_eq!(h.ttl, 0);
    }
});
