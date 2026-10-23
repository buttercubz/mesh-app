//! `link.json`: L2CAP stream framing, GATT chunking at ATT MTU 23/185/517, datagrams
//! (DESIGN §4.2).

use serde_json::json;

use super::{VectorFile, hx, pattern};
use crate::error::Result;
use crate::link::{GattChunker, encode_stream};

pub(super) fn file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "link.json",
        "Link framing (DESIGN §4.2): L2CAP/tunnel `len u16 LE` + frame (len 1..=1024); GATT \
         values `hdr u8` (b7 START, b6 END, b5-0 frame counter mod 64) + chunk of at most \
         ATT_MTU-4 bytes; Wi-Fi Aware datagram = one frame (<= 1200 B).",
    );

    for len in [1usize, 118, 1024] {
        let frame = pattern(0x40, len);
        f.push(
            &format!("l2cap/frame{len}"),
            json!({ "frame": hx(&frame) }),
            &encode_stream(&frame)?,
        );
    }
    let a = pattern(0x01, 3);
    let b = pattern(0x02, 5);
    let mut stream = encode_stream(&a)?;
    stream.extend_from_slice(&encode_stream(&b)?);
    f.push(
        "l2cap/two-frames-back-to-back",
        json!({ "frames": [hx(&a), hx(&b)] }),
        &stream,
    );

    for mtu in [23u16, 185, 517] {
        for len in [1usize, 19, 20, 100, 1024] {
            let frame = pattern(0x80, len);
            let mut ch = GattChunker::new(mtu);
            let parts = ch.chunk(&frame)?;
            f.push_parts(
                &format!("gatt/mtu{mtu}/frame{len}"),
                json!({ "att_mtu": mtu, "counter": 0, "frame": hx(&frame) }),
                &parts,
            );
        }
    }
    // The counter advances per frame and wraps mod 64.
    let mut ch = GattChunker::new(23);
    for _ in 0..63 {
        ch.chunk(&[0])?;
    }
    let frame = pattern(0x99, 25);
    let mut parts = ch.chunk(&frame)?;
    parts.extend(ch.chunk(&frame)?);
    f.push_parts(
        "gatt/mtu23/counter63-then-wrap",
        json!({ "att_mtu": 23, "counter": 63, "frames": [hx(&frame), hx(&frame)] }),
        &parts,
    );

    let dg = pattern(0x10, 86);
    f.push("datagram/aware-86", json!({ "frame": hx(&dg) }), &dg);
    Ok(f)
}
