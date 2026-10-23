//! `records.json`: every record type 0x01-0x0F (DESIGN §4.4), the LEB128 length forms
//! and a coalesced plaintext.

use serde_json::{Value, json};

use super::{VectorFile, hx, pattern, pattern_array};
use crate::error::Result;
use crate::frame::RejectReason;
use crate::mesh::{MeshHeader, MeshPacket, PacketType, Presence};
use crate::record::{
    Caps, Close, IdProof, LabCtl, LabCtlOp, LabGps, Lecho, Lprobe, Lstat, Nbr, NbrEntry, NbrFlags,
    Ping, Pong, Record, Sync, SyncKind, Upgrade, UpgradeOp, UpgradeTransport, encode_records,
};

fn push(f: &mut VectorFile, name: &str, inputs: Value, rec: &Record<'_>) -> Result<()> {
    f.push(name, inputs, &rec.encode()?);
    Ok(())
}

pub(super) fn file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "records.json",
        "Records (DESIGN §4.4): rtype u8 || len LEB128 (1-2 B, <= 16383, shortest form) || body; \
         <= 32 records per plaintext; unknown types are skipped. expected = the whole record.",
    );

    let mut h = MeshHeader::new(PacketType::Presence, 0x0102, 0x5A, 0x1234);
    h.age = 3;
    let presence = Presence::default().encode();
    let pkt = MeshPacket {
        header: h,
        payload: &presence,
    }
    .encode()?;
    push(
        &mut f,
        "0x01-mesh",
        json!({ "mesh_packet": hx(&pkt) }),
        &Record::Mesh(&pkt),
    )?;

    push(
        &mut f,
        "0x02-ping",
        json!({ "id": 7, "t_tx": 0x0102_0304u32 }),
        &Record::Ping(Ping {
            id: 7,
            t_tx: 0x0102_0304,
        }),
    )?;
    push(
        &mut f,
        "0x03-pong",
        json!({ "id": 7, "t_tx": 0x0102_0304u32, "hold_us": 850 }),
        &Record::Pong(Pong {
            id: 7,
            t_tx: 0x0102_0304,
            hold_us: 850,
        }),
    )?;

    let entries = [
        NbrEntry {
            node_id: 0x0A0B,
            q: 80,
            flags: NbrFlags::new(true, false, NbrFlags::KIND_BLE),
            vcap: 2,
        },
        NbrEntry {
            node_id: 0x0C0D,
            q: 40,
            flags: NbrFlags::new(false, true, NbrFlags::KIND_AWARE),
            vcap: 4,
        },
    ];
    let nbr = Nbr::new(9, &entries)?;
    push(
        &mut f,
        "0x04-nbr-v1-2-entries",
        json!({ "ver": 1, "nseq": 9, "entries": [
            { "node_id": 0x0A0B, "q": 80, "good": true, "congested": false, "kind": 0, "vcap": 2 },
            { "node_id": 0x0C0D, "q": 40, "good": false, "congested": true, "kind": 1, "vcap": 4 } ] }),
        &Record::Nbr(nbr),
    )?;
    push(
        &mut f,
        "0x04-nbr-v1-empty",
        json!({ "ver": 1, "nseq": 0, "entries": [] }),
        &Record::Nbr(Nbr::new(0, &[])?),
    )?;

    let lstat = Lstat {
        rx_frames: 250,
        rx_bytes: 29_500,
        gaps: 1,
        pace_kbps: 60,
        queue_ms: 12,
    };
    push(
        &mut f,
        "0x05-lstat",
        json!({ "rx_frames": 250, "rx_bytes": 29_500, "gaps": 1, "pace_kbps": 60, "queue_ms": 12 }),
        &Record::Lstat(lstat),
    )?;

    let caps = Caps {
        proto: 1,
        platform: Caps::PLATFORM_IOS,
        app_ver: 0x0102,
        codec: Caps::CODEC_OPUS_161 | Caps::CODEC_DRED,
        transports: Caps::T_L2CAP | Caps::T_GATT | Caps::T_AWARE_IOS,
        max_links: 3,
        role: 1,
        epoch: 0x0001_0002,
        roster_version: 17,
        ctl_version: 2,
    };
    push(
        &mut f,
        "0x06-caps",
        json!({ "proto": 1, "platform": 0, "app_ver": 0x0102, "codec": 3, "transports": 7,
                "max_links": 3, "role": 1, "epoch": 0x0001_0002u32, "roster_version": 17, "ctl_version": 2 }),
        &Record::Caps(caps),
    )?;
    let lab_caps = Caps {
        role: 0,
        epoch: 0,
        roster_version: 0,
        ctl_version: 0,
        ..caps
    };
    push(
        &mut f,
        "0x06-caps-lab-link-zero-summary",
        json!({ "summary": "zeros on Lab links" }),
        &Record::Caps(lab_caps),
    )?;

    let lp = Lprobe {
        probe_id: 0xDEAD_BEEF,
        t_tx: 123_456_789,
        flags: 1,
        pad_len: 107,
    };
    push(
        &mut f,
        "0x07-lprobe-pad107",
        json!({ "probe_id": 0xDEAD_BEEFu32, "t_tx": 123_456_789u64, "flags": 1, "pad_len": 107 }),
        &Record::Lprobe(lp),
    )?;
    let le = Lecho {
        probe_id: 0xDEAD_BEEF,
        t_tx: 123_456_789,
        hold_us: 640,
        rx_count: 1000,
    };
    push(
        &mut f,
        "0x08-lecho",
        json!({ "probe_id": 0xDEAD_BEEFu32, "t_tx": 123_456_789u64, "hold_us": 640, "rx_count": 1000 }),
        &Record::Lecho(le),
    )?;

    let params = [0x11u8, 0x22, 0x33];
    let up = Upgrade {
        op: UpgradeOp::Offer,
        transport: UpgradeTransport::AwareIos,
        nonce: 0x0A0B_0C0D,
        params: &params,
    };
    push(
        &mut f,
        "0x09-upgrade-offer",
        json!({ "op": 1, "transport": 1, "nonce": 0x0A0B_0C0Du32, "params": hx(&params) }),
        &Record::Upgrade(up),
    )?;

    for (r, name) in [(5u8, "member-removed"), (9, "redundant"), (10, "leaving")] {
        let reason = RejectReason::from_u8(r, true)?;
        push(
            &mut f,
            &format!("0x0a-close-{name}"),
            json!({ "reason": r }),
            &Record::Close(Close { reason }),
        )?;
    }

    let ann = LabCtl::announce_params(0x4321, 0x0102_0304_0506_0708);
    let ctl = LabCtl {
        op: LabCtlOp::Announce,
        run_id: 77,
        bench: 0,
        params: &ann,
    };
    push(
        &mut f,
        "0x0b-lab_ctl-announce",
        json!({ "op": 5, "run_id": 77, "bench": 0, "lab_node_id": 0x4321, "nonce": 0x0102_0304_0506_0708u64 }),
        &Record::LabCtl(ctl),
    )?;
    let start = LabCtl {
        op: LabCtlOp::Start,
        run_id: 77,
        bench: 3,
        params: &[],
    };
    push(
        &mut f,
        "0x0b-lab_ctl-start",
        json!({ "op": 1, "run_id": 77, "bench": 3, "params": "" }),
        &Record::LabCtl(start),
    )?;

    let idp = IdProof {
        ed_pk: pattern_array(0x21),
        x_pk: pattern_array(0x42),
        sig: pattern_array(0x63),
    };
    push(
        &mut f,
        "0x0c-idproof",
        json!({ "ed_pk": hx(&idp.ed_pk), "x_pk": hx(&idp.x_pk), "sig": hx(&idp.sig),
                "signed_context": "\"cvic/v1/idproof\" || role u8 || handshake_hash[32]" }),
        &Record::IdProof(idp),
    )?;
    let hh: [u8; 32] = pattern_array(0x05);
    f.push(
        "0x0c-idproof/signed-message-responder",
        json!({ "role": 1, "handshake_hash": hx(&hh) }),
        &IdProof::signed_message(1, &hh),
    );

    let gps = LabGps {
        t_ms: 5_000,
        lat_e7: 194_326_077,
        lon_e7: -991_332_080,
        speed_cms: 2_778,
        acc_dm: 35,
        heading_cdeg: 27_000,
    };
    push(
        &mut f,
        "0x0d-lab_gps",
        json!({ "t_ms": 5000, "lat_e7": 194_326_077, "lon_e7": -991_332_080, "speed_cms": 2778,
                "acc_dm": 35, "heading_cdeg": 27000 }),
        &Record::LabGps(gps),
    )?;

    push(
        &mut f,
        "0x0e-pad-16",
        json!({ "len": 16 }),
        &Record::Pad(16),
    )?;
    push(
        &mut f,
        "0x0e-pad-200-two-byte-leb128",
        json!({ "len": 200 }),
        &Record::Pad(200),
    )?;

    let chunk = pattern(0x77, 40);
    let sync = Sync {
        kind: SyncKind::Roster,
        msg_id: 0x0102,
        idx: 1,
        count: 3,
        chunk: &chunk,
    };
    push(
        &mut f,
        "0x0f-sync-roster-seg1of3",
        json!({ "kind": 1, "msg_id": 0x0102, "idx": 1, "count": 3, "chunk": hx(&chunk) }),
        &Record::Sync(sync),
    )?;

    let unknown_body = [1u8, 2, 3];
    push(
        &mut f,
        "0x42-unknown-skipped",
        json!({ "rtype": 0x42, "body": hx(&unknown_body) }),
        &Record::Unknown {
            rtype: 0x42,
            body: &unknown_body,
        },
    )?;

    let plaintext = encode_records(&[
        Record::Mesh(&pkt),
        Record::Ping(Ping { id: 1, t_tx: 2 }),
        Record::Pad(3),
    ])?;
    f.push(
        "coalesced/mesh-ping-pad",
        json!({ "records": ["0x01-mesh", "ping id 1 t_tx 2", "pad 3"] }),
        &plaintext,
    );
    Ok(f)
}
