//! Every mesh packet type (DESIGN §4.6): the typed payload decoder chosen by the header,
//! plus each ptype decoder run directly on the input. Accepted payloads round-trip.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::mesh::{
    Ext, FloorReject, GroupCtl, Leave, MeshPacket, MeshPayload, MeshProbe, PathHint, Position,
    Presence, RideCtl, Segment, Talk, Voice, encode_packet,
};

fuzz_target!(|data: &[u8]| {
    if let Ok(pkt) = MeshPacket::decode(data)
        && let Ok(p) = MeshPayload::decode(&pkt)
    {
        assert_eq!(
            encode_packet(&pkt.header, &p).expect("typed payload encodes"),
            data
        );
    }
    let Some((&sel, p)) = data.split_first() else {
        return;
    };
    let reencoded: Option<Vec<u8>> = match sel % 12 {
        0 => Voice::decode(p).ok().map(|v| v.encode().unwrap()),
        1 => Talk::decode(p).ok().map(|v| v.encode()),
        2 => FloorReject::decode(p).ok().map(|v| v.encode()),
        3 => Presence::decode(p).ok().map(|v| v.encode()),
        4 => Leave::decode(p).ok().map(|v| v.encode()),
        5 => RideCtl::decode(p).ok().map(|v| v.encode()),
        6 => MeshProbe::decode(p).ok().map(|v| v.encode().unwrap()),
        7 => PathHint::decode(p).ok().map(|v| v.encode().unwrap()),
        8 => GroupCtl::decode(p).ok().map(|v| v.encode().unwrap()),
        9 => Position::decode(p).ok().map(|v| v.encode()),
        10 => Segment::decode(p).ok().map(|v| v.encode().unwrap()),
        _ => Ext::decode(p).ok().map(|v| v.encode()),
    };
    if let Some(b) = reencoded {
        assert_eq!(b, p);
    }
});
