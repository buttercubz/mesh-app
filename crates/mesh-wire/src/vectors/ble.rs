//! `hello.json`, `adv.json`, `names.json` and `sizes.json` (DESIGN §9.3, §9.4, §4.6.1,
//! §4.8). Swift and Kotlin tests replay `hello.json` and `adv.json`: these are the only
//! bytes the native layers build themselves.

use serde_json::json;

use super::{VectorFile, hx};
use crate::adv::{
    AdvFlags, ServiceData, android_adv_data, ios_local_name, ios_local_name_ad, token_prefix14,
};
use crate::budget::{
    C_CTRL_BPS, Framing, POSITION_RECORD, PRESENCE_RECORD, coalesced_bps, cost_bps, r_stream_bps,
    voice_sizes,
};
use crate::consts::{HELLO_CHR, MESH_SVC, RX_CHR, TX_CHR};
use crate::error::Result;
use crate::frame::lab_tag;
use crate::hello::{Hello, HelloFlags};
use crate::mesh::EncoderProfile;
use crate::name::{DISPLAY_NAME_MAX, GROUP_NAME_MAX, truncate_graphemes};

pub(super) fn hello_file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "hello.json",
        "HELLO characteristic value (DESIGN §9.3), 16 B: magic 0xc1 || proto_min || proto_max || \
         flags (b0 android, b1 lab, b2 accepting, b3 aware, b4 leader) || psm u16 LE (0 = GATT \
         only) || group_tag[4] (Lab: lab_tag u16 LE || 0000) || node_token[4] || epoch_lo u16 LE \
         (epoch mod 2^16).",
    );
    let cases = [
        (
            "group/ios-accepting-l2cap",
            HelloFlags::new(false, false, true, false, false),
            0x0081u16,
            0x0000_0007u32,
        ),
        (
            "group/android-leader-aware-gatt-only",
            HelloFlags::new(true, false, true, true, true),
            0,
            0x0001_0002,
        ),
        (
            "group/not-accepting-epoch-wraps",
            HelloFlags::new(false, false, false, false, false),
            0x00C1,
            0x0002_FFFF,
        ),
    ];
    for (name, flags, psm, epoch) in cases {
        let gt = [0xA1, 0xB2, 0xC3, 0xD4];
        let nt = [0x1F, 0x2E, 0x3D, 0x4C];
        let h = Hello::group(flags, psm, gt, nt, epoch);
        f.push(
            name,
            json!({ "proto_min": 1, "proto_max": 1, "flags": flags.0, "psm": psm, "group_tag": hx(&gt),
                    "node_token": hx(&nt), "epoch": epoch, "epoch_lo": h.epoch_lo }),
            &h.encode()?,
        );
    }
    let tag = lab_tag(1234);
    let h = Hello::lab(
        HelloFlags::new(false, false, true, false, false),
        0x0081,
        tag,
        [9, 8, 7, 6],
    );
    f.push(
        "lab/code-1234",
        json!({ "lab_code": 1234, "lab_tag": tag, "flags": h.flags.0, "psm": 0x0081, "node_token": "09080706",
                "epoch_lo": 0 }),
        &h.encode()?,
    );
    Ok(f)
}

pub(super) fn adv_file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "adv.json",
        "Advertising payloads (DESIGN §9.4) and GATT UUIDs (§9.3). AD structures are len || \
         type || data; 128-bit UUIDs inside AD are little-endian. iPhone local name: 6 RFC 4648 \
         base32 chars (A-Z2-7, MSB first) of group_tag[0] || group_tag[1] || top 14 bits of \
         node_token (30 bits).",
    );
    for (name, u) in [
        ("uuid/mesh_svc", MESH_SVC),
        ("uuid/hello", HELLO_CHR),
        ("uuid/rx", RX_CHR),
        ("uuid/tx", TX_CHR),
    ] {
        f.push(
            name,
            json!({ "uuid": u.to_canonical(), "byte_order": "canonical (big-endian)" }),
            &u.bytes(),
        );
    }
    f.push(
        "uuid/mesh_svc-le",
        json!({ "uuid": MESH_SVC.to_canonical(), "byte_order": "AD/ATT little-endian" }),
        &MESH_SVC.le_bytes(),
    );

    f.push(
        "android/adv-data-21",
        json!({ "flags": 6, "service": MESH_SVC.to_canonical() }),
        &android_adv_data(),
    );
    for (name, android, lab) in [
        ("android/scan-response-group", true, false),
        ("android/scan-response-lab", true, true),
    ] {
        let sd = ServiceData {
            flags: AdvFlags::new(android, lab),
            group_tag: [0xA1, 0xB2, 0xC3, 0xD4],
            node_token: [0x1F, 0x2E, 0x3D, 0x4C],
        };
        f.push(
            name,
            json!({ "flags": sd.flags.0, "group_tag": "a1b2c3d4", "node_token": "1f2e3d4c",
                    "service_data": hx(&sd.encode()) }),
            &sd.scan_response(),
        );
    }
    for (gt, nt) in [
        ([0xA1u8, 0xB2, 0xC3, 0xD4], [0x1Fu8, 0x2E, 0x3D, 0x4C]),
        ([0, 0, 0, 0], [0, 3, 0, 0]),
        ([0xFF; 4], [0xFF; 4]),
    ] {
        let name = ios_local_name(&gt, &nt);
        f.push(
            &format!("ios/local-name-{name}"),
            json!({ "group_tag": hx(&gt), "node_token": hx(&nt), "token_prefix14": token_prefix14(&nt),
                    "local_name": name }),
            &ios_local_name_ad(&gt, &nt),
        );
    }
    Ok(f)
}

pub(super) fn names_file() -> VectorFile {
    let mut f = VectorFile::new(
        "names.json",
        "Names (DESIGN §4.6.1): UTF-8 bytes, truncated at the last extended grapheme-cluster \
         boundary that fits (display name <= 24 B, group name <= 32 B).",
    );
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let cases: [(&str, String, usize); 6] = [
        ("display/ascii-short", "Ana".to_owned(), DISPLAY_NAME_MAX),
        (
            "display/ascii-long",
            "Juan Carlos de la Fuente Ruiz".to_owned(),
            DISPLAY_NAME_MAX,
        ),
        (
            "display/combining-accents",
            "e\u{301}".repeat(9),
            DISPLAY_NAME_MAX,
        ),
        (
            "display/zwj-family-does-not-fit",
            format!("Jo{family}"),
            DISPLAY_NAME_MAX,
        ),
        (
            "display/spanish-accents",
            "Mónica Hernández Álvarez Peña".to_owned(),
            DISPLAY_NAME_MAX,
        ),
        (
            "group/flags",
            "\u{1F1F2}\u{1F1FD}".repeat(5),
            GROUP_NAME_MAX,
        ),
    ];
    for (name, input, max) in cases {
        f.push(
            name,
            json!({ "name": input, "name_utf8": hx(input.as_bytes()), "max_bytes": max }),
            truncate_graphemes(&input, max).as_bytes(),
        );
    }
    f
}

pub(super) fn sizes_file() -> VectorFile {
    let mut f = VectorFile::new(
        "sizes.json",
        "Sizes and cost model (DESIGN §4.8): byte chain MeshPacket -> record -> SEC frame -> \
         on-air unit, per-stream rate in bps, cost(k) = k * r_stream + 3000 bps. expected is the \
         u32 LE encoding of the number named by the vector.",
    );
    let rows = [
        ("ble16", EncoderProfile::Ble16, 80usize, Framing::Stream),
        ("ble12", EncoderProfile::Ble12, 60, Framing::Stream),
        ("ble-dred", EncoderProfile::BleDred, 100, Framing::Stream),
        ("wifi20", EncoderProfile::Wifi20, 50, Framing::Datagram),
    ];
    for (label, p, opus, framing) in rows {
        let s = voice_sizes(opus, framing);
        let inputs = json!({ "opus_bytes": opus, "mesh_packet": s.mesh_packet, "record": s.record,
                             "sec_frame": s.sec_frame, "on_air": s.on_air });
        f.push(
            &format!("{label}/on-air-bytes"),
            inputs.clone(),
            &(s.on_air as u32).to_le_bytes(),
        );
        f.push(
            &format!("{label}/r_stream-bps"),
            inputs,
            &r_stream_bps(p).to_le_bytes(),
        );
    }
    f.push(
        "ble16/cost-2-bps",
        json!({ "k": 2, "c_ctrl_bps": C_CTRL_BPS }),
        &cost_bps(2, EncoderProfile::Ble16).to_le_bytes(),
    );
    f.push(
        "ble16/coalesced-2-bps",
        json!({ "k": 2 }),
        &coalesced_bps(2, EncoderProfile::Ble16).to_le_bytes(),
    );
    f.push(
        "presence/record-bytes",
        json!({ "payload": 14 }),
        &(PRESENCE_RECORD as u32).to_le_bytes(),
    );
    f.push(
        "position/record-bytes",
        json!({ "payload": 14 }),
        &(POSITION_RECORD as u32).to_le_bytes(),
    );
    f
}
